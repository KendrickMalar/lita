//! Runs one real "rewrite shorter" (#40) on a hand-made profile and an
//! over-long draft, and prints what comes back. Uses about ten seconds of
//! the account's Codex quota.

use lita_codex::post::{PlatformRules, PostDraft, post_schema, shorten_prompt};
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
    let platform = PlatformRules { name: "X".into(), max_chars: Some(280), rules: "One post. No hashtags.".into() };
    let brief = "資本政策の相談を受けたときに最初に聞くことについて。創業者に向けて、エクイティは時間を売る契約だと伝えたい。";
    let previous = "資本政策の相談で最初に聞くのは、いくら欲しいかではなく、何を諦められるかだ。エクイティは返さなくていい金ではなく、時間と自由を先に売る契約なのだろうか。ファイナンスの選択肢を並べる前に、まず自分が手放せないものを一つ決める。そこから逆算すると、借入か出資かの答えはほとんど決まっていると思う。それでも迷うなら、迷っている理由の方が本題ではないか。資本政策の話は、結局のところ何を諦めるかの順番を決める作業でしかないのだろうか。それを最初に言葉にできるかどうかで、その後の交渉の景色は変わると思う。";
    println!("previous: {} chars", previous.chars().count());
    let req = Request { prompt: shorten_prompt(&profile, brief, &platform, previous), schema: post_schema(), model: None, effort: Effort::Fast, working_dir: std::env::temp_dir() };
    let run = CodexCli::on_path().run_typed::<PostDraft>(&req, |_| {})?;
    println!("elapsed: {:?}\nresult: {} chars (self-reported {})\n{}\nnotes: {}", run.elapsed, run.value.text.chars().count(), run.value.char_count, run.value.text, run.value.voice_notes);
    Ok(())
}
