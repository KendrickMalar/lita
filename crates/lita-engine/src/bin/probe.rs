//! Runs Lita's two real Codex tasks end to end against the local CLI.
//!
//! This exists to answer one question before any UI is built: can we spawn the
//! user's Codex CLI and get back JSON we can rely on? Run it with
//! `cargo run -p lita-codex --bin probe`.

use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use lita_codex::voice::VoiceProfile;
use lita_codex::{CodexCli, Effort, Event, Preflight, Request};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Stand-in for what Lita will pull out of Slack. Deliberately messy: different
/// lengths, a typo, a half-finished thought — real messages are not clean.
const SAMPLE_MESSAGES: &[&str] = &[
    "リリースノート書き直しました。前のやつ、機能の説明しかしてなくて「で、何が嬉しいの？」が抜けてたので。",
    "今日の取材、思ったより深掘りされて焦りました。数字の根拠を聞かれたときに即答できなかったのが悔しい。次までに整理しておきます",
    "資料できました！ざっと見てもらえると助かります 🙏",
    "個人的には、機能を全部並べるより「これ1つだけ覚えて帰ってください」を決めたほうが刺さると思ってます。全部言うと何も残らないので。",
    "すみません、さっきの数字間違ってました。正しくは前月比 +12% です。訂正します",
    "note の記事、公開しました。反応見ながら次の企画考えます",
    "採用広報って、結局は「うちで働くとどうなるか」を具体的に書けるかどうかだと思っていて。制度の説明だけだとどこの会社も同じに見えちゃう。",
    "打ち合わせ、15分押しそうです。先に始めててください",
];

#[derive(Debug, Deserialize, Serialize)]
struct Post {
    text: String,
    char_count: i64,
    voice_notes: String,
}

fn post_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["text", "char_count", "voice_notes"],
        "properties": {
            "text": { "type": "string" },
            "char_count": { "type": "integer" },
            "voice_notes": { "type": "string" }
        }
    })
}

/// Records how the event stream behaved, so we can tell whether a progress UI
/// is actually feasible or whether everything lands at once at the end.
struct EventTrace {
    started: Instant,
    first_event_at: Option<Duration>,
    kinds: Vec<String>,
}

impl EventTrace {
    fn new() -> Self {
        Self {
            started: Instant::now(),
            first_event_at: None,
            kinds: Vec::new(),
        }
    }

    fn record(&mut self, event: &Event) {
        if self.first_event_at.is_none() {
            self.first_event_at = Some(self.started.elapsed());
        }
        if !self.kinds.contains(&event.kind) {
            self.kinds.push(event.kind.clone());
        }
    }
}

fn main() -> Result<()> {
    let codex = CodexCli::on_path();

    println!("== preflight ==");
    let version = match codex.preflight()? {
        Preflight::Ready { version } => {
            println!("  codex {version}, credentials configured");
            version
        }
        Preflight::NotLoggedIn { version } => {
            bail!("codex {version} is installed but not logged in. Run `codex login` and retry.");
        }
        Preflight::NotInstalled => {
            bail!("no `codex` binary on PATH. Install the Codex CLI and retry.");
        }
        Preflight::ConfigBroken { version, detail } => {
            bail!("codex {version} cannot load its configuration: {detail}. Run `codex doctor` and fix it.");
        }
    };

    // Run somewhere that is deliberately NOT a git repository. Lita's data
    // directory never will be, and this is what --skip-git-repo-check buys us.
    let workdir = tempfile::tempdir()?;
    let is_repo = workdir.path().join(".git").exists();
    println!(
        "  working directory: {} (git repo: {is_repo})",
        workdir.path().display()
    );

    // --- Task 1: turn raw messages into a Voice profile -------------------
    println!(
        "\n== task 1: extract a voice profile from {} messages ==",
        SAMPLE_MESSAGES.len()
    );
    let corpus = SAMPLE_MESSAGES
        .iter()
        .map(|m| format!("- {m}"))
        .collect::<Vec<_>>()
        .join("\n");
    let voice_prompt = format!(
        "You are analysing one person's writing so that another writer can imitate it \
         convincingly. Below are messages they wrote in a work chat.\n\n{corpus}\n\n\
         Describe their voice. Be concrete and specific to these messages: a generic \
         description is useless. `avg_sentence_length_chars` is the mean sentence length in \
         characters. `representative_excerpts` should quote at most three short passages \
         verbatim and say what each one reveals. Write the descriptive values in Japanese."
    );
    lita_codex::assert_no_credentials(&voice_prompt);

    let mut trace = EventTrace::new();
    let voice = codex.run_typed::<VoiceProfile>(
        &Request {
            prompt: voice_prompt,
            schema: VoiceProfile::extraction_schema(),
            model: None,
            effort: Effort::Quality,
            working_dir: workdir.path().to_path_buf(),
        },
        |e| trace.record(e),
    )?;
    report_trace(&trace, voice.elapsed, voice.events.len());
    println!("{}", serde_json::to_string_pretty(&voice.value)?);

    // --- Task 2: write a post in that voice -------------------------------
    println!("\n== task 2: write an X post in that voice ==");
    let brief = "Announce that our v2 is out. Aimed at people already using v1. \
                 The one thing to remember: migration takes under five minutes.";
    let post_prompt = format!(
        "Write a single post for X in the voice described by this profile.\n\n\
         VOICE PROFILE:\n{}\n\nBRIEF:\n{brief}\n\n\
         Write in Japanese. Stay under 140 characters. Match the profile's first person, \
         sentence endings and emoji habit exactly — the profile is a contract, not a \
         suggestion. `char_count` is the character count of `text`. In `voice_notes`, say in \
         one sentence which parts of the profile you leaned on.",
        serde_json::to_string_pretty(&voice.value.generation_view())?
    );
    lita_codex::assert_no_credentials(&post_prompt);

    let mut trace = EventTrace::new();
    let post = codex.run_typed::<Post>(
        &Request {
            prompt: post_prompt,
            schema: post_schema(),
            model: None,
            effort: Effort::Quality,
            working_dir: workdir.path().to_path_buf(),
        },
        |e| trace.record(e),
    )?;
    report_trace(&trace, post.elapsed, post.events.len());
    println!("{}", serde_json::to_string_pretty(&post.value)?);

    let actual_chars = post.value.text.chars().count();
    println!(
        "\n  reported char_count: {} / actual: {actual_chars} / within 140: {}",
        post.value.char_count,
        actual_chars <= 140
    );

    println!("\n== verdict ==");
    println!("  codex version        {version}");
    println!("  typed JSON           both tasks deserialized into Rust structs");
    println!("  ran outside a repo   yes");
    println!(
        "  total time           {:.1}s",
        (voice.elapsed + post.elapsed).as_secs_f64()
    );
    Ok(())
}

fn report_trace(trace: &EventTrace, elapsed: Duration, events: usize) {
    let first = trace
        .first_event_at
        .map(|d| format!("{:.1}s", d.as_secs_f64()))
        .unwrap_or_else(|| "never".to_string());
    println!(
        "  {events} events in {:.1}s, first at {first}\n  event kinds: {}",
        elapsed.as_secs_f64(),
        trace.kinds.join(", ")
    );
}
