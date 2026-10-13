//! Characterization tests for how Lita starts the Codex CLI and how it reads a
//! failure. They pin the argument order, stdin, working directory and failure
//! classification, so that moving `CodexCli` into another crate or module
//! cannot change what Codex is asked to do.
//!
//! Every test builds its own fake `codex` shell script in a temp directory and
//! points `CodexCli::at` at it. The real `codex` is never started, and no
//! process-wide environment variable is touched (the tests run in parallel).

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use lita_engine::{CodexCli, Effort, FailureKind, Preflight, Request, RunFailure};
use serde::Deserialize;
use serde_json::{json, Value};

const TURN_COMPLETED: &str = r#"{"type":"turn.completed"}"#;
const UNAUTHORIZED: &str = r#"{"type":"turn.failed","error":{"message":"unexpected status 401 Unauthorized: Missing bearer or basic authentication in header"}}"#;
const USAGE_LIMIT: &str = r#"{"type":"turn.failed","error":{"message":"You've hit your usage limit."}}"#;

/// A fake `codex` plus the files it leaves behind for the test to inspect.
struct Fake {
    dir: tempfile::TempDir,
    program: PathBuf,
}

/// How the fake's `login status` and `exec` behave.
struct Script {
    /// Printed by `login status`.
    login_output: &'static str,
    login_exit: i32,
    /// Lines printed to stdout by `exec`.
    exec_stdout: String,
    exec_exit: i32,
    /// Written to the `--output-last-message` path when `exec` succeeds.
    last_message: &'static str,
    /// `doctor --json` prints this and exits 0 (empty: the subcommand is unknown).
    doctor_json: String,
    /// Replaces the whole `exec` body (for example, to hang until killed).
    exec_override: Option<&'static str>,
}

impl Default for Script {
    fn default() -> Self {
        Script {
            login_output: "Logged in using ChatGPT",
            login_exit: 0,
            exec_stdout: TURN_COMPLETED.to_string(),
            exec_exit: 0,
            last_message: r#"{"text":"ok"}"#,
            doctor_json: String::new(),
            exec_override: None,
        }
    }
}

impl Fake {
    fn new(script: Script) -> Fake {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        fs::write(d.join("login-output.txt"), script.login_output).unwrap();
        fs::write(d.join("exec-stdout.txt"), format!("{}\n", script.exec_stdout)).unwrap();
        fs::write(d.join("last-message.json"), script.last_message).unwrap();
        fs::write(d.join("doctor.json"), &script.doctor_json).unwrap();

        let exec_body = script.exec_override.map(str::to_string).unwrap_or_else(|| {
            format!(
                r#"cat "$D/exec-stdout.txt"
  if [ {exit} -eq 0 ]; then cat "$D/last-message.json" > "$out"; fi
  exit {exit}"#,
                exit = script.exec_exit
            )
        });
        let body = format!(
            r#"#!/bin/sh
D='{dir}'
case "$1" in
login)
  echo "$*" >> "$D/calls.log"
  cat "$D/login-output.txt"
  exit {login_exit}
  ;;
doctor)
  echo "$*" >> "$D/calls.log"
  cat "$D/doctor.json"
  exit 0
  ;;
exec)
  echo "exec" >> "$D/calls.log"
  printf '[%s]\n' "$@" > "$D/exec-args.log"
  pwd -P > "$D/exec-cwd.txt"
  out=""
  schema=""
  prev=""
  for a in "$@"; do
    if [ "$prev" = "--output-last-message" ]; then out="$a"; fi
    if [ "$prev" = "--output-schema" ]; then schema="$a"; fi
    prev="$a"
  done
  cp "$schema" "$D/schema-copy.json"
  cat > "$D/exec-stdin.txt"
  {exec_body}
  ;;
*)
  echo "unexpected $*" >> "$D/calls.log"
  exit 64
  ;;
