//! Settles the Voice schema by measuring which fields actually steer output.
//!
//! Extracts one profile from the sample corpus, then writes the same post from
//! four different subsets of it. Every field we send costs latency on every
//! post, so the question is not "what describes a voice" but "what changes the
//! draft". Run with `cargo run -p lita-codex --bin schema`.

use anyhow::Result;
use lita_codex::sample::WORK_CHAT;
use lita_codex::voice::VoiceProfile;
use lita_codex::{CodexCli, Effort, Preflight, Request};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const BRIEF: &str = "Announce that our v2 is out. Aimed at people already using v1. \
                     The one thing to remember: migration takes under five minutes.";

#[derive(Debug, Deserialize, Serialize)]
struct Post {
    text: String,
}

fn post_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["text"],
        "properties": { "text": { "type": "string", "description":
            "A post for X in Japanese, under 140 characters." } }
    })
}

/// The subsets of a profile we are choosing between.
fn variants(p: &VoiceProfile) -> Vec<(&'static str, Value)> {
    vec![
        ("full", serde_json::to_value(p).unwrap()),
        ("structured-only", p.generation_view()),
        (
            "excerpts-only",
            json!({ "representative_excerpts": p.representative_excerpts }),
        ),
        (
            "minimal",
            json!({
                "first_person": p.first_person,
                "tone": p.tone,
                "sentence_endings": p.sentence_endings,
                "uses_emoji": p.uses_emoji,
            }),
        ),
    ]
}

/// How many times each variant is generated. One run per variant told us
/// nothing we could trust: the differences we cared about were the size of the
/// run-to-run variance.
const REPEATS: usize = 3;

/// The profile writes endings as "〜ました。" or "〜ます／〜ます。". Matching those
/// literally never succeeds, so reduce each to the forms that appear in text.
fn ending_forms(raw: &str) -> Vec<String> {
    raw.split('／')
        .map(|part| {
            part.trim()
                .trim_start_matches('〜')
                .trim_end_matches('。')
                .to_string()
        })
        .filter(|s| !s.is_empty())
        .collect()
}

fn has_emoji(s: &str) -> bool {
    s.chars().any(|c| {
        let c = c as u32;
        (0x1F300..=0x1FAFF).contains(&c) || (0x2600..=0x27BF).contains(&c)
    })
}

fn main() -> Result<()> {
    let codex = CodexCli::on_path();
    match codex.preflight()? {
        Preflight::Ready { version } => println!("codex {version}\n"),
        other => anyhow::bail!("codex is not ready: {other:?}"),
    }
    let workdir = tempfile::tempdir()?;
    let dir = workdir.path().to_path_buf();

    println!(
        "== extracting a profile from {} messages ==",
        WORK_CHAT.len()
    );
    let extracted = codex.run_typed::<VoiceProfile>(
        &Request {
            prompt: VoiceProfile::extraction_prompt(WORK_CHAT),
            schema: VoiceProfile::extraction_schema(),
            model: None,
            effort: Effort::Quality,
            working_dir: dir.clone(),
        },
        |_| {},
    )?;
    let profile = extracted.value;
    println!(
        "{:.0}s\n{}\n",
        extracted.elapsed.as_secs_f64(),
        serde_json::to_string_pretty(&profile)?
    );

    let endings: Vec<String> = profile
        .sentence_endings
        .iter()
        .flat_map(|e| ending_forms(e))
        .collect();

    println!("== writing the same post from four subsets, {REPEATS} times each ==\n");
    let mut rows = Vec::new();
    for (name, subset) in variants(&profile) {
        let voice_json = serde_json::to_string_pretty(&subset)?;
        let chars = voice_json.chars().count();
        let prompt = format!(
            "Write a single post for X in the voice described below.\n\n\
             VOICE:\n{voice_json}\n\nBRIEF:\n{BRIEF}\n\n\
             Write in Japanese, under 140 characters. The voice description is a contract, \
             not a suggestion."
        );
        println!("--- {name} (voice block {chars} chars) ---");
        let (mut secs, mut words, mut ends, mut emoji_ok) = (0.0, 0usize, 0usize, 0usize);
        for i in 1..=REPEATS {
            let run = codex.run_typed::<Post>(
                &Request {
                    prompt: prompt.clone(),
                    schema: post_schema(),
                    model: None,
                    effort: Effort::Fast,
                    working_dir: dir.clone(),
                },
                |_| {},
            )?;
            let text = run.value.text;
            let w = profile
                .preferred_words
                .iter()
                .filter(|x| text.contains(*x))
                .count();
            let e = endings.iter().filter(|x| text.contains(*x)).count();
            let em = has_emoji(&text) == profile.uses_emoji;
            secs += run.elapsed.as_secs_f64();
            words += w;
            ends += e;
            emoji_ok += usize::from(em);
            println!(
                "  {i}. [{:.0}s w{w} e{e} {}] {text}",
                run.elapsed.as_secs_f64(),
                if em { "emoji ok" } else { "EMOJI MISS" }
            );
        }
        println!();
        rows.push((
            name,
            secs / REPEATS as f64,
            chars,
            words as f64 / REPEATS as f64,
            ends as f64 / REPEATS as f64,
            emoji_ok,
        ));
    }

    println!("=== averages over {REPEATS} runs ===");
    println!(
        "{:<16} {:>7} {:>7} {:>11} {:>9} {:>10}",
        "variant", "time", "chars", "word hits", "endings", "emoji ok"
    );
    for (n, t, c, w, e, em) in &rows {
        println!("{n:<16} {t:>6.1}s {c:>7} {w:>11.1} {e:>9.1} {em:>7}/{REPEATS}");
    }
    println!(
        "\nprofile offers {} preferred words and {} ending forms to match",
        profile.preferred_words.len(),
        endings.len()
    );
    Ok(())
}
