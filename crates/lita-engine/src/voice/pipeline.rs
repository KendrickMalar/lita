//! The careful way to read a voice (voice quality requirement, 2026-09-23,
//! revised the same day after five blind comparisons):
//!
//! 0. measure   — Rust counts rhythm, punctuation and scripts (no model)
//! 1. extract   — the single-call extraction of 2026-09-22, unchanged: it is
//!                the contract generation sees, and in four blind rounds the
//!                author recognised the writer in its output every time
//! 2. evidence  — Rust finds verbatim quotes for the words, endings,
//!                pronoun and emoji the profile claims
//! 3. check     — one adversarial call over the material and the claims,
//!                hunting for counter-examples; its quotes are verified and
//!                feed confidence; a contradicted claim is low confidence
//!
//! A multi-pass synthesis (read each piece → synthesise → check) was built
//! first and lost 4/4 blind rounds to the single call while costing 4× the
//! time, so it was removed. Rigour lives in what is shown (evidence,
//! confidence, measurements), not in a different contract.

use super::measure::measure;
use super::{Backing, Confidence, Evidence, VoiceProfile};
use crate::{CodexCli, Effort, Request};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct Config {
    /// Pieces read one by one in stage 1; the rest still count in measurement.
    pub max_pieces: usize,
    pub concurrency: usize,
    /// Per piece: this many characters from the head …
    pub head_chars: usize,
    /// … and this many from the tail, with a marker between.
    pub tail_chars: usize,
    pub total_chars: usize,
    /// Past this, stage 3 is skipped; past `hard_limit`, fall back.
    pub soft_limit: Duration,
    pub hard_limit: Duration,
    pub self_check: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            max_pieces: 20,
            concurrency: 4,
            head_chars: 2_500,
            tail_chars: 1_500,
            total_chars: 80_000,
            soft_limit: Duration::from_secs(360),
            hard_limit: Duration::from_secs(480),
            self_check: true,
        }
    }
}

/// What happened, for the PoC report and the handoff.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Report {
    pub pieces_read: usize,
    pub pieces_total: usize,
    pub stage_secs: BTreeMap<String, u64>,
    pub total_secs: u64,
    pub quotes_offered: usize,
    pub quotes_verified: usize,
    pub confidence: BTreeMap<String, usize>,
    pub check_fit_pct: Option<usize>,
    pub redo: bool,
    pub fell_back: bool,
    pub dropped_words: Vec<String>,
    pub notes: Vec<String>,
    /// field → verdict from the last self-check.
    pub verdicts: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Measuring,
    /// Kept for API compatibility; the per-piece reading pass was removed.
    Reading { done: usize, total: usize },
    Synthesising,
    Checking,
    Done,
}

pub struct Outcome {
    pub profile: VoiceProfile,
    pub report: Report,
}

/// The contract fields the self-check judges.
pub const FIELDS: &[&str] = &[
    "first_person", "formality", "tone", "sentence_endings", "preferred_words", "avoided_words", "opens_with", "closes_with", "uses_emoji",
];

// ----- stage 3: self-check ---------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
struct CheckReport {
    items: Vec<CheckItem>,
}

#[derive(Debug, Clone, Deserialize)]
struct CheckItem {
    field: String,
    verdict: String,
    #[serde(default)]
    quote: String,
    #[serde(default)]
    piece: usize,
    #[serde(default)]
    note: String,
}

fn check_schema() -> Value {
    json!({
        "type": "object", "additionalProperties": false, "required": ["items"],
        "properties": { "items": { "type": "array", "description": "One entry per profile field listed in the prompt.",
            "items": { "type": "object", "additionalProperties": false, "required": ["field", "verdict", "quote", "piece", "note"], "properties": {
                "field": { "type": "string", "description": "The field name exactly as listed." },
                "verdict": { "type": "string", "enum": ["fits", "partial", "contradicts"], "description": "fits: no counter-example found; partial: the claim holds in some pieces but not others; contradicts: the material shows the opposite." },
                "quote": { "type": "string", "description": "A verbatim quote from the material that is the strongest counter-example (for partial/contradicts) or the strongest support (for fits)." },
                "piece": { "type": "integer", "description": "1-based piece number of the quote." },
                "note": { "type": "string", "description": "One sentence on why, or empty." }
            } } } }
    })
}