esac
"#,
            dir = d.display(),
            login_exit = script.login_exit,
            exec_body = exec_body,
        );
        let program = d.join("codex");
        fs::write(&program, body).unwrap();
        fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).unwrap();
        Fake { dir, program }
    }

    fn cli(&self) -> CodexCli {
        CodexCli::at(&self.program)
    }

    fn read(&self, name: &str) -> String {
        fs::read_to_string(self.dir.path().join(name)).unwrap_or_else(|e| panic!("{name}: {e}"))
    }

    fn exists(&self, name: &str) -> bool {
        self.dir.path().join(name).exists()
    }

    /// The `exec` arguments, one per element, as the fake saw them.
    fn exec_args(&self) -> Vec<String> {
        self.read("exec-args.log")
            .lines()
            .map(|l| {
                l.strip_prefix('[')
                    .and_then(|l| l.strip_suffix(']'))
                    .unwrap_or_else(|| panic!("unexpected argument line: {l:?}"))
                    .to_string()
            })
            .collect()
    }

    fn calls(&self) -> Vec<String> {
        self.read("calls.log").lines().map(str::to_string).collect()
    }
}

fn request(effort: Effort, model: Option<&str>, working_dir: &Path) -> Request {
    Request {
        prompt: "Write one short post.".to_string(),
        schema: json!({"type":"object","properties":{"text":{"type":"string"}},"required":["text"]}),
        model: model.map(str::to_string),
        effort,
        working_dir: working_dir.to_path_buf(),
    }
}

#[derive(Debug, Deserialize, PartialEq)]
struct Out {
    text: String,
}

/// Runs `req` against the fake and returns the failure it produced.
fn run_failure(fake: &Fake, req: &Request) -> RunFailure {
    match fake.cli().run_typed::<Value>(req, |_| {}) {
        Ok(_) => panic!("expected the run to fail"),
        Err(e) => match e.downcast::<RunFailure>() {
            Ok(f) => f,
            Err(e) => panic!("expected a RunFailure, got: {e:#}"),
        },
    }
}

/// Everything in the argument list except the two scratch paths, which are
/// checked by their file names only.
fn assert_exec_args(fake: &Fake, effort_value: &str, model: Option<&str>) {
    let args = fake.exec_args();
    let mut expected_tail: Vec<String> = vec![
        "--sandbox".into(),
        "read-only".into(),
        "--skip-git-repo-check".into(),
        "--ephemeral".into(),
        "--ignore-user-config".into(),
        "-c".into(),
        format!("model_reasoning_effort=\"{effort_value}\""),
    ];
    if let Some(m) = model {
        expected_tail.push("--model".into());
        expected_tail.push(m.into());
    }
    expected_tail.push("-".into());

    assert_eq!(args.len(), 6 + expected_tail.len(), "unexpected argument list: {args:?}");
    assert_eq!(args[0], "exec");
    assert_eq!(args[1], "--json");
    assert_eq!(args[2], "--output-schema");
    assert_eq!(Path::new(&args[3]).file_name().unwrap(), "schema.json", "{args:?}");
    assert_eq!(args[4], "--output-last-message");
    assert_eq!(Path::new(&args[5]).file_name().unwrap(), "final.json", "{args:?}");
    assert_eq!(&args[6..], &expected_tail[..], "{args:?}");
    // Both scratch files live in the same private directory.
    assert_eq!(Path::new(&args[3]).parent(), Path::new(&args[5]).parent());
}

#[test]
fn codex_exec_args_quality_without_model_are_in_the_pinned_order() {
    let fake = Fake::new(Script::default());
    let work = tempfile::tempdir().unwrap();
    let run = fake
        .cli()
        .run_typed::<Value>(&request(Effort::Quality, None, work.path()), |_| {})
        .unwrap_or_else(|e| panic!("{e:#}"));
    drop(run);

    // 14 arguments: exec --json --output-schema S --output-last-message F
    // --sandbox read-only --skip-git-repo-check --ephemeral --ignore-user-config
    // -c model_reasoning_effort="medium" -
    assert_exec_args(&fake, "medium", None);
    assert_eq!(fake.exec_args().len(), 14);
    assert!(!fake.exec_args().iter().any(|a| a == "--model"));
}

