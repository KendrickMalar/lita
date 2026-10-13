//! Fake-CLI tests for `ClaudeCli` (Task #151, Feature #144 / Epic #143).
//!
//! The contract is `docs/spec/143-basic-design.md` chapter 3 (arguments, how
//! the state is read, how failures are classified) plus B9 (cancelling kills
//! the whole process group). Nothing here starts a real `claude`: every test
//! writes its own `#!/bin/sh` script into a temporary directory and points
//! `ClaudeCli::at` at it. The script records how it was called into files
//! inside that directory, so no environment variable is touched and the tests
//! can run in parallel.
//!
//! The email address and organisation name used in the fake `auth status`
//! output are made up (`.invalid` is a reserved top-level domain).
//!
//! The failure samples for "usage limit" and "schema mismatch" are
//! **fictional**: they could not be reproduced against a real claude, and are
//! only shaped after the words listed in design section 3.3. The "not logged
//! in" sample is the one measured on 2026-10-13 (claude 2.1.296).

#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::{Duration, Instant};

use lita_engine::{ClaudeCli, Effort, EngineKind, FailureKind, Preflight, Request, Run, RunFailure};
use serde::Deserialize;
use serde_json::{json, Value};
use tempfile::TempDir;

const SECRET_EMAIL: &str = "secret-user@example.invalid";
const SECRET_ORG: &str = "Secret Org Ltd";

/// Upper bound for a whole run of a fake claude. A hang is a failure, not a
/// stuck test suite.
const WATCHDOG_SECS: u64 = 30;

const VERSION_OUTPUT: &str = "2.1.296 (Claude Code)\n";

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

fn logged_in_json() -> String {
    json!({
        "loggedIn": true,
        "authMethod": "claude.ai",
        "email": SECRET_EMAIL,
        "orgName": SECRET_ORG
    })
    .to_string()
}

fn logged_out_json() -> String {
    json!({ "loggedIn": false, "email": SECRET_EMAIL, "orgName": SECRET_ORG }).to_string()
}

fn success_json() -> String {
    json!({
        "type": "result",
        "subtype": "success",
        "is_error": false,
        "result": "done",
        "structured_output": { "text": "hi" }
    })
    .to_string()
}

// ---------------------------------------------------------------------------
// The fake claude
// ---------------------------------------------------------------------------

/// Every invocation is appended to `calls.log` as a `--- call` line followed
/// by one `[arg]` line per argument. The generation branch (`-p`) is filled in
/// per test.
const SCRIPT: &str = r#"#!/bin/sh
D='@DIR@'
{ echo '--- call'; printf '[%s]\n' "$@"; } >> "$D/calls.log"
case "$1" in
  --version)
    cat "$D/version.txt"
    exit 0
    ;;
  auth)
    cat "$D/auth.err" >&2
    cat "$D/auth.out"
    exit "$(cat "$D/auth.exit")"
    ;;
  -p)
@GEN@
    ;;
esac
echo "unexpected invocation" >&2
exit 99
"#;

/// The default generation: remember the arguments, stdin and cwd, then answer
/// with the canned output.
const DEFAULT_GEN: &str = r#"    printf '[%s]\n' "$@" > "$D/gen_args.txt"
    cat > "$D/gen_stdin.txt"
    pwd -P > "$D/gen_pwd.txt"
    cat "$D/gen.err" >&2
    cat "$D/gen.out"
    exit "$(cat "$D/gen.exit")"
"#;

struct Spec {
    version: String,
    auth_out: String,
    auth_err: String,
    auth_exit: i32,
    gen_out: String,
    gen_err: String,
    gen_exit: i32,
    gen_script: Option<String>,
}

impl Spec {
    fn new() -> Self {
        Spec {
            version: VERSION_OUTPUT.to_string(),
            auth_out: logged_in_json(),
            auth_err: String::new(),
            auth_exit: 0,
            gen_out: success_json(),
            gen_err: String::new(),
            gen_exit: 0,
            gen_script: None,
        }
    }

    fn auth(mut self, out: impl Into<String>, exit: i32) -> Self {
        self.auth_out = out.into();
        self.auth_exit = exit;
        self
    }