fn check_prompt(pieces: &[String], profile: &VoiceProfile) -> String {
    let corpus = pieces.iter().enumerate().map(|(i, p)| format!("<piece n=\"{}\">\n{}\n</piece>", i + 1, p)).collect::<Vec<_>>().join("\n\n");
    let claims = json!({
        "first_person": profile.first_person, "formality": profile.formality, "tone": profile.tone, "sentence_endings": profile.sentence_endings,
        "preferred_words": profile.preferred_words, "avoided_words": profile.avoided_words, "opens_with": profile.opens_with, "closes_with": profile.closes_with,
        "uses_emoji": profile.uses_emoji,
    });
    format!(
        "Below is a person's writing, and a set of claims about how they write. Your job is adversarial: for EACH claim, search the writing for a counter-example. \
         Only if you find none may you answer fits, and then quote the strongest support. Quotes must be copied verbatim.\n\n\
         <writing>\n{corpus}\n</writing>\n\n<claims>\n{}\n</claims>\n\nReturn one item per claim, field names exactly as in the claims.",
        serde_json::to_string_pretty(&claims).unwrap_or_default()
    )
}

// ----- helpers ---------------------------------------------------------------

fn normalize(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).collect()
}

/// Cuts a piece to head + tail with a marker, so closings survive.
pub fn trim_piece(text: &str, head: usize, tail: usize) -> String {
    let chars: Vec<char> = text.trim().chars().collect();
    if chars.len() <= head + tail {
        return chars.iter().collect();
    }
    let h: String = chars[..head].iter().collect();
    let t: String = chars[chars.len() - tail..].iter().collect();
    format!("{h}\n\n［…中略…］\n\n{t}")
}

struct Verifier {
    normalized: Vec<String>,
}

impl Verifier {
    fn new(pieces: &[String]) -> Self {
        Self { normalized: pieces.iter().map(|p| normalize(p)).collect() }
    }
    /// The piece index the quote occurs in (preferring the claimed one), or None.
    fn find(&self, quote: &str, claimed: Option<usize>) -> Option<usize> {
        let q = normalize(quote);
        if q.chars().count() < 4 { return None; }
        if let Some(c) = claimed { if self.normalized.get(c).is_some_and(|p| p.contains(&q)) { return Some(c); } }
        self.normalized.iter().position(|p| p.contains(&q))
    }
    fn occurrences(&self, needle: &str) -> usize {
        let n = normalize(needle);
        if n.is_empty() { return 0; }
        self.normalized.iter().map(|p| p.matches(&n).count()).sum()
    }
}

fn confidence_for(evidence: &[Evidence], verifier: &Verifier) -> Confidence {
    if evidence.is_empty() { return Confidence::Low; }
    let mut pieces: Vec<usize> = evidence.iter().map(|e| e.piece).collect();
    pieces.sort_unstable();
    pieces.dedup();
    let occurrences: usize = evidence.iter().map(|e| verifier.occurrences(&e.quote).max(1)).sum();
    if pieces.len() >= 2 && occurrences >= 3 { Confidence::High } else if evidence.len() >= 2 { Confidence::Medium } else { Confidence::Low }
}

// ----- the pipeline ----------------------------------------------------------

/// Sentences from the material that contain `needle`, as evidence, at most
/// `max`, preferring different pieces.
fn quotes_for(pieces: &[String], needle: &str, max: usize) -> Vec<Evidence> {
    let n = normalize(needle);
    if n.chars().count() < 2 { return vec![]; }
    let mut out: Vec<Evidence> = Vec::new();
    for (i, p) in pieces.iter().enumerate() {
        for sentence in super::measure::split_sentences(p) {
            if normalize(&sentence).contains(&n) && sentence.chars().count() <= 120 {
                out.push(Evidence { piece: i, quote: sentence.clone() });
                break; // one per piece first
            }
        }
        if out.len() >= max { break; }
    }
    if out.len() < max {
        for (i, p) in pieces.iter().enumerate() {
            for sentence in super::measure::split_sentences(p) {
                if out.len() >= max { break; }
                if normalize(&sentence).contains(&n) && sentence.chars().count() <= 120 && !out.iter().any(|e| e.quote == sentence) {
                    out.push(Evidence { piece: i, quote: sentence.clone() });
                }
            }
        }
    }
    out
}