#[test]
fn codex_login_status_is_checked_first_and_exec_runs_once() {
    let fake = Fake::new(Script::default());
    let work = tempfile::tempdir().unwrap();
    fake.cli()
        .run_typed::<Value>(&request(Effort::Quality, None, work.path()), |_| {})
        .unwrap_or_else(|e| panic!("{e:#}"));
    assert_eq!(fake.calls(), vec!["login status".to_string(), "exec".to_string()]);
}

#[test]
fn codex_effort_fast_uses_low_reasoning() {
    let fake = Fake::new(Script::default());
    let work = tempfile::tempdir().unwrap();
    fake.cli()
        .run_typed::<Value>(&request(Effort::Fast, None, work.path()), |_| {})
        .unwrap_or_else(|e| panic!("{e:#}"));
    assert_exec_args(&fake, "low", None);
}

#[test]
fn codex_effort_best_uses_high_reasoning() {
    let fake = Fake::new(Script::default());
    let work = tempfile::tempdir().unwrap();
    fake.cli()
        .run_typed::<Value>(&request(Effort::Best, None, work.path()), |_| {})
        .unwrap_or_else(|e| panic!("{e:#}"));
    assert_exec_args(&fake, "high", None);
}

#[test]
fn codex_model_flag_goes_right_before_the_stdin_marker() {
    let fake = Fake::new(Script::default());
    let work = tempfile::tempdir().unwrap();
    fake.cli()
        .run_typed::<Value>(&request(Effort::Quality, Some("m"), work.path()), |_| {})
        .unwrap_or_else(|e| panic!("{e:#}"));
    assert_exec_args(&fake, "medium", Some("m"));
    let args = fake.exec_args();
    assert_eq!(args.len(), 16);
    assert_eq!(&args[args.len() - 3..], ["--model", "m", "-"]);
}

#[test]
fn codex_prompt_is_sent_on_stdin_verbatim() {
    let fake = Fake::new(Script::default());
    let work = tempfile::tempdir().unwrap();
    let mut req = request(Effort::Quality, None, work.path());
    req.prompt = "first line\n日本語の二行目 \"quoted\" $HOME `x`\n".to_string();
    fake.cli().run_typed::<Value>(&req, |_| {}).unwrap_or_else(|e| panic!("{e:#}"));
    assert_eq!(fake.read("exec-stdin.txt"), req.prompt);
    // The prompt is never an argument.
    assert!(!fake.exec_args().iter().any(|a| a.contains("first line")));
}

#[test]
fn codex_runs_in_the_requested_working_directory() {
    let fake = Fake::new(Script::default());
    let work = tempfile::tempdir().unwrap();
    fake.cli()
        .run_typed::<Value>(&request(Effort::Quality, None, work.path()), |_| {})
        .unwrap_or_else(|e| panic!("{e:#}"));
    let seen = fake.read("exec-cwd.txt");
    assert_eq!(Path::new(seen.trim_end()), fs::canonicalize(work.path()).unwrap());
}

#[test]
fn codex_output_schema_file_holds_the_request_schema() {
    let fake = Fake::new(Script::default());
    let work = tempfile::tempdir().unwrap();
    let req = request(Effort::Quality, None, work.path());
    fake.cli().run_typed::<Value>(&req, |_| {}).unwrap_or_else(|e| panic!("{e:#}"));
    let written: Value = serde_json::from_str(&fake.read("schema-copy.json")).unwrap();
    assert_eq!(written, req.schema);
}

#[test]
fn codex_run_returns_the_value_from_the_last_message_file() {
    let fake = Fake::new(Script::default());
    let work = tempfile::tempdir().unwrap();
    let mut seen = Vec::new();
    let run = fake
        .cli()
        .run_typed::<Out>(&request(Effort::Quality, None, work.path()), |e| seen.push(e.kind.clone()))
        .unwrap_or_else(|e| panic!("{e:#}"));
    assert_eq!(run.value, Out { text: "ok".into() });
    assert_eq!(seen, vec!["turn.completed".to_string()]);
    assert_eq!(run.events.len(), 1);
    assert_eq!(run.events[0].kind, "turn.completed");

    let as_value = fake
        .cli()
        .run_typed::<Value>(&request(Effort::Quality, None, work.path()), |_| {})
        .unwrap_or_else(|e| panic!("{e:#}"));
    assert_eq!(as_value.value, json!({"text":"ok"}));
}