    fn auth_stderr(mut self, err: impl Into<String>) -> Self {
        self.auth_err = err.into();
        self
    }

    fn generation(mut self, out: impl Into<String>, exit: i32, err: impl Into<String>) -> Self {
        self.gen_out = out.into();
        self.gen_exit = exit;
        self.gen_err = err.into();
        self
    }

    fn script(mut self, body: &str) -> Self {
        self.gen_script = Some(body.to_string());
        self
    }

    fn build(self) -> Fake {
        let dir = tempfile::tempdir().expect("tempdir for the fake claude");
        let wd = tempfile::tempdir().expect("tempdir for the working directory");
        let d = dir.path();
        fs::write(d.join("version.txt"), &self.version).unwrap();
        fs::write(d.join("auth.out"), &self.auth_out).unwrap();
        fs::write(d.join("auth.err"), &self.auth_err).unwrap();
        fs::write(d.join("auth.exit"), self.auth_exit.to_string()).unwrap();
        fs::write(d.join("gen.out"), &self.gen_out).unwrap();
        fs::write(d.join("gen.err"), &self.gen_err).unwrap();
        fs::write(d.join("gen.exit"), self.gen_exit.to_string()).unwrap();
        let generation = self.gen_script.as_deref().unwrap_or(DEFAULT_GEN);
        let script = SCRIPT
            .replace("@DIR@", d.to_str().expect("utf-8 temp path"))
            .replace("@GEN@", generation);
        let claude = d.join("claude");
        fs::write(&claude, script).unwrap();
        fs::set_permissions(&claude, fs::Permissions::from_mode(0o755)).unwrap();
        Fake { dir, wd, claude }
    }
}

struct Fake {
    dir: TempDir,
    wd: TempDir,
    claude: PathBuf,
}

fn parse_args(text: &str) -> Vec<String> {
    text.lines()
        .map(|l| {
            l.strip_prefix('[')
                .and_then(|l| l.strip_suffix(']'))
                .unwrap_or(l)
                .to_string()
        })
        .collect()
}

impl Fake {
    fn read(&self, name: &str) -> Option<String> {
        fs::read_to_string(self.dir.path().join(name)).ok()
    }

    /// Every invocation, each as its argument list.
    fn calls(&self) -> Vec<Vec<String>> {
        let Some(text) = self.read("calls.log") else {
            return Vec::new();
        };
        let mut calls: Vec<Vec<String>> = Vec::new();
        for line in text.lines() {
            if line == "--- call" {
                calls.push(Vec::new());
            } else if let Some(last) = calls.last_mut() {
                last.extend(parse_args(line));
            }
        }
        calls
    }

    fn generation_calls(&self) -> Vec<Vec<String>> {
        self.calls()
            .into_iter()
            .filter(|c| c.first().map(String::as_str) == Some("-p"))
            .collect()
    }

    fn gen_args(&self) -> Vec<String> {
        parse_args(&self.read("gen_args.txt").expect("the generation was never called"))
    }

    fn request(&self, effort: Effort, model: Option<&str>, prompt: &str) -> Request {
        Request {
            prompt: prompt.to_string(),
            schema: schema(),
            model: model.map(str::to_string),
            effort,
            working_dir: self.wd.path().to_path_buf(),
        }
    }
}

/// Runs `f` on its own thread and fails the test if it takes too long.
fn within<T: Send + 'static>(secs: u64, f: impl FnOnce() -> T + Send + 'static) -> T {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let _ = tx.send(f());
    });
    rx.recv_timeout(Duration::from_secs(secs))
        .unwrap_or_else(|_| panic!("timed out after {secs}s; the call did not return"))
}

fn run_with(fake: &Fake, effort: Effort, model: Option<&str>, prompt: &str) -> anyhow::Result<Run<Out>> {
    let program = fake.claude.clone();
    let req = fake.request(effort, model, prompt);
    within(WATCHDOG_SECS, move || {
        ClaudeCli::at(program).run_typed::<Out>(&req, |_| {})
    })
}

fn run(fake: &Fake) -> anyhow::Result<Run<Out>> {
    run_with(fake, Effort::Quality, None, "write something")
}

