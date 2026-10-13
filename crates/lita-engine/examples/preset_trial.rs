//! Writes one note article with a bundled voice, to read how it sounds:
//! `cargo run -p lita-codex --example preset_trial -- <preset id>`
use lita_codex::post::{PlatformRules, PostDraft, generation_prompt, post_schema};
use lita_codex::{CodexCli, Effort, Request};

fn main() -> anyhow::Result<()> {
    let id = std::env::args().nth(1).unwrap_or_else(|| "pr-polite-ja".into());
    let (name, profile) = lita_codex::presets::find(&id).expect("preset");
    let note = PlatformRules { name: "note".into(), max_chars: None, rules: "A long-form article for note.com. Plain text: a title on its own (returned separately), then paragraphs separated by blank lines. Use headings sparingly as a line starting with \"## \". No markdown emphasis, no hashtags, no links unless the brief provides one.".into() };
    let brief = "題: 新しい社内勉強会を始めた理由\n誰に向けて: 採用に興味のある求職者と取引先\n持ち帰ってほしいこと: 学びを続ける会社だと分かる\n長さ: 1,500〜3,000 字。\n外部の事実は調べず、自分の経験と意見の範囲で書く。数字や出来事を断定しない。";
    let req = Request { prompt: generation_prompt(&profile, brief, &note), schema: post_schema(), model: None, effort: Effort::Quality, working_dir: std::env::temp_dir() };
    let t = std::time::Instant::now();
    let out = CodexCli::on_path().run_typed::<PostDraft>(&req, |_| {})?.value;
    println!("== {name} ({}s, {} chars) ==\n# {}\n\n{}\n\n-- {}", t.elapsed().as_secs(), out.text.chars().count(), out.title, out.text, out.voice_notes);
    Ok(())
}