#[test]
fn codex_output_that_misses_the_requested_type_is_an_error() {
    let fake = Fake::new(Script { last_message: r#"{"other":1}"#, ..Script::default() });
    let work = tempfile::tempdir().unwrap();
    let err = match fake.cli().run_typed::<Out>(&request(Effort::Quality, None, work.path()), |_| {}) {
        Ok(_) => panic!("expected an error"),
        Err(e) => e,
    };
    assert!(err.to_string().contains("did not match the schema"), "{err:#}");
}

#[test]
fn codex_not_logged_in_stops_before_exec() {
    let fake = Fake::new(Script { login_output: "Not logged in", login_exit: 1, ..Script::default() });
    let work = tempfile::tempdir().unwrap();
    let failure = run_failure(&fake, &request(Effort::Quality, None, work.path()));
    assert_eq!(failure.kind, FailureKind::NotLoggedIn);
    assert_eq!(failure.exit_code, None);
    assert_eq!(fake.calls(), vec!["login status".to_string()]);
    assert!(!fake.exists("exec-args.log"), "exec must not run when not logged in");
}

#[test]
fn codex_login_status_that_fails_for_another_reason_does_not_block_the_run() {
    let fake = Fake::new(Script { login_output: "error loading config", login_exit: 1, ..Script::default() });
    let work = tempfile::tempdir().unwrap();
    fake.cli()
        .run_typed::<Value>(&request(Effort::Quality, None, work.path()), |_| {})
        .unwrap_or_else(|e| panic!("{e:#}"));
    assert_eq!(fake.calls(), vec!["login status".to_string(), "exec".to_string()]);
}

#[test]
fn codex_logged_in_reports_true_false_and_unknown() {
    let work = tempfile::tempdir().unwrap();
    let _ = work;
    let yes = Fake::new(Script::default());
    assert_eq!(yes.cli().logged_in(), Some(true));
    let no = Fake::new(Script { login_output: "Not logged in", login_exit: 1, ..Script::default() });
    assert_eq!(no.cli().logged_in(), Some(false));
    let unknown = Fake::new(Script { login_output: "error loading config", login_exit: 1, ..Script::default() });
    assert_eq!(unknown.cli().logged_in(), None);
    let missing = CodexCli::at("/nonexistent/lita-test/codex");
    assert_eq!(missing.logged_in(), None);
}

#[test]
fn codex_turn_failed_401_is_classified_as_not_logged_in() {
    let fake = Fake::new(Script { exec_stdout: UNAUTHORIZED.to_string(), exec_exit: 1, ..Script::default() });
    let work = tempfile::tempdir().unwrap();
    let failure = run_failure(&fake, &request(Effort::Quality, None, work.path()));
    assert_eq!(failure.kind, FailureKind::NotLoggedIn);
    assert_eq!(failure.exit_code, Some(1));
    assert_eq!(
        failure.message.as_deref(),
        Some("unexpected status 401 Unauthorized: Missing bearer or basic authentication in header")
    );
    assert_eq!(fake.calls(), vec!["login status".to_string(), "exec".to_string()]);
}

#[test]
fn codex_turn_failed_usage_limit_is_classified_as_quota_exhausted() {
    let fake = Fake::new(Script { exec_stdout: USAGE_LIMIT.to_string(), exec_exit: 1, ..Script::default() });
    let work = tempfile::tempdir().unwrap();
    let failure = run_failure(&fake, &request(Effort::Quality, None, work.path()));
    assert_eq!(failure.kind, FailureKind::QuotaExhausted);
    assert_eq!(failure.exit_code, Some(1));
    assert_eq!(failure.message.as_deref(), Some("You've hit your usage limit."));
}

#[test]
fn codex_turn_failed_mentioning_the_schema_is_a_schema_mismatch() {
    let line = r#"{"type":"turn.failed","error":{"message":"the output does not match the schema"}}"#;
    let fake = Fake::new(Script { exec_stdout: line.to_string(), exec_exit: 1, ..Script::default() });
    let work = tempfile::tempdir().unwrap();
    let failure = run_failure(&fake, &request(Effort::Quality, None, work.path()));
    assert_eq!(failure.kind, FailureKind::SchemaMismatch);
}

#[test]
fn codex_unclassified_failure_is_unknown_and_keeps_the_exit_code() {
    let line = r#"{"type":"turn.failed","error":{"message":"something odd happened"}}"#;
    let fake = Fake::new(Script { exec_stdout: line.to_string(), exec_exit: 3, ..Script::default() });
    let work = tempfile::tempdir().unwrap();
    let failure = run_failure(&fake, &request(Effort::Quality, None, work.path()));
    assert_eq!(failure.kind, FailureKind::Unknown);
    assert_eq!(failure.exit_code, Some(3));
    assert_eq!(failure.message.as_deref(), Some("something odd happened"));
}

#[test]
fn codex_failure_without_a_turn_record_is_classified_from_stderr() {
    let fake = Fake::new(Script {
        exec_override: Some(
            "echo 'error: You have hit your usage limit.' >&2\n  exit 1",
        ),
        ..Script::default()
    });
    let work = tempfile::tempdir().unwrap();
    let failure = run_failure(&fake, &request(Effort::Quality, None, work.path()));
    assert_eq!(failure.kind, FailureKind::QuotaExhausted);
    assert_eq!(failure.message, None);
    assert!(failure.stderr_tail.contains("usage limit"), "{}", failure.stderr_tail);
}

#[test]
fn codex_cancel_kills_the_run_and_reports_cancelled() {
    let fake = Fake::new(Script {
        // `exec` replaces the shell, so killing the child kills the sleep too.
        exec_override: Some("exec sleep 30"),
        ..Script::default()
    });
    let work = tempfile::tempdir().unwrap();
    let cancel = Arc::new(AtomicBool::new(false));
    let setter = {
        let cancel = Arc::clone(&cancel);
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(400));
            cancel.store(true, Ordering::Relaxed);
        })
    };
    let started = Instant::now();
    let result = fake.cli().run_typed_cancellable::<Value>(
        &request(Effort::Quality, None, work.path()),
        |_| {},
        Arc::clone(&cancel),
    );
    setter.join().unwrap();
    let failure = match result {
        Ok(_) => panic!("a cancelled run must fail"),
        Err(e) => e.downcast::<RunFailure>().unwrap_or_else(|e| panic!("{e:#}")),
    };
    assert_eq!(failure.kind, FailureKind::Cancelled);
    assert!(started.elapsed() < Duration::from_secs(20), "cancel did not stop the child");
}

