//! Drives the Codex CLI as a subprocess.
//!
//! Lita never reads, stores, or refreshes the user's OpenAI credentials. It
//! spawns the `codex` binary the user has already logged into and reads
//! structured output back from it.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::{EngineKind, Event, FailureKind, Preflight, Request, Run, RunFailure, tail};

/// A located Codex CLI.
#[derive(Debug, Clone)]
pub struct CodexCli {
    program: PathBuf,
}

impl CodexCli {
    /// Uses whatever `codex` is on PATH.
    pub fn on_path() -> Self {
        Self {
            program: PathBuf::from("codex"),
        }
    }

    /// Uses a specific binary, for users who installed it somewhere unusual.
    pub fn at(program: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
        }
    }

    /// Asks the CLI to describe its own health.
    ///
    /// `codex doctor --json` emits a redacted machine-readable report, which is
    /// far steadier to read than scraping the human-facing output.
    pub fn preflight(&self) -> Result<Preflight> {
        let out = match Command::new(&self.program)
            .arg("doctor")
            .arg("--json")
            .output()
        {
            Ok(out) => out,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Preflight::NotInstalled);
            }
            Err(e) => return Err(e).context("failed to run `codex doctor --json`"),
        };

        let report: Value = serde_json::from_slice(&out.stdout)
            .context("`codex doctor --json` did not produce JSON")?;
        Ok(interpret_doctor(&report))
    }

    /// Whether the CLI has credentials, without touching the network.
    ///
    /// `codex login status` exits 0 when logged in and 1 otherwise, in about
    /// 10 ms. Checking first spares a run that would otherwise spend ten
    /// seconds retrying a 401 (verified against codex-cli 0.155.1).
    /// Returns `None` if the command itself could not be run.
    pub fn logged_in(&self) -> Option<bool> {
        let out = Command::new(&self.program).args(["login", "status"]).output().ok()?;
        if out.status.success() {
            return Some(true);
        }
        // Exit 1 also covers a configuration the CLI cannot load; only the
        // CLI's own words settle it.
        let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)).to_lowercase();
        if text.contains("not logged in") { Some(false) } else { None }
    }

    /// Runs one prompt and deserializes the final message into `T`.
    ///
    /// `on_event` is called as each JSONL event arrives, so a caller can drive
    /// a progress indicator without waiting for the run to finish.
    pub fn run_typed<T: DeserializeOwned>(
        &self,
        req: &Request,
        on_event: impl FnMut(&Event),
    ) -> Result<Run<T>> {
        self.run_typed_cancellable(req, on_event, Arc::new(AtomicBool::new(false)))
    }

    /// Like `run_typed`, but stops (killing the CLI) as soon as `cancel`
    /// becomes true. A cancelled run fails with `FailureKind::Cancelled`.
    pub fn run_typed_cancellable<T: DeserializeOwned>(
        &self,
        req: &Request,
        mut on_event: impl FnMut(&Event),
        cancel: Arc<AtomicBool>,
    ) -> Result<Run<T>> {
        if self.logged_in() == Some(false) {
            return Err(RunFailure {
                engine: EngineKind::Codex,
                kind: FailureKind::NotLoggedIn,
                exit_code: None,
                message: None,
                stderr_tail: String::new(),
            }
            .into());
        }
        let scratch = tempfile::tempdir().context("failed to create a scratch directory")?;
        let schema_path = scratch.path().join("schema.json");
        let output_path = scratch.path().join("final.json");
        std::fs::write(&schema_path, serde_json::to_vec_pretty(&req.schema)?)
            .context("failed to write the output schema")?;

        let started = Instant::now();
        let mut child = self
            .command(req, &schema_path, &output_path)
            .spawn()
            .with_context(|| format!("failed to spawn {}", self.program.display()))?;

        // Hand over the prompt and close stdin, or the CLI waits forever.
        child
            .stdin
            .take()
            .expect("stdin was piped")
            .write_all(req.prompt.as_bytes())
            .context("failed to write the prompt to stdin")?;

        // Drain stderr on its own thread. The CLI streams progress there, and a
        // full pipe buffer would deadlock the stdout reader below.
        let mut stderr = child.stderr.take().expect("stderr was piped");
        let stderr_reader = std::thread::spawn(move || {
            let mut buf = String::new();
            let _ = stderr.read_to_string(&mut buf);
            buf
        });

        let stdout = child.stdout.take().expect("stdout was piped");

        // A watcher kills the child when asked to cancel; that closes stdout,
        // which ends the read loop below.
        let child = Arc::new(Mutex::new(child));
        let finished = Arc::new(AtomicBool::new(false));
        let watcher = {
            let child = Arc::clone(&child);
            let finished = Arc::clone(&finished);
            let cancel = Arc::clone(&cancel);
            std::thread::spawn(move || {
                while !finished.load(Ordering::Relaxed) {
                    if cancel.load(Ordering::Relaxed) {
                        if let Ok(mut c) = child.lock() {
                            let _ = c.kill();
                        }
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
            })
        };
        let mut events = Vec::new();
        for line in BufReader::new(stdout).lines() {
            let line = line.context("failed to read the event stream")?;
            if line.trim().is_empty() {
                continue;
            }
            // A line that is not JSON is not worth failing the run over; the
            // typed output is what we actually need.
            if let Ok(raw) = serde_json::from_str::<Value>(&line) {
                let kind = raw["type"].as_str().unwrap_or("unknown").to_string();
                let event = Event { kind, raw };
                on_event(&event);
                events.push(event);
            }
        }

        let status = child
            .lock()
            .map_err(|_| anyhow::anyhow!("child handle poisoned"))?
            .wait()
            .context("failed to wait for the Codex CLI")?;
        finished.store(true, Ordering::Relaxed);
        let _ = watcher.join();
        let stderr = stderr_reader.join().unwrap_or_default();
        let elapsed = started.elapsed();

        if cancel.load(Ordering::Relaxed) {
            return Err(RunFailure {
                engine: EngineKind::Codex,
                kind: FailureKind::Cancelled,
                exit_code: status.code(),
                message: None,
                stderr_tail: String::new(),
            }
            .into());
        }
        if !status.success() {
            let failure = turn_failure(&events);
            return Err(RunFailure {
                engine: EngineKind::Codex,
                kind: classify(&failure, &stderr),
                exit_code: status.code(),
                message: failure.message,
                stderr_tail: tail(&stderr),
            }
            .into());
        }

        let raw = std::fs::read_to_string(&output_path).with_context(|| {
            format!(
                "the run succeeded but wrote no output to {}",
                output_path.display()
            )
        })?;
        let value = serde_json::from_str::<T>(&raw).with_context(|| {
            format!(
                "the final message did not match the schema we asked for:\n{}",
                tail(&raw)
            )
        })?;

        Ok(Run {
            value,
            events,
            elapsed,
        })
    }

    fn command(&self, req: &Request, schema: &Path, output: &Path) -> Command {
        let mut cmd = Command::new(&self.program);
        cmd.arg("exec")
            // Turn stdout into a JSONL event stream we can drive a UI from.
            .arg("--json")
            // Force the final message to satisfy our schema.
            .arg("--output-schema")
            .arg(schema)
            .arg("--output-last-message")
            .arg(output)
            // Read-only means no approval request can ever arrive, and a
            // non-interactive run dies on approval requests.
            .arg("--sandbox")
            .arg("read-only")
            // Lita's working directory is not a git repository, and `codex
            // exec` refuses to start in one otherwise.
            .arg("--skip-git-repo-check")
            // Keep the user's drafts out of ~/.codex session rollouts.
            .arg("--ephemeral")
            // Lita's output must not depend on the user's Codex profile,
            // AGENTS.md files or MCP servers. Without this, a run picks up
            // whatever they have configured — we saw one fail to refresh an
            // unrelated MCP server's OAuth token mid-generation. Credentials
            // live in auth.json, not config.toml, so login still works.
            .arg("--ignore-user-config")
            .arg("-c")
            .arg(format!(
                "model_reasoning_effort=\"{}\"",
                req.effort.as_config_value()
            ));

        if let Some(model) = &req.model {
            cmd.arg("--model").arg(model);
        }

        // `-` makes stdin the prompt.
        cmd.arg("-")
            .current_dir(&req.working_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        cmd
    }
}

/// What Codex reported about a failed turn, pulled from its own event stream.
///
/// `codex exec --json` ends a failed run with `{"type":"turn.failed","error":
/// {"message": …}}` (and repeats the text in `{"type":"error"}` events before
/// it). That record is the sturdy signal; stderr is tracing output that
/// changes shape between releases. Newer CLIs may add a machine-readable
/// `codexErrorInfo` next to the message, so it is read when present.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct TurnFailure {
    pub message: Option<String>,
    pub info: Option<String>,
}

fn turn_failure(events: &[Event]) -> TurnFailure {
    let failed = events.iter().rev().find(|e| e.kind == "turn.failed");
    let error = failed.map(|e| &e.raw["error"]);
    let message = error
        .and_then(|e| e["message"].as_str())
        .or_else(|| {
            events
                .iter()
                .rev()
                .find(|e| e.kind == "error")
                .and_then(|e| e.raw["message"].as_str())
        })
        .map(str::to_string);
    let info = error
        .and_then(|e| e["codexErrorInfo"].as_str().or_else(|| e["codex_error_info"].as_str()))
        .map(str::to_string);
    TurnFailure { message, info }
}

/// Maps a failure to something Lita can act on.
///
/// Verified 2026-09-22 against codex-cli 0.155.1: with no credentials the
/// stream ends in `turn.failed` whose message carries `401 Unauthorized:
/// Missing bearer or basic authentication in header`; an exhausted plan ends
/// in `turn.failed` with `You've hit your usage limit …` (from the CLI's
/// public issue tracker; not reproducible on demand).
fn classify(failure: &TurnFailure, stderr: &str) -> FailureKind {
    if let Some(info) = failure.info.as_deref() {
        let info = info.to_ascii_lowercase();
        if info.contains("unauthorized") {
            return FailureKind::NotLoggedIn;
        }
        if info.contains("usagelimit") || info.contains("usage_limit") || info.contains("ratelimit") {
            return FailureKind::QuotaExhausted;
        }
    }
    let text = failure.message.as_deref().unwrap_or(stderr);
    let lower = text.to_lowercase();
    if lower.contains("401") || lower.contains("unauthorized") || lower.contains("missing bearer")
        || lower.contains("not logged in") || lower.contains("codex login")
    {
        FailureKind::NotLoggedIn
    } else if lower.contains("usage limit")
        || lower.contains("rate limit")
        || lower.contains("too many requests")
        || lower.contains("quota")
        || lower.contains("429")
    {
        FailureKind::QuotaExhausted
    } else if lower.contains("schema") {
        FailureKind::SchemaMismatch
    } else {
        FailureKind::Unknown
    }
}

/// Reads a `codex doctor --json` report into a [`Preflight`].
///
/// A broken config makes `doctor` skip the auth check entirely; that must
/// not read as "not logged in" (seen 2026-09-23 with a duplicate key in
/// config.toml). Only an `error` on `config.load`, or a config.toml that
/// did not parse, counts as broken: `doctor` also reports `warning` for
/// harmless things such as a deprecated setting, and the config loaded
/// fine in that case (seen 2026-09-23 with `analytics_enabled`).
fn interpret_doctor(report: &Value) -> Preflight {
    let version = report["codexVersion"]
        .as_str()
        .unwrap_or("unknown")
        .to_string();
    let config = &report["checks"]["config.load"];
    let status = config["status"].as_str();
    let parsed = config["details"]["config.toml parse"].as_str();
    if status == Some("error") || parsed.is_some_and(|p| p != "ok") {
        let detail = config["summary"]
            .as_str()
            .unwrap_or("the Codex configuration could not be loaded")
            .to_string();
        return Preflight::ConfigBroken { version, detail };
    }
    let auth = &report["checks"]["auth.credentials"]["status"];
    if auth.is_null() {
        return Preflight::ConfigBroken { version, detail: "codex doctor did not report on credentials".into() };
    }
    if auth.as_str() == Some("ok") {
        Preflight::Ready { version }
    } else {
        Preflight::NotLoggedIn { version }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(json: &str) -> Event {
        let raw: Value = serde_json::from_str(json).unwrap();
        Event { kind: raw["type"].as_str().unwrap().to_string(), raw }
    }

    // Captured from codex-cli 0.155.1 with an empty CODEX_HOME (2026-09-22).
    const NOT_LOGGED_IN: &str = r#"{"type":"turn.failed","error":{"message":"unexpected status 401 Unauthorized: Missing bearer or basic authentication in header, url: https://api.openai.com/v1/responses, cf-ray: a3f1f0aa1f32dfaa-NRT, request id: req_05817a7d59a044429961d9e51826b96d"}}"#;
    // As reported for codex-cli on the public tracker (openai/codex).
    const USAGE_LIMIT: &str = r#"{"type":"turn.failed","error":{"message":"You've hit your usage limit. Upgrade to Plus to continue using Codex, or try again at Sep 13th, 2026 10:04 PM."}}"#;

    #[test]
    fn reads_the_turn_failed_record() {
        let events = vec![event(r#"{"type":"thread.started","thread_id":"t"}"#), event(r#"{"type":"error","message":"Reconnecting... 2/5"}"#), event(NOT_LOGGED_IN)];
        let f = turn_failure(&events);
        assert!(f.message.unwrap().starts_with("unexpected status 401"));
        assert_eq!(f.info, None);
    }

    #[test]
    fn classifies_a_login_failure_from_the_stream() {
        let f = turn_failure(&[event(NOT_LOGGED_IN)]);
        assert_eq!(classify(&f, "unrelated stderr noise"), FailureKind::NotLoggedIn);
    }

    #[test]
    fn classifies_a_quota_failure_from_the_stream() {
        let f = turn_failure(&[event(USAGE_LIMIT)]);
        assert_eq!(classify(&f, ""), FailureKind::QuotaExhausted);
    }

    #[test]
    fn prefers_machine_readable_error_info_when_present() {
        let f = turn_failure(&[event(r#"{"type":"turn.failed","error":{"message":"anything","codexErrorInfo":"usageLimitExceeded"}}"#)]);
        assert_eq!(classify(&f, ""), FailureKind::QuotaExhausted);
        let f = turn_failure(&[event(r#"{"type":"turn.failed","error":{"message":"anything","codexErrorInfo":"unauthorized"}}"#)]);
        assert_eq!(classify(&f, ""), FailureKind::NotLoggedIn);
    }

    #[test]
    fn falls_back_to_stderr_without_a_turn_record() {
        assert_eq!(classify(&TurnFailure::default(), "error: not logged in, run codex login"), FailureKind::NotLoggedIn);
        assert_eq!(classify(&TurnFailure::default(), "You have hit your usage limit."), FailureKind::QuotaExhausted);
        assert_eq!(classify(&TurnFailure::default(), "segfault"), FailureKind::Unknown);
    }


    fn doctor(config_status: &str, parse: &str, auth: Option<&str>) -> Value {
        let mut checks = serde_json::json!({
            "config.load": { "status": config_status, "summary": "config loaded", "details": { "config.toml parse": parse } }
        });
        if let Some(a) = auth {
            checks["auth.credentials"] = serde_json::json!({ "status": a });
        }
        serde_json::json!({ "codexVersion": "0.156.0", "checks": checks })
    }

    #[test]
    fn doctor_warning_on_config_is_not_broken() {
        // A deprecated setting yields status "warning" with the config loaded.
        assert!(matches!(interpret_doctor(&doctor("warning", "ok", Some("ok"))), Preflight::Ready { .. }));
        assert!(matches!(interpret_doctor(&doctor("warning", "ok", Some("error"))), Preflight::NotLoggedIn { .. }));
    }

    #[test]
    fn doctor_error_or_parse_failure_is_broken() {
        assert!(matches!(interpret_doctor(&doctor("error", "ok", None)), Preflight::ConfigBroken { .. }));
        assert!(matches!(interpret_doctor(&doctor("warning", "failed", Some("ok"))), Preflight::ConfigBroken { .. }));
        // No auth report at all still reads as a config problem, never as "not logged in".
        assert!(matches!(interpret_doctor(&doctor("ok", "ok", None)), Preflight::ConfigBroken { .. }));
    }
}
