//! The voice-quality PoC (requirement 2026-09-23, acceptance 7): builds a
//! voice the old way (one call) and the careful way (four stages) from the
//! same material, prints the numbers that show whether rigour went up, and
//! writes one article per voice with the answer key kept apart.
//!
//! `cargo run -p lita-codex --example voice_quality -- <material dir> <out dir> "<brief>"`
//! Uses the account's Codex quota (several minutes).

use lita_codex::post::{generation_prompt, post_schema, PlatformRules, PostDraft};
use lita_codex::voice::{pipeline, select_material, Budget, Confidence, VoiceProfile};
use lita_codex::{CodexCli, Effort, Request};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Instant;

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let dir = PathBuf::from(args.next().expect("material dir"));
    let out = PathBuf::from(args.next().expect("out dir"));
    let brief = args.next().unwrap_or_else(|| "最近考えていることを一つ、読者に向けて。".into());
    std::fs::create_dir_all(&out)?;
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)?.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| matches!(p.extension().and_then(|x| x.to_str()), Some("md") | Some("txt"))).collect();
    files.sort();
    let pieces: Vec<String> = files.iter().map(|f| std::fs::read_to_string(f).unwrap_or_default()).filter(|s| !s.trim().is_empty()).collect();
    let refs: Vec<&str> = pieces.iter().map(String::as_str).collect();
    eprintln!("material: {} pieces, {} chars", pieces.len(), pieces.iter().map(|p| p.chars().count()).sum::<usize>());
    let cli = CodexCli::on_path();
    let wd = std::env::temp_dir();

    // Old way.
    let t = Instant::now();
    let material = select_material(&refs, Budget::default());
    let samples: Vec<&str> = material.samples.iter().map(String::as_str).collect();
    let req = Request { prompt: VoiceProfile::extraction_prompt(&samples), schema: VoiceProfile::extraction_schema(), model: None, effort: Effort::Quality, working_dir: wd.clone() };
    let old = cli.run_typed::<VoiceProfile>(&req, |_| {})?.value;
    let old_secs = t.elapsed().as_secs();
    eprintln!("old: {old_secs}s  words={:?}", old.preferred_words);

    // Careful way.
    let outcome = pipeline::extract(&cli, &refs, &pipeline::Config::default(), wd.clone(), Arc::new(AtomicBool::new(false)), |s| eprintln!("  stage: {s:?}"))?;
    let new = outcome.profile;
    let r = &outcome.report;
    eprintln!("new: {}s  words={:?}", r.total_secs, new.preferred_words);

    let stops: Vec<&String> = new.preferred_words.iter().filter(|w| lita_codex::voice::stoplist::is_stop(w)).collect();
    let lows = new.backing.values().filter(|b| b.confidence == Confidence::Low).count();
    let with_evidence = new.backing.values().filter(|b| !b.evidence.is_empty()).count();
    println!("== numbers ==");
    println!("old_total_secs\t{old_secs}");
    println!("new_total_secs\t{}", r.total_secs);
    println!("stage_secs\t{}", serde_json::to_string(&r.stage_secs)?);
    println!("pieces_read\t{}/{}", r.pieces_read, r.pieces_total);
    println!("quotes_verified\t{}/{}", r.quotes_verified, r.quotes_offered);
    println!("confidence\t{}", serde_json::to_string(&r.confidence)?);
    println!("fields_with_evidence\t{}/{}", with_evidence, pipeline::FIELDS.len());
    println!("check_fit_pct\t{:?}", r.check_fit_pct);
    println!("redo\t{}\tfell_back\t{}", r.redo, r.fell_back);
    println!("preferred_words_new\t{} (stop words among them: {})", new.preferred_words.len(), stops.len());
    println!("dropped_words\t{:?}", r.dropped_words);
    println!("low_confidence_fields\t{lows}");
    println!("notes\t{:?}", r.notes);
    println!("verdicts\t{}", serde_json::to_string(&r.verdicts)?);
    println!("avoided\t{:?}\tnever_does\t{:?}", new.avoided_words, new.never_does);

    std::fs::write(out.join("旧.json"), serde_json::to_string_pretty(&old)?)?;
    std::fs::write(out.join("新.json"), serde_json::to_string_pretty(&new)?)?;
    std::fs::write(out.join("report.json"), serde_json::to_string_pretty(r)?)?;

    // One article each, order hidden.
    let platform = PlatformRules { name: "note".into(), max_chars: None, rules: String::new() };
    let write = |p: &VoiceProfile| -> anyhow::Result<String> {
        let req = Request { prompt: generation_prompt(p, &brief, &platform), schema: post_schema(), model: None, effort: Effort::Quality, working_dir: wd.clone() };
        let d = cli.run_typed::<PostDraft>(&req, |_| {})?.value;
        Ok(if d.title.is_empty() { d.text } else { format!("# {}\n\n{}", d.title, d.text) })
    };
    let a = write(&old)?;
    let b = write(&new)?;
    let flip = (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos() % 2) == 1;
    let (one, two, key) = if flip { (b, a, "記事-1 = 新, 記事-2 = 旧") } else { (a, b, "記事-1 = 旧, 記事-2 = 新") };
    std::fs::write(out.join("記事-1.md"), one)?;
    std::fs::write(out.join("記事-2.md"), two)?;
    std::fs::write(out.join("答え.txt"), key)?;
    println!("written to {}", out.display());
    Ok(())
}