fn preflight(fake: &Fake) -> anyhow::Result<Preflight> {
    let program = fake.claude.clone();
    within(WATCHDOG_SECS, move || ClaudeCli::at(program).preflight())
}

/// Builds the fake, runs it once, and returns the error (panics on success).
fn failure_of(spec: Spec) -> anyhow::Error {
    let fake = spec.build();
    match run(&fake) {
        Ok(r) => panic!("expected a failure but the run succeeded with {:?}", r.value),
        Err(e) => e,
    }
}

fn rf(e: &anyhow::Error) -> &RunFailure {
    e.downcast_ref::<RunFailure>()
        .unwrap_or_else(|| panic!("the error is not a RunFailure: {e:#}"))
}

/// The failure must be a `RunFailure` from the Claude engine of this kind.
fn assert_kind(spec: Spec, kind: FailureKind) -> anyhow::Error {
    let e = failure_of(spec);
    let f = rf(&e);
    assert_eq!(f.kind, kind, "wrong kind: {e:#}");
    assert_eq!(f.engine, EngineKind::Claude, "wrong engine: {e:#}");
    e
}

fn assert_no_secrets(text: &str, raw_fragments: &[&str]) {
    for needle in [SECRET_EMAIL, SECRET_ORG].iter().chain(raw_fragments.iter()) {
        assert!(!text.contains(needle), "leaked {needle:?} into: {text}");
    }
}

fn assert_error_has_no_secrets(e: &anyhow::Error, raw_fragments: &[&str]) {
    assert_no_secrets(&format!("{e:#}"), raw_fragments);
    assert_no_secrets(&format!("{e:?}"), raw_fragments);
}

