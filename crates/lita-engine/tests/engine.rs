//! Fake-CLI tests for `Engine` (Task #152, Feature #144 / Epic #143).
//!
//! Contract: `docs/spec/143-basic-design.md` chapter 4 and the detailed design
//! of Feature #144, section 2: `Engine::Codex` only ever starts `codex`,
//! `Engine::Claude` only ever starts `claude`, there is no path to the other
//! one (not even after a failure), and `RunFailure.engine` names the engine
//! that actually ran.
//!
//! A fake `codex` and a fake `claude` live in one temporary directory. Both
//! append "name [arg] [arg] ..." to the same `calls.log`, so the tests can
//! count who was started. Nothing starts a real CLI. The email address and
//! organisation name in the fake `auth status` output are made up.

#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::sync::atomic::AtomicBool;
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::Duration;

use lita_engine::{
    ClaudeCli, CodexCli, Effort, Engine, EngineKind, FailureKind, Preflight, Request, Run, RunFailure,
};
use serde::Deserialize;
use serde_json::{json, Value};
use tempfile::TempDir;

const SECRET_EMAIL: &str = "secret-user@example.invalid";
const SECRET_ORG: &str = "Secret Org Ltd";
const WATCHDOG_SECS: u64 = 30;

#[derive(Debug, Deserialize, PartialEq)]
struct Out {
    text: String,
}

fn schema() -> Value {
    json!({
        "type": "object",
        "properties": { "text": { "type": "string" } },
        "required": ["text"],
        "additionalProperties": false
    })
}

const CODEX_DOCTOR: &str = r#"{"codexVersion":"0.157.0","checks":{"config.load":{"status":"ok","details":{"config.toml parse":"ok"}},"auth.credentials":{"status":"ok"}}}"#;

// Same shape as a real `codex exec --json` failure (401 sample measured on
// codex-cli 0.155.1, see crates/lita-codex/src/lib.rs).
const CODEX_FAILED_EVENT: &str = r#"{"type":"turn.failed","error":{"message":"unexpected status 401 Unauthorized: Missing bearer or basic authentication in header"}}"#;

/// Fake `codex`. `login status` exits 0, `doctor --json` prints a healthy
/// report, `exec` writes the JSON after `--output-last-message` and prints a
/// JSONL stream (or, for the failing variant, a `turn.failed` line, exit 1).
const CODEX_SCRIPT: &str = r#"#!/bin/sh
D='@DIR@'
{ printf 'codex'; printf ' [%s]' "$@"; printf '\n'; } >> "$D/calls.log"
case "$1" in
  login)
    exit 0
    ;;
  doctor)
    cat "$D/doctor.json"
    exit 0
    ;;
  exec)
    out=""
    prev=""
    for a in "$@"; do
      if [ "$prev" = "--output-last-message" ]; then out="$a"; fi
      prev="$a"
    done
    cat > /dev/null
    if [ "$(cat "$D/codex.mode")" = "fail" ]; then
      cat "$D/codex.events"
      exit 1
    fi
    printf '%s' '{"text":"hi"}' > "$out"
    cat "$D/codex.events"
    exit 0
    ;;
esac
echo "unexpected invocation" >&2
exit 99
"#;

/// Fake `claude`: `--version`, `auth status --json` and `-p` (generation).
const CLAUDE_SCRIPT: &str = r#"#!/bin/sh
D='@DIR@'
{ printf 'claude'; printf ' [%s]' "$@"; printf '\n'; } >> "$D/calls.log"
case "$1" in
  --version)
    echo '2.1.296 (Claude Code)'
    exit 0
    ;;
  auth)
    cat "$D/claude.auth.out"
    exit "$(cat "$D/claude.auth.exit")"
    ;;
  -p)
    cat > /dev/null
    cat "$D/claude.gen.err" >&2
    cat "$D/claude.gen.out"
    exit "$(cat "$D/claude.gen.exit")"
    ;;
esac
echo "unexpected invocation" >&2
exit 99
"#;

struct Spec {
    codex_fails: bool,
    claude_logged_in: bool,
    claude_gen_out: String,
    claude_gen_exit: i32,
    claude_gen_err: String,
}

impl Spec {
    fn new() -> Self {
        Spec {
            codex_fails: false,
            claude_logged_in: true,
            claude_gen_out: json!({
                "type": "result", "subtype": "success", "is_error": false,
                "result": "done", "structured_output": { "text": "hi" }
            })
            .to_string(),
            claude_gen_exit: 0,
            claude_gen_err: String::new(),
        }
    }

    fn codex_fails(mut self) -> Self {
        self.codex_fails = true;
        self
    }

    fn claude_logged_out(mut self) -> Self {
        self.claude_logged_in = false;
        self
    }

    /// A generation that fails in a way no rule classifies: `Unknown`.
    fn claude_unknown_failure(mut self) -> Self {
        self.claude_gen_out = json!({
            "type": "result", "subtype": "success", "is_error": true, "result": "something odd"
        })
        .to_string();
        self.claude_gen_exit = 1;
        self.claude_gen_err = "boom".to_string();
        self
    }