pub fn extract(
    cli: &CodexCli,
    sources: &[&str],
    cfg: &Config,
    working_dir: PathBuf,
    cancel: Arc<AtomicBool>,
    mut on_stage: impl FnMut(Stage),
) -> Result<Outcome> {
    let started = Instant::now();
    let mut report = Report::default();
    let mut lap = Instant::now();

    // Stage 0: material and measurement.
    on_stage(Stage::Measuring);
    let mut pieces: Vec<String> = Vec::new();
    let mut used = 0usize;
    for s in sources {
        if s.trim().is_empty() { continue; }
        if used >= cfg.total_chars { break; }
        let p = trim_piece(s, cfg.head_chars, cfg.tail_chars);
        used += p.chars().count();
        pieces.push(p);
    }
    if pieces.is_empty() {
        anyhow::bail!("no writing to read");
    }
    let measured = measure(&pieces);
    report.pieces_total = pieces.len();
    report.pieces_read = pieces.len();
    report.stage_secs.insert("measure".into(), lap.elapsed().as_secs());
    lap = Instant::now();

    // Stage 1: the contract, extracted exactly as before.
    on_stage(Stage::Synthesising);
    let samples: Vec<&str> = pieces.iter().map(String::as_str).collect();
    let req = Request { prompt: VoiceProfile::extraction_prompt(&samples), schema: VoiceProfile::extraction_schema(), model: None, effort: Effort::Quality, working_dir: working_dir.clone() };
    let mut profile = cli.run_typed_cancellable::<VoiceProfile>(&req, |_| {}, Arc::clone(&cancel))?.value;
    report.stage_secs.insert("extract".into(), lap.elapsed().as_secs());
    lap = Instant::now();

    // Stage 2: evidence Rust can find on its own.
    let verifier = Verifier::new(&pieces);
    let mut backing: BTreeMap<String, Backing> = BTreeMap::new();
    let mut word_ev: Vec<Evidence> = Vec::new();
    let mut kept: Vec<String> = Vec::new();
    for w in &profile.preferred_words {
        let ev = quotes_for(&pieces, w, 2);
        if ev.is_empty() {
            report.dropped_words.push(format!("{w} (not found)"));
            continue;
        }
        kept.push(w.clone());
        word_ev.extend(ev);
    }
    if !kept.is_empty() { profile.preferred_words = kept; }
    report.quotes_offered += word_ev.len();
    report.quotes_verified += word_ev.len();
    backing.insert("preferred_words".into(), Backing { confidence: confidence_for(&word_ev, &verifier), evidence: word_ev, note: String::new() });
    let mut end_ev: Vec<Evidence> = Vec::new();
    for e in &profile.sentence_endings { end_ev.extend(quotes_for(&pieces, e, 1)); }
    backing.insert("sentence_endings".into(), Backing { confidence: confidence_for(&end_ev, &verifier), evidence: end_ev, note: String::new() });
    let fp_ev = if profile.first_person.trim().is_empty() { vec![] } else { quotes_for(&pieces, &profile.first_person, 3) };
    backing.insert("first_person".into(), Backing { confidence: confidence_for(&fp_ev, &verifier), evidence: fp_ev, note: String::new() });
    let emoji_ok = profile.uses_emoji == (measured.emoji > 0);
    backing.insert("uses_emoji".into(), Backing { confidence: if emoji_ok { Confidence::High } else { Confidence::Low }, evidence: vec![], note: if emoji_ok { String::new() } else { "measured emoji count disagrees".into() } });
    profile.uses_emoji = measured.emoji > 0;
    report.stage_secs.insert("evidence".into(), lap.elapsed().as_secs());
    lap = Instant::now();

    // Stage 3: adversarial check, unless out of time.
    if cfg.self_check && started.elapsed() <= cfg.soft_limit {
        on_stage(Stage::Checking);
        let req = Request { prompt: check_prompt(&pieces, &profile), schema: check_schema(), model: None, effort: Effort::Quality, working_dir: working_dir.clone() };
        match cli.run_typed_cancellable::<CheckReport>(&req, |_| {}, Arc::clone(&cancel)) {
            Ok(run) => {
                let mut fits = 0usize;
                let mut considered = 0usize;
                for item in &run.value.items {
                    if !FIELDS.contains(&item.field.as_str()) { continue; }
                    considered += 1;
                    report.verdicts.insert(item.field.clone(), item.verdict.clone());
                    report.quotes_offered += 1;
                    let verified = verifier.find(&item.quote, item.piece.checked_sub(1));
                    if verified.is_some() { report.quotes_verified += 1; }
                    let b = backing.entry(item.field.clone()).or_insert(Backing { confidence: Confidence::Low, evidence: vec![], note: String::new() });
                    match item.verdict.as_str() {
                        "fits" => {
                            fits += 2;
                            if let Some(pi) = verified {
                                if !b.evidence.iter().any(|e| normalize(&e.quote) == normalize(&item.quote)) { b.evidence.push(Evidence { piece: pi, quote: item.quote.clone() }); }
                            }
                            b.confidence = confidence_for(&b.evidence, &verifier).max(if verified.is_some() { Confidence::Medium } else { Confidence::Low });
                        }
                        "partial" => {
                            fits += 1;
                            if !item.note.is_empty() { b.note = item.note.clone(); }
                            if b.evidence.is_empty() { b.confidence = Confidence::Low; } else { b.confidence = b.confidence.min(Confidence::Medium); }
                        }
                        _ => {
                            b.confidence = Confidence::Low;
                            if verified.is_some() && !item.note.is_empty() { b.note = item.note.clone(); }
                        }
                    }
                }
                report.check_fit_pct = Some(if considered == 0 { 100 } else { fits * 50 / considered });
            }
            Err(e) => report.notes.push(format!("self-check failed: {e}")),
        }
        report.stage_secs.insert("check".into(), lap.elapsed().as_secs());
    } else if cfg.self_check {
        report.notes.push("self-check skipped: soft time limit".into());
    }

    for f in FIELDS {
        let entry = backing.entry((*f).into()).or_insert(Backing { confidence: Confidence::Low, evidence: vec![], note: String::new() });
        *report.confidence.entry(match entry.confidence { Confidence::High => "high", Confidence::Medium => "medium", Confidence::Low => "low" }.into()).or_default() += 1;
    }
    profile.backing = backing;
    profile.measured = Some(measured);
    report.total_secs = started.elapsed().as_secs();
    on_stage(Stage::Done);
    Ok(Outcome { profile, report })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trim_keeps_head_and_tail() {
        let text: String = (0..100).map(|i| char::from_u32('あ' as u32 + (i % 20)).unwrap()).collect();
        let t = trim_piece(&text, 10, 5);
        assert!(t.starts_with(&text.chars().take(10).collect::<String>()));
        assert!(t.ends_with(&text.chars().skip(95).collect::<String>()));
        assert!(t.contains("中略"));
        assert_eq!(trim_piece("短い", 10, 5), "短い");
    }

    #[test]
    fn quotes_are_verified_ignoring_whitespace() {
        let v = Verifier::new(&["資本政策は、何を諦めるかを先に決める作業だ。".into(), "別の記事。".into()]);
        assert_eq!(v.find("何を諦めるかを 先に決める", Some(1)), Some(0));
        assert_eq!(v.find("存在しない引用", None), None);
        assert_eq!(v.find("短い", None), None);
    }

    #[test]
    fn confidence_follows_the_rule() {
        let v = Verifier::new(&["結局は構造だ。構造が先。".into(), "構造を見る。".into()]);
        let two_pieces = vec![Evidence { piece: 0, quote: "結局は構造だ".into() }, Evidence { piece: 1, quote: "構造を見る".into() }];
        assert_eq!(confidence_for(&two_pieces, &v), Confidence::Medium);
        let strong = vec![Evidence { piece: 0, quote: "構造".into() }, Evidence { piece: 1, quote: "構造".into() }];
        assert_eq!(confidence_for(&strong, &v), Confidence::High);
        assert_eq!(confidence_for(&[], &v), Confidence::Low);
    }

    #[test]
    fn schemas_describe_every_property() {
        fn walk(v: &Value) {
            if let Some(props) = v.get("properties").and_then(Value::as_object) {
                for (k, p) in props { assert!(p.get("description").is_some(), "{k} lacks a description"); walk(p); }
            }
            if let Some(items) = v.get("items") { walk(items); }
        }
        walk(&check_schema());
    }
}