#[test]
fn codex_preflight_reads_the_doctor_report() {
    let doctor = |config: &str, auth: &str| {
        json!({
            "codexVersion": "0.156.0",
            "checks": {
                "config.load": {"status": config, "summary": "config loaded", "details": {"config.toml parse": "ok"}},
                "auth.credentials": {"status": auth}
            }
        })
        .to_string()
    };

    let ready = Fake::new(Script { doctor_json: doctor("ok", "ok"), ..Script::default() });
    assert_eq!(ready.cli().preflight().unwrap(), Preflight::Ready { version: "0.156.0".into() });
    assert_eq!(ready.calls(), vec!["doctor --json".to_string()]);

    let logged_out = Fake::new(Script { doctor_json: doctor("ok", "error"), ..Script::default() });
    assert_eq!(logged_out.cli().preflight().unwrap(), Preflight::NotLoggedIn { version: "0.156.0".into() });

    let broken = Fake::new(Script { doctor_json: doctor("error", "ok"), ..Script::default() });
    assert!(matches!(broken.cli().preflight().unwrap(), Preflight::ConfigBroken { .. }));
}

#[test]
fn codex_preflight_without_a_binary_is_not_installed() {
    let cli = CodexCli::at("/nonexistent/lita-test/codex");
    assert_eq!(cli.preflight().unwrap(), Preflight::NotInstalled);
}

#[test]
fn codex_preflight_with_non_json_output_is_an_error() {
    let fake = Fake::new(Script { doctor_json: "not json".to_string(), ..Script::default() });
    assert!(fake.cli().preflight().is_err());
}
