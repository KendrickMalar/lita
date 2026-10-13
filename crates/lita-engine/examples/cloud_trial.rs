//! Tries the topic cloud on real articles:
//! `cargo run -p lita-codex --example cloud_trial -- <dir of .md>`
use lita_codex::topics::{self, Existing, GatheredCloud, Lang, Policy};
use lita_codex::{CodexCli, Effort, Request};

fn main() -> anyhow::Result<()> {
    let dir = std::path::PathBuf::from(std::env::args().nth(1).expect("dir"));
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
    let t = std::time::Instant::now();
    let req = Request { prompt: topics::cloud_prompt(&Policy::default(), &refs, &existing, Lang::Ja), schema: topics::cloud_schema(), model: None, effort: Effort::Quality, working_dir: std::env::temp_dir() };
    let out = CodexCli::on_path().run_typed::<GatheredCloud>(&req, |_| {})?.value;
    let words = topics::tidy_cloud(out.words);
    println!("{}s, {} words", t.elapsed().as_secs(), words.len());
    for w in &words { println!("{} {} {}", w.weight, if w.written { "*" } else { " " }, w.word); }
    Ok(())
}
