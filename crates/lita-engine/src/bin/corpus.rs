//! Measures how much writing a usable Voice actually needs.
//!
//! This gates a scope decision: a free Slack workspace only exposes the last
//! 90 days of messages, so if a Voice needs hundreds of messages, free-plan
//! users cannot build one. Run with `cargo run -p lita-codex --bin corpus`.

use anyhow::Result;
use lita_codex::voice::VoiceProfile;
use lita_codex::{CodexCli, Effort, Preflight, Request};

use lita_codex::sample::WORK_CHAT as MESSAGES;

const SIZES: &[usize] = &[4, 8, 24];

fn main() -> Result<()> {
    let codex = CodexCli::on_path();
    match codex.preflight()? {
        Preflight::Ready { version } => println!("codex {version}\n"),
        other => anyhow::bail!("codex is not ready: {other:?}"),
    }
    let workdir = tempfile::tempdir()?;

    let mut results = Vec::new();
    for &n in SIZES {
        let corpus = MESSAGES[..n]
            .iter()
            .map(|m| format!("- {m}"))
            .collect::<Vec<_>>()
            .join("\n");
        let prompt = format!(
            "You are analysing one person's writing so that another writer can imitate it \
             convincingly. Below are messages they wrote in a work chat.\n\n{corpus}\n\n\
             Describe their voice, following every field description exactly. Write the \
             descriptive values in Japanese."
        );
        let run = codex.run_typed::<VoiceProfile>(
            &Request {
                prompt,
                schema: VoiceProfile::extraction_schema(),
                model: None,
                effort: Effort::Quality,
                working_dir: workdir.path().to_path_buf(),
            },
            |_| {},
        )?;
        println!("--- n = {n} ({:.0}s) ---", run.elapsed.as_secs_f64());
        println!("{}", serde_json::to_string_pretty(&run.value)?);
        println!();
        results.push((n, run.value));
    }

    println!("=== how the profile moved as the corpus grew ===");
    for (n, p) in &results {
        println!(
            "n={n:>2}  first_person={:<8} emoji={:<5} avg_len={:>3}  tone={}",
            if p.first_person.is_empty() {
                "(none)"
            } else {
                &p.first_person
            },
            p.uses_emoji,
            p.avg_sentence_length_chars,
            p.tone.join("/")
        );
    }
    Ok(())
}
