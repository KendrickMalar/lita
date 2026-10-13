//! Runs one real job through one engine, to check that Lita can drive it:
//! `cargo run -p lita-engine --example engine_trial -- <claude|codex> <post|topics|voice>`
//!
//! This starts the real `claude` or `codex` CLI and uses the quota of the
//! account the person is logged into, so it is not part of the automated
//! tests. It is used for the acceptance check (AC005) on a machine where the
//! chosen CLI is installed and logged in.
//!
//! The only writing it reads is `lita_engine::sample::WORK_CHAT`. It prints
//! the engine, the kind of job, how long it took, whether the output matched
//! the schema, and a short summary. It never prints `claude auth status`
//! output. A failed run exits with code 1; bad arguments exit with code 2.

use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use lita_engine::post::{PlatformRules, PostDraft, generation_prompt, post_schema};
use lita_engine::sample::WORK_CHAT;
use lita_engine::topics::{Lang, Policy, Topics, topics_prompt, topics_schema};
use lita_engine::voice::pipeline::{self, Config};
use lita_engine::{Effort, Engine, EngineKind, Preflight, Request, RunFailure};

const USAGE: &str = "usage: cargo run -p lita-engine --example engine_trial -- <claude|codex> <post|topics|voice>";

#[derive(Clone, Copy)]
enum Job {
    Post,
    Topics,
    Voice,
}

impl Job {
    fn name(self) -> &'static str {
        match self {
            Job::Post => "post",
            Job::Topics => "topics",
            Job::Voice => "voice",
        }
    }
}

fn parse_args() -> Option<(EngineKind, Job)> {
    let mut args = std::env::args().skip(1);
    let kind = match args.next()?.as_str() {
        "claude" => EngineKind::Claude,
        "codex" => EngineKind::Codex,
        _ => return None,
    };
    let job = match args.next()?.as_str() {
        "post" => Job::Post,
        "topics" => Job::Topics,
        "voice" => Job::Voice,
        _ => return None,
    };
    if args.next().is_some() {
        return None;
    }
    Some((kind, job))
}

fn main() -> ExitCode {
    let Some((kind, job)) = parse_args() else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    let engine = Engine::on_path(kind);
    let name = kind.display_name();

    // Say only which kind of answer the preflight gave, plus the version.
    match engine.preflight() {
        Ok(Preflight::Ready { version }) => println!("preflight: {name} Ready (version {version})"),
        Ok(Preflight::NotLoggedIn { version }) => {
            println!("preflight: {name} NotLoggedIn (version {version})");
            return ExitCode::from(1);
        }
        Ok(Preflight::NotInstalled) => {
            println!("preflight: {name} NotInstalled");
            return ExitCode::from(1);
        }
        Ok(Preflight::ConfigBroken { version, .. }) => {
            println!("preflight: {name} ConfigBroken (version {version})");
            return ExitCode::from(1);
        }
        Err(e) => {
            println!("preflight: {name} failed: {e:#}");
            return ExitCode::from(1);
        }
    }

    let started = Instant::now();
    match run(&engine, job) {
        Ok(summary) => {
            println!("engine: {name}");
            println!("job: {}", job.name());
            println!("elapsed: {}s", started.elapsed().as_secs());
            println!("schema: matched");
            println!("{summary}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            println!("engine: {name}");
            println!("job: {}", job.name());
            println!("elapsed: {}s", started.elapsed().as_secs());
            match e.downcast_ref::<RunFailure>() {
                Some(f) => println!(
                    "failed: engine={:?} kind={:?} exit_code={:?}",
                    f.engine, f.kind, f.exit_code
                ),
                None => println!("failed: {e:#}"),
            }
            ExitCode::from(1)
        }
    }
}

fn run(engine: &Engine, job: Job) -> anyhow::Result<String> {
    let working_dir = std::env::temp_dir();
    match job {
        Job::Post => {
            let (_, profile) = lita_engine::presets::find("pr-polite-ja").expect("preset");
            let platform = PlatformRules {
                name: "X".into(),
                max_chars: Some(140),
                rules: "A short post for X. Plain text, no hashtags, no links, no markdown.".into(),
            };
            let recent = WORK_CHAT.iter().take(5).map(|m| format!("- {m}")).collect::<Vec<_>>().join("\n");
            let brief = format!(
                "最近のやりとり:\n{recent}\n\nこの中の1つを題に、短い投稿を1本書く。\n外部の事実は調べず、自分の経験と意見の範囲で書く。数字や出来事を断定しない。"
            );
            let req = Request {
                prompt: generation_prompt(&profile, &brief, &platform),
                schema: post_schema(),
                model: None,
                effort: Effort::Quality,
                working_dir,
            };
            let draft = engine.run_typed::<PostDraft>(&req, |_| {})?.value;
            Ok(format!("post: {} chars\n{}", draft.text.chars().count(), draft.text))
        }
        Job::Topics => {
            let req = Request {
                prompt: topics_prompt(&Policy::default(), WORK_CHAT, &[], &[], "", 5, Lang::Ja),
                schema: topics_schema(),
                model: None,
                effort: Effort::Quality,
                working_dir,
            };
            let topics = engine.run_typed::<Topics>(&req, |_| {})?.value;
            let titles = topics.topics.iter().map(|t| format!("- {t}")).collect::<Vec<_>>().join("\n");
            Ok(format!("topics: {}\n{titles}", topics.topics.len()))
        }
        Job::Voice => {
            let cancel = Arc::new(AtomicBool::new(false));
            let outcome = pipeline::extract(engine, WORK_CHAT, &Config::default(), working_dir, cancel, |_| {})?;
            Ok(format!("voice one_line: {}", outcome.profile.one_line))
        }
    }
}
