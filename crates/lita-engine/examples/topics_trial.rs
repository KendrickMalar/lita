//! Tries the title suggestion and the policy draft on real articles:
//! `cargo run -p lita-codex --example topics_trial -- <dir of .md> [direction]`

use lita_codex::topics::{self, Existing, Lang, Policy, Topics};
use lita_codex::{CodexCli, Effort, Request};

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let dir = std::path::PathBuf::from(args.next().expect("dir"));
    let direction = args.next().unwrap_or_default();
    let mut samples = Vec::new();
    let mut existing = Vec::new();
    let mut names: Vec<_> = std::fs::read_dir(&dir)?.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "md")).collect();
    names.sort();
    for p in names {
        let text = std::fs::read_to_string(&p)?;
        let title = text.lines().next().unwrap_or("").trim_start_matches("# ").to_string();
        let body = text.lines().skip(2).collect::<Vec<_>>().join("\n");
        existing.push(Existing { title, first_line: body.lines().find(|l| !l.trim().is_empty()).unwrap_or("").to_string() });
        samples.push(body);
    }
    let refs: Vec<&str> = samples.iter().map(String::as_str).collect();
    let cli = CodexCli::on_path();
    let wd = std::env::temp_dir();

    let t = std::time::Instant::now();
    let req = Request { prompt: topics::policy_prompt(&refs, &existing, Lang::Ja), schema: topics::policy_schema(), model: None, effort: Effort::Quality, working_dir: wd.clone() };
    let policy: Policy = cli.run_typed::<Policy>(&req, |_| {})?.value;
    println!("== policy ({}s) ==\n{}", t.elapsed().as_secs(), serde_json::to_string_pretty(&policy)?);

    for (label, pol) in [("empty policy", Policy::default()), ("drafted policy", policy)] {
        let t = std::time::Instant::now();
        let req = Request { prompt: topics::topics_prompt(&pol, &refs, &existing, &[], &direction, 14, Lang::Ja), schema: topics::topics_schema(), model: None, effort: Effort::Quality, working_dir: wd.clone() };
        let out = cli.run_typed::<Topics>(&req, |_| {})?.value;
        let kept = topics::dedupe(out.topics.clone(), &existing, 10);
        println!("== titles with {label} ({}s, {} returned, {} kept) ==", t.elapsed().as_secs(), out.topics.len(), kept.len());
        for k in &kept { println!("- {k}"); }
    }
    Ok(())
}