    fn build(self) -> Fake {
        let dir = tempfile::tempdir().expect("tempdir for the fake CLIs");
        let wd = tempfile::tempdir().expect("tempdir for the working directory");
        let d = dir.path();
        let dir_str = d.to_str().expect("utf-8 temp path");

        fs::write(d.join("doctor.json"), CODEX_DOCTOR).unwrap();
        fs::write(d.join("codex.mode"), if self.codex_fails { "fail" } else { "ok" }).unwrap();
        let events = if self.codex_fails {
            format!("{CODEX_FAILED_EVENT}\n")
        } else {
            "{\"type\":\"thread.started\",\"thread_id\":\"t\"}\n".to_string()
        };
        fs::write(d.join("codex.events"), events).unwrap();

        let auth = if self.claude_logged_in {
            json!({ "loggedIn": true, "authMethod": "claude.ai", "email": SECRET_EMAIL, "orgName": SECRET_ORG })
        } else {
            json!({ "loggedIn": false, "email": SECRET_EMAIL, "orgName": SECRET_ORG })
        };
        fs::write(d.join("claude.auth.out"), auth.to_string()).unwrap();
        fs::write(d.join("claude.auth.exit"), if self.claude_logged_in { "0" } else { "1" }).unwrap();
        fs::write(d.join("claude.gen.out"), &self.claude_gen_out).unwrap();
        fs::write(d.join("claude.gen.exit"), self.claude_gen_exit.to_string()).unwrap();
        fs::write(d.join("claude.gen.err"), &self.claude_gen_err).unwrap();

        let codex = d.join("codex");
        let claude = d.join("claude");
        fs::write(&codex, CODEX_SCRIPT.replace("@DIR@", dir_str)).unwrap();
        fs::write(&claude, CLAUDE_SCRIPT.replace("@DIR@", dir_str)).unwrap();
        for p in [&codex, &claude] {
            fs::set_permissions(p, fs::Permissions::from_mode(0o755)).unwrap();
        }
        Fake { dir, wd, codex, claude }
    }
}

struct Fake {
    dir: TempDir,
    wd: TempDir,
    codex: std::path::PathBuf,
    claude: std::path::PathBuf,
}

impl Fake {
    fn codex_engine(&self) -> Engine {
        Engine::Codex(CodexCli::at(self.codex.clone()))
    }

    fn claude_engine(&self) -> Engine {
        Engine::Claude(ClaudeCli::at(self.claude.clone()))
    }

    fn log_lines(&self) -> Vec<String> {
        fs::read_to_string(self.dir.path().join("calls.log"))
            .map(|t| t.lines().map(str::to_string).collect())
            .unwrap_or_default()
    }

    /// How many times `name` ("codex" or "claude") was started.
    fn started(&self, name: &str) -> usize {
        self.log_lines()
            .iter()
            .filter(|l| *l == name || l.starts_with(&format!("{name} ")))
            .count()
    }

    fn request(&self) -> Request {
        Request {
            prompt: "write something".to_string(),
            schema: schema(),
            model: None,
            effort: Effort::Quality,
            working_dir: self.wd.path().to_path_buf(),
        }
    }

    fn assert_only(&self, name: &str, other: &str) {
        assert!(self.started(name) >= 1, "{name} should have been started: {:?}", self.log_lines());
        assert_eq!(
            self.started(other),
            0,
            "{other} must not be started by the {name} engine: {:?}",
            self.log_lines()
        );
    }
}

/// Runs `f` on its own thread and fails the test if it takes too long.
fn within<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let _ = tx.send(f());
    });
    rx.recv_timeout(Duration::from_secs(WATCHDOG_SECS))
        .unwrap_or_else(|_| panic!("timed out after {WATCHDOG_SECS}s; the call did not return"))
}

fn run(engine: Engine, req: Request) -> anyhow::Result<Run<Out>> {
    within(move || engine.run_typed::<Out>(&req, |_| {}))
}

fn preflight(engine: Engine) -> anyhow::Result<Preflight> {
    within(move || engine.preflight())
}

fn failure_of(result: anyhow::Result<Run<Out>>) -> anyhow::Error {
    match result {
        Ok(r) => panic!("expected a failure but the run succeeded with {:?}", r.value),
        Err(e) => e,
    }
}

fn rf(e: &anyhow::Error) -> &RunFailure {
    e.downcast_ref::<RunFailure>()
        .unwrap_or_else(|| panic!("the error is not a RunFailure: {e:#}"))
}

// ---------------------------------------------------------------------------
// 1-2. A successful run starts only its own CLI
// ---------------------------------------------------------------------------

#[test]
fn engine_codex_run_succeeds_and_never_starts_claude() {
    let fake = Spec::new().build();
    let r = run(fake.codex_engine(), fake.request()).expect("codex run");
    assert_eq!(r.value, Out { text: "hi".to_string() });
    fake.assert_only("codex", "claude");
}

