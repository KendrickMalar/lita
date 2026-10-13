//! Runs one real long-form generation (note) on a hand-made profile and
//! prints the title, the length and the first lines. Uses about a minute
//! of the account's Codex quota.

use lita_codex::post::{PlatformRules, PostDraft, generation_prompt, post_schema};
use lita_codex::voice::VoiceProfile;
use lita_codex::{CodexCli, Effort, Request};

fn main() -> anyhow::Result<()> {
    let profile = VoiceProfile {
        language: "ja".into(), first_person: "私".into(), formality: "常体".into(),
        tone: vec!["分析的".into(), "懐疑的".into()],
        sentence_endings: vec!["のだろうか。".into(), "と思う。".into(), "ではないか。".into()],
        avg_sentence_length_chars: 42,
        preferred_words: vec!["エクイティ".into(), "ファイナンス".into(), "構造".into()],
        avoided_words: vec!["絶対".into()], opens_with: "問いから入る".into(), closes_with: "含みを残して締める".into(),
        uses_emoji: false, representative_excerpts: vec![], one_line: String::new(), ..Default::default()
    };
    let note = PlatformRules { name: "note".into(), max_chars: None, rules: "A long-form article for note.com. Plain text: a title on its own (returned separately), then paragraphs separated by blank lines. Use headings sparingly as a line starting with \"## \". No markdown emphasis, no hashtags, no links unless the brief provides one.".into() };
    let brief = "投資家との初回面談で何を聞くべきか。創業者向けに、彼らが何を恐れているかを最初に聞け、という一点を掘り下げる。1,500 字ほど。";
    let req = Request { prompt: generation_prompt(&profile, brief, &note), schema: post_schema(), model: None, effort: Effort::Quality, working_dir: std::env::temp_dir() };
    let run = CodexCli::on_path().run_typed::<PostDraft>(&req, |_| {})?;
    let d = run.value;
    let heads = d.text.lines().filter(|l| l.starts_with("## ")).count();
    let paras = d.text.split("\n\n").filter(|p| !p.trim().is_empty()).count();
    println!("elapsed: {:?}\ntitle: {}\nchars: {} (self {}), paragraphs: {paras}, headings: {heads}\n---\n{}\n---\nnotes: {}", run.elapsed, d.title, d.text.chars().count(), d.char_count, d.text, d.voice_notes);
    Ok(())
}