fn expected_args(schema: &Value, effort: &str) -> Vec<String> {
    [
        "-p",
        "--output-format",
        "json",
        "--json-schema",
        &serde_json::to_string(schema).unwrap(),
        "--tools",
        "",
        "--setting-sources",
        "",
        "--strict-mcp-config",
        "--disable-slash-commands",
        "--no-session-persistence",
        "--effort",
        effort,
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

// ---------------------------------------------------------------------------
// 1. Arguments (REQ004, design 3.1)
// ---------------------------------------------------------------------------

#[test]
fn claude_args_are_exactly_the_designed_list_for_quality_without_model() {
    let fake = Spec::new().build();
    run(&fake).expect("run");
    assert_eq!(fake.gen_args(), expected_args(&schema(), "medium"));
}

#[test]
fn claude_prompt_goes_to_stdin_and_cwd_is_the_working_dir() {
    let fake = Spec::new().build();
    let prompt = "line one\nline two with 日本語 and \"quotes\" and $dollar `tick`\n";
    within(WATCHDOG_SECS, {
        let program = fake.claude.clone();
        let req = fake.request(Effort::Quality, None, prompt);
        move || ClaudeCli::at(program).run_typed::<Out>(&req, |_| {})
    })
    .expect("run");
    assert_eq!(fake.read("gen_stdin.txt").expect("stdin was recorded"), prompt);
    // The prompt must not also be passed as an argument.
    assert!(fake.gen_args().iter().all(|a| !a.contains("line one")));
    let cwd = fake.read("gen_pwd.txt").expect("cwd was recorded");
    assert_eq!(
        fs::canonicalize(cwd.trim()).unwrap(),
        fs::canonicalize(fake.wd.path()).unwrap()
    );
}

#[test]
fn claude_effort_fast_is_low_and_best_is_high() {
    for (effort, word) in [(Effort::Fast, "low"), (Effort::Best, "high")] {
        let fake = Spec::new().build();
        run_with(&fake, effort, None, "p").expect("run");
        assert_eq!(fake.gen_args(), expected_args(&schema(), word), "effort {word}");
    }
}

#[test]
fn claude_model_flag_is_added_at_the_end_only_when_requested() {
    let with = Spec::new().build();
    run_with(&with, Effort::Quality, Some("x"), "p").expect("run");
    let mut expected = expected_args(&schema(), "medium");
    expected.extend(["--model".to_string(), "x".to_string()]);
    assert_eq!(with.gen_args(), expected);

    let without = Spec::new().build();
    run_with(&without, Effort::Quality, None, "p").expect("run");
    assert!(
        !without.gen_args().iter().any(|a| a == "--model"),
        "--model must not be passed when Request.model is None"
    );
}

#[test]
fn claude_run_asks_auth_status_before_generating() {
    let fake = Spec::new().build();
    run(&fake).expect("run");
    let calls = fake.calls();
    let auth = calls
        .iter()
        .position(|c| c.as_slice() == ["auth", "status", "--json"])
        .expect("`claude auth status --json` was never called before generating");
    let generation = calls
        .iter()
        .position(|c| c.first().map(String::as_str) == Some("-p"))
        .expect("the generation was never called");
    assert!(auth < generation, "auth status must come before the generation: {calls:?}");
}

// ---------------------------------------------------------------------------
// 2. Success
// ---------------------------------------------------------------------------

#[test]
fn claude_success_returns_the_structured_output() {
    let fake = Spec::new()
        .generation(
            r#"{"type":"result","subtype":"success","is_error":false,"result":"done","structured_output":{"text":"hi"}}"#,
            0,
            "",
        )
        .build();
    let r = run(&fake).expect("run");
    assert_eq!(r.value, Out { text: "hi".to_string() });
    assert!(r.events.is_empty(), "Claude Code's Run.events may stay empty");
}

// ---------------------------------------------------------------------------
// 3. Preflight (design 3.2)
// ---------------------------------------------------------------------------

#[test]
fn claude_preflight_not_installed_for_a_missing_binary() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("no-such-claude");
    let p = ClaudeCli::at(missing).preflight().expect("preflight");
    assert_eq!(p, Preflight::NotInstalled);
}

#[test]
fn claude_preflight_ready_reads_the_first_word_as_the_version() {
    let fake = Spec::new().build();
    let p = preflight(&fake).expect("preflight");
    assert_eq!(p, Preflight::Ready { version: "2.1.296".to_string() });
    let calls = fake.calls();
    assert!(calls.iter().any(|c| c.as_slice() == ["--version"]), "{calls:?}");
    assert!(
        calls.iter().any(|c| c.as_slice() == ["auth", "status", "--json"]),
        "{calls:?}"
    );
    assert!(fake.generation_calls().is_empty(), "preflight must not generate");
}

#[test]
fn claude_preflight_not_logged_in_when_logged_in_is_false() {
    let fake = Spec::new().auth(logged_out_json(), 0).build();
    let p = preflight(&fake).expect("preflight");
    assert_eq!(p, Preflight::NotLoggedIn { version: "2.1.296".to_string() });
}

#[test]
fn claude_preflight_not_logged_in_with_exit_code_1() {
    // `claude auth status` exits 1 when logged out while still printing JSON.
    let fake = Spec::new().auth(r#"{"loggedIn":false}"#, 1).build();
    let p = preflight(&fake).expect("preflight");
    assert_eq!(p, Preflight::NotLoggedIn { version: "2.1.296".to_string() });
}

#[test]
fn claude_preflight_errors_on_non_json_without_leaking_the_output() {
    let raw = "totally garbled status line";
    let fake = Spec::new()
        .auth(format!("{raw} for {SECRET_EMAIL} at {SECRET_ORG}"), 0)
        .auth_stderr(format!("warning about {SECRET_EMAIL} ({SECRET_ORG})"))
        .build();
    let e = match preflight(&fake) {
        Ok(p) => panic!("expected an error but got {p:?}"),
        Err(e) => e,
    };
    assert_error_has_no_secrets(&e, &[raw, "warning about"]);
}

#[test]
fn claude_preflight_errors_when_logged_in_key_is_missing_without_leaking() {
    let raw = json!({ "authMethod": "claude.ai", "email": SECRET_EMAIL, "orgName": SECRET_ORG }).to_string();
    let fake = Spec::new().auth(raw.clone(), 0).build();
    let e = match preflight(&fake) {
        Ok(p) => panic!("expected an error but got {p:?}"),
        Err(e) => e,
    };
    assert_error_has_no_secrets(&e, &[&raw, "authMethod", "claude.ai"]);
}

// ---------------------------------------------------------------------------
// 4. Not logged in before generating
// ---------------------------------------------------------------------------

#[test]
fn claude_run_stops_before_generating_when_logged_out() {
    let fake = Spec::new().auth(logged_out_json(), 1).build();
    let e = match run(&fake) {
        Ok(r) => panic!("expected a failure but the run succeeded with {:?}", r.value),
        Err(e) => e,
    };
    let f = rf(&e);
    assert_eq!(f.engine, EngineKind::Claude);
    assert_eq!(f.kind, FailureKind::NotLoggedIn);
    assert!(
        fake.generation_calls().is_empty(),
        "the generation (-p) must not run when logged out: {:?}",
        fake.calls()
    );
    assert!(
        fake.calls().iter().any(|c| c.as_slice() == ["auth", "status", "--json"]),
        "auth status must have been consulted"
    );
    assert_error_has_no_secrets(&e, &[]);
    assert_no_secrets(&format!("{f:?}"), &[]);
    assert_no_secrets(f.message.as_deref().unwrap_or(""), &[]);
    assert_no_secrets(&f.stderr_tail, &[]);
}

// ---------------------------------------------------------------------------
// 5. Failure classification (design 3.3). auth says logged in.
// ---------------------------------------------------------------------------

#[test]
fn claude_failure_measured_not_logged_in_sample() {
    // Measured 2026-10-13 against claude 2.1.296 with an empty CLAUDE_CONFIG_DIR.
    let out = r#"{"type":"result","subtype":"success","is_error":true,"terminal_reason":"api_error","result":"Not logged in · Please run /login"}"#;
    assert_kind(Spec::new().generation(out, 1, ""), FailureKind::NotLoggedIn);
}

#[test]
fn claude_failure_status_401_alone_is_not_logged_in() {
    let out = json!({
        "type": "result", "subtype": "success", "is_error": true,
        "api_error_status": 401, "result": "request failed"
    })
    .to_string();
    assert_kind(Spec::new().generation(out, 1, ""), FailureKind::NotLoggedIn);
}

#[test]
fn claude_failure_status_429_is_quota_exhausted_fictional_sample() {
    // Fictional sample: a real usage-limit failure could not be reproduced.
    let out = json!({
        "type": "result", "subtype": "success", "is_error": true,
        "api_error_status": 429, "result": "request failed"
    })
    .to_string();
    assert_kind(Spec::new().generation(out, 1, ""), FailureKind::QuotaExhausted);
}

#[test]
fn claude_failure_hit_your_limit_text_is_quota_exhausted_fictional_sample() {
    // Fictional sample, worded after design 3.3 ("hit your limit").
    let out = json!({
        "type": "result", "subtype": "success", "is_error": true,
        "result": "You've hit your limit · resets 5pm"
    })
    .to_string();
    assert_kind(Spec::new().generation(out, 1, ""), FailureKind::QuotaExhausted);
}

#[test]
fn claude_failure_is_error_true_counts_even_when_the_exit_code_is_zero() {
    // Design 3.3: a failure is "exit code != 0" OR "is_error: true".
    let out = json!({
        "type": "result", "subtype": "success", "is_error": true,
        "api_error_status": 429, "result": "request failed"
    })
    .to_string();
    assert_kind(Spec::new().generation(out, 0, ""), FailureKind::QuotaExhausted);
}

#[test]
fn claude_failure_max_structured_output_retries_is_schema_mismatch_fictional_sample() {
    // Fictional sample, shaped after the subtype named in design 3.3.
    let out = json!({
        "type": "result", "subtype": "error_max_structured_output_retries", "is_error": true
    })
    .to_string();
    assert_kind(Spec::new().generation(out, 1, ""), FailureKind::SchemaMismatch);
}

#[test]
fn claude_failure_success_without_structured_output_is_schema_mismatch() {
    let out = json!({
        "type": "result", "subtype": "success", "is_error": false, "result": "done"
    })
    .to_string();
    assert_kind(Spec::new().generation(out, 0, ""), FailureKind::SchemaMismatch);
}

#[test]
fn claude_failure_structured_output_of_the_wrong_type_is_schema_mismatch() {
    let out = json!({
        "type": "result", "subtype": "success", "is_error": false,
        "result": "done", "structured_output": { "text": 123 }
    })
    .to_string();
    assert_kind(Spec::new().generation(out, 0, ""), FailureKind::SchemaMismatch);
}

#[test]
fn claude_failure_unclassifiable_is_unknown_with_message_and_stderr_tail() {
    let out = json!({
        "type": "result", "subtype": "success", "is_error": true, "result": "something odd"
    })
    .to_string();
    let e = assert_kind(Spec::new().generation(out, 1, "boom"), FailureKind::Unknown);
    let f = rf(&e);
    assert_eq!(f.exit_code, Some(1));
    assert!(
        f.message.as_deref().unwrap_or("").contains("something odd"),
        "message should carry the result text: {:?}",
        f.message
    );
    assert!(f.stderr_tail.contains("boom"), "stderr_tail: {:?}", f.stderr_tail);
}

#[test]
fn claude_failure_with_unparseable_stdout_is_unknown_and_keeps_stderr() {
    let e = assert_kind(Spec::new().generation("not json at all", 3, "crash dump"), FailureKind::Unknown);
    let f = rf(&e);
    assert_eq!(f.exit_code, Some(3));
    assert!(f.stderr_tail.contains("crash dump"), "stderr_tail: {:?}", f.stderr_tail);
}

/// A failed result whose `result` text is `text` (and nothing else that could
/// classify it), run through the fake with the given exit code.
fn kind_of_result_text(text: &str, exit: i32) -> FailureKind {
    let out = json!({
        "type": "result", "subtype": "success", "is_error": true, "result": text
    })
    .to_string();
    let e = failure_of(Spec::new().generation(out, exit, ""));
    let f = rf(&e);
    assert_eq!(f.engine, EngineKind::Claude, "wrong engine for {text:?}: {e:#}");
    f.kind
}

/// Every judging word of design 3.3, one at a time, through a real run (AC004
/// "failure classification"). The texts are fictional except the measured one.
#[test]
fn claude_failure_every_not_logged_in_word_is_not_logged_in() {
    for text in [
        "Not logged in · Please run /login",
        "please run /login to continue",
        "API Error: 401 {\"type\":\"error\"}",
        "Unauthorized",
        "Invalid API key provided",
        "OAuth token has expired",
        "NOT LOGGED IN",
    ] {
        assert_eq!(kind_of_result_text(text, 1), FailureKind::NotLoggedIn, "{text:?}");
    }
}

#[test]
fn claude_failure_every_quota_word_is_quota_exhausted() {
    for text in [
        "You have reached your usage limit",
        "5-hour limit reached",
        "You've hit your limit",
        "rate limit exceeded",
        "Quota exceeded",
        "RATE LIMIT",
    ] {
        assert_eq!(kind_of_result_text(text, 1), FailureKind::QuotaExhausted, "{text:?}");
    }
}

#[test]
fn claude_failure_every_schema_word_is_schema_mismatch() {
    for text in [
        "Failed to produce structured output",
        "output does not match the schema",
        "STRUCTURED OUTPUT was rejected",
    ] {
        assert_eq!(kind_of_result_text(text, 1), FailureKind::SchemaMismatch, "{text:?}");
    }
    // The subtype alone is enough, with no text at all.
    let out = json!({
        "type": "result", "subtype": "error_max_structured_output_retries", "is_error": true,
        "result": "gave up"
    })
    .to_string();
    assert_kind(Spec::new().generation(out, 1, ""), FailureKind::SchemaMismatch);
}

/// A failure without any judging word stays `Unknown` whatever the exit code.
#[test]
fn claude_failure_without_a_judging_word_is_unknown_for_any_exit_code() {
    for exit in [1, 2, 137] {
        assert_eq!(kind_of_result_text("the server fell over", exit), FailureKind::Unknown, "exit {exit}");
    }
}

/// Design 3.3 says "first match wins, top to bottom": NotLoggedIn, then
/// QuotaExhausted, then SchemaMismatch.
#[test]
fn claude_failure_order_not_logged_in_then_quota_then_schema() {
    // 401 status beats a quota word in the text.
    let out = json!({
        "type": "result", "subtype": "success", "is_error": true,
        "api_error_status": 401, "result": "usage limit"
    })
    .to_string();
    assert_kind(Spec::new().generation(out, 1, ""), FailureKind::NotLoggedIn);

    // A login word beats a schema word.
    assert_eq!(kind_of_result_text("Not logged in, schema unknown", 1), FailureKind::NotLoggedIn);

    // 429 status beats a schema subtype.
    let out = json!({
        "type": "result", "subtype": "error_max_structured_output_retries", "is_error": true,
        "api_error_status": 429, "result": "x"
    })
    .to_string();
    assert_kind(Spec::new().generation(out, 1, ""), FailureKind::QuotaExhausted);

    // A quota word beats a schema word.
    assert_eq!(kind_of_result_text("quota exceeded while checking the schema", 1), FailureKind::QuotaExhausted);
}

/// A classified failure still reports the Claude engine and does not carry
/// anything from `auth status` (email, organisation).
#[test]
fn claude_failure_classified_run_keeps_message_and_leaks_nothing() {
    let out = json!({
        "type": "result", "subtype": "success", "is_error": true,
        "api_error_status": 429, "result": "You've hit your limit"
    })
    .to_string();
    let e = assert_kind(Spec::new().generation(out, 1, "stderr line"), FailureKind::QuotaExhausted);
    let f = rf(&e);
    assert_eq!(f.exit_code, Some(1));
    assert!(f.message.as_deref().unwrap_or("").contains("hit your limit"), "{:?}", f.message);
    assert_error_has_no_secrets(&e, &[]);
}

// ---------------------------------------------------------------------------
// 6. Cancelling (B9): the child AND its grandchild must be gone.
// ---------------------------------------------------------------------------

fn read_pid(path: &Path) -> Option<i32> {
    fs::read_to_string(path).ok()?.trim().parse().ok()
}

fn pid_alive(pid: i32) -> bool {
    Command::new("sh")
        .arg("-c")
        .arg(r#"kill -0 "$0" 2>/dev/null"#)
        .arg(pid.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Whatever happens in the test, do not leave a `sleep 300` behind.
struct PidGuard {
    dir: PathBuf,
}

impl Drop for PidGuard {
    fn drop(&mut self) {
        let child = read_pid(&self.dir.join("c.pid"));
        let grandchild = read_pid(&self.dir.join("gc.pid"));
        if let Some(pid) = child.filter(|p| pid_alive(*p)) {
            let _ = Command::new("sh")
                .arg("-c")
                .arg(r#"kill -9 -- "-$0" 2>/dev/null; kill -9 "$0" 2>/dev/null"#)
                .arg(pid.to_string())
                .status();
        }
        if let Some(pid) = grandchild.filter(|p| pid_alive(*p)) {
            let _ = Command::new("sh")
                .arg("-c")
                .arg(r#"kill -9 "$0" 2>/dev/null"#)
                .arg(pid.to_string())
                .status();
        }
    }
}

#[test]
fn claude_cancel_returns_cancelled_and_kills_child_and_grandchild() {
    let fake = Spec::new()
        .script(
            r#"    sleep 300 &
    echo $! > "$D/gc.pid"
    echo $$ > "$D/c.pid"
    wait
"#,
        )
        .build();
    let _guard = PidGuard { dir: fake.dir.path().to_path_buf() };

    let cancel = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::channel();
    let handle = {
        let program = fake.claude.clone();
        let req = fake.request(Effort::Quality, None, "prompt");
        let cancel = Arc::clone(&cancel);
        thread::spawn(move || {
            let result = ClaudeCli::at(program).run_typed_cancellable::<Out>(&req, |_| {}, cancel);
            let _ = tx.send(result);
        })
    };

    // Wait (at most 10 s) until the fake claude and its grandchild exist.
    let deadline = Instant::now() + Duration::from_secs(10);
    let (child_pid, grandchild_pid) = loop {
        let c = read_pid(&fake.dir.path().join("c.pid"));
        let g = read_pid(&fake.dir.path().join("gc.pid"));
        if let (Some(c), Some(g)) = (c, g) {
            break (c, g);
        }
        assert!(
            Instant::now() < deadline,
            "the fake claude never started its child and grandchild (calls: {:?})",
            fake.calls()
        );
        thread::sleep(Duration::from_millis(50));
    };
    assert!(pid_alive(child_pid) && pid_alive(grandchild_pid), "both should be running before the cancel");

    cancel.store(true, Ordering::SeqCst);

    let result = rx
        .recv_timeout(Duration::from_secs(15))
        .expect("run_typed_cancellable did not return within 15 s of the cancel");
    handle.join().expect("worker thread");
    let e = match result {
        Ok(r) => panic!("a cancelled run must fail, got {:?}", r.value),
        Err(e) => e,
    };
    let f = rf(&e);
    assert_eq!(f.kind, FailureKind::Cancelled, "{e:#}");
    assert_eq!(f.engine, EngineKind::Claude);

    // Both PIDs must disappear within 5 s.
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let child_alive = pid_alive(child_pid);
        let grandchild_alive = pid_alive(grandchild_pid);
        if !child_alive && !grandchild_alive {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "after the cancel, child alive = {child_alive} (pid {child_pid}), grandchild alive = {grandchild_alive} (pid {grandchild_pid})"
        );
        thread::sleep(Duration::from_millis(50));
    }
}

/// Design 3.3: `Cancelled` is judged first, whatever the CLI printed. A raised
/// flag must not turn into a login, quota or schema failure, or into a value.
#[test]
fn claude_cancel_flag_wins_over_whatever_the_cli_printed() {
    let failing = r#"{"type":"result","subtype":"success","is_error":true,"api_error_status":401,"result":"Not logged in · Please run /login"}"#;
    for (out, exit) in [(failing.to_string(), 1), (success_json(), 0)] {
        let fake = Spec::new().generation(out, exit, "").build();
        let program = fake.claude.clone();
        let req = fake.request(Effort::Quality, None, "prompt");
        let cancel = Arc::new(AtomicBool::new(true));
        let result = within(WATCHDOG_SECS, move || {
            ClaudeCli::at(program).run_typed_cancellable::<Out>(&req, |_| {}, cancel)
        });
        let e = match result {
            Ok(r) => panic!("a cancelled run must fail, got {:?}", r.value),
            Err(e) => e,
        };
        let f = rf(&e);
        assert_eq!(f.kind, FailureKind::Cancelled, "{e:#}");
        assert_eq!(f.engine, EngineKind::Claude);
    }
}

/// A flag that is already up before the run starts must still leave nothing
/// behind: the CLI (if it got to start) and its grandchild are gone.
#[test]
fn claude_cancel_before_the_run_leaves_no_process_behind() {
    let fake = Spec::new()
        .script(
            r#"    echo $$ > "$D/c.pid"
    sleep 300 &
    echo $! > "$D/gc.pid"
    wait
"#,
        )
        .build();
    let _guard = PidGuard { dir: fake.dir.path().to_path_buf() };
    let program = fake.claude.clone();
    let req = fake.request(Effort::Quality, None, "prompt");
    let cancel = Arc::new(AtomicBool::new(true));
    let result = within(WATCHDOG_SECS, move || {
        ClaudeCli::at(program).run_typed_cancellable::<Out>(&req, |_| {}, cancel)
    });
    let e = match result {
        Ok(r) => panic!("a cancelled run must fail, got {:?}", r.value),
        Err(e) => e,
    };
    assert_eq!(rf(&e).kind, FailureKind::Cancelled, "{e:#}");

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let alive: Vec<i32> = ["c.pid", "gc.pid"]
            .iter()
            .filter_map(|n| read_pid(&fake.dir.path().join(n)))
            .filter(|p| pid_alive(*p))
            .collect();
        if alive.is_empty() {
            break;
        }
        assert!(Instant::now() < deadline, "processes still alive after the cancel: {alive:?}");
        thread::sleep(Duration::from_millis(50));
    }
}