#[test]
fn engine_claude_run_succeeds_and_never_starts_codex() {
    let fake = Spec::new().build();
    let r = run(fake.claude_engine(), fake.request()).expect("claude run");
    assert_eq!(r.value, Out { text: "hi".to_string() });
    fake.assert_only("claude", "codex");
}

#[test]
fn engine_run_typed_cancellable_also_uses_only_its_own_cli() {
    for claude in [false, true] {
        let fake = Spec::new().build();
        let engine = if claude { fake.claude_engine() } else { fake.codex_engine() };
        let req = fake.request();
        let r = within(move || engine.run_typed_cancellable::<Out>(&req, |_| {}, Arc::new(AtomicBool::new(false))))
            .expect("run");
        assert_eq!(r.value, Out { text: "hi".to_string() });
        if claude {
            fake.assert_only("claude", "codex");
        } else {
            fake.assert_only("codex", "claude");
        }
    }
}

// ---------------------------------------------------------------------------
// 3-4. A failure names the engine that ran and does not fall back
// ---------------------------------------------------------------------------

#[test]
fn engine_codex_failure_reports_codex_and_never_starts_claude() {
    let fake = Spec::new().codex_fails().build();
    let e = failure_of(run(fake.codex_engine(), fake.request()));
    let f = rf(&e);
    assert_eq!(f.engine, EngineKind::Codex, "{e:#}");
    assert_eq!(f.kind, FailureKind::NotLoggedIn, "a turn.failed 401 is NotLoggedIn: {e:#}");
    assert_eq!(f.exit_code, Some(1));
    fake.assert_only("codex", "claude");
}

#[test]
fn engine_claude_failure_reports_claude_and_never_starts_codex() {
    let fake = Spec::new().claude_unknown_failure().build();
    let e = failure_of(run(fake.claude_engine(), fake.request()));
    let f = rf(&e);
    assert_eq!(f.engine, EngineKind::Claude, "{e:#}");
    assert_eq!(f.kind, FailureKind::Unknown, "{e:#}");
    fake.assert_only("claude", "codex");
}

#[test]
fn engine_claude_not_logged_in_reports_claude_and_never_starts_codex() {
    let fake = Spec::new().claude_logged_out().build();
    let e = failure_of(run(fake.claude_engine(), fake.request()));
    let f = rf(&e);
    assert_eq!(f.engine, EngineKind::Claude, "{e:#}");
    assert_eq!(f.kind, FailureKind::NotLoggedIn, "{e:#}");
    fake.assert_only("claude", "codex");
    // Logged out means the generation itself was never started.
    assert!(
        fake.log_lines().iter().all(|l| !l.starts_with("claude [-p]")),
        "the generation must not run when logged out: {:?}",
        fake.log_lines()
    );
}

// ---------------------------------------------------------------------------
// 5. preflight also touches only its own CLI
// ---------------------------------------------------------------------------

#[test]
fn engine_codex_preflight_calls_only_codex() {
    let fake = Spec::new().build();
    let p = preflight(fake.codex_engine()).expect("codex preflight");
    assert_eq!(p, Preflight::Ready { version: "0.157.0".to_string() });
    fake.assert_only("codex", "claude");
}

#[test]
fn engine_claude_preflight_calls_only_claude() {
    let fake = Spec::new().build();
    let p = preflight(fake.claude_engine()).expect("claude preflight");
    assert_eq!(p, Preflight::Ready { version: "2.1.296".to_string() });
    fake.assert_only("claude", "codex");
}

#[test]
fn engine_claude_preflight_logged_out_calls_only_claude() {
    let fake = Spec::new().claude_logged_out().build();
    let p = preflight(fake.claude_engine()).expect("claude preflight");
    assert_eq!(p, Preflight::NotLoggedIn { version: "2.1.296".to_string() });
    fake.assert_only("claude", "codex");
}

// ---------------------------------------------------------------------------
// 6. kind()
// ---------------------------------------------------------------------------

#[test]
fn engine_kind_matches_the_variant() {
    let dir = tempfile::tempdir().unwrap();
    let codex = Engine::Codex(CodexCli::at(dir.path().join("codex")));
    let claude = Engine::Claude(ClaudeCli::at(dir.path().join("claude")));
    assert_eq!(codex.kind(), EngineKind::Codex);
    assert_eq!(claude.kind(), EngineKind::Claude);
}

#[test]
fn engine_on_path_kind_is_the_requested_kind_without_starting_anything() {
    // `on_path` only builds the value; reading `kind()` must not run a CLI.
    assert_eq!(Engine::on_path(EngineKind::Claude).kind(), EngineKind::Claude);
    assert_eq!(Engine::on_path(EngineKind::Codex).kind(), EngineKind::Codex);
}

#[test]
fn engine_kind_default_is_codex_and_display_names_are_fixed() {
    assert_eq!(EngineKind::default(), EngineKind::Codex);
    assert_eq!(EngineKind::Codex.display_name(), "Codex");
    assert_eq!(EngineKind::Claude.display_name(), "Claude Code");
}
