//! Runs one real extraction on four short samples and prints the profile's
//! `one_line`, to check what Codex writes for the top of the Voice screen.
//! Uses the account's Codex quota (about a minute).

use lita_codex::{CodexCli, Effort, Request};
use lita_codex::voice::VoiceProfile;

fn main() -> anyhow::Result<()> {
    let samples = [
        "資本政策の相談で最初に聞くのは、いくら欲しいかではなく、何を諦められるかだ。エクイティは返さなくていい金ではなく、時間と自由を先に売る契約なのだろうか。",
        "ファイナンスの話は結局、時間をどう買うかに帰着すると思う。借入か出資かは手段の違いでしかなく、先に決めるべきは手放せないものの方ではないか。",
        "利益率の議論が空回りするのは、分母が揃っていないからだ。売上で割るのか、投下資本で割るのか。そこを曖昧にしたまま数字を並べても、構造は見えてこない。",
        "投資家との最初の会話で聞くべきことは一つで、彼らが何を恐れているかだ。リターンの話は後からいくらでもできる。恐れが分かれば、こちらの提案の形はほとんど決まる。",
    ];
    let req = Request {
        prompt: VoiceProfile::extraction_prompt(&samples),
        schema: VoiceProfile::extraction_schema(),
        model: None,
        effort: Effort::Quality,
        working_dir: std::env::temp_dir(),
    };
    let run = CodexCli::on_path().run_typed::<VoiceProfile>(&req, |_| {})?;
    println!("elapsed: {:?}", run.elapsed);
    println!("one_line: {}", run.value.one_line);
    println!("formality: {} | tone: {:?} | endings: {:?} | words: {:?}", run.value.formality, run.value.tone, run.value.sentence_endings, run.value.preferred_words);
    Ok(())
}
