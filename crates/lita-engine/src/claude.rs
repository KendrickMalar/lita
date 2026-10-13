//! Drives the Claude Code CLI as a subprocess.
//!
//! Lita never reads, stores, or refreshes the user's Anthropic credentials. It
//! spawns the `claude` binary the user has already logged into and reads one
//! JSON result back from it.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::{EngineKind, Event, FailureKind, Preflight, Request, Run, RunFailure, tail};

/// A located Claude Code CLI.
#[derive(Debug, Clone)]
pub struct ClaudeCli {
    program: PathBuf,
}

impl ClaudeCli {
    /// Uses whatever `claude` is on PATH.
    pub fn on_path() -> Self {
        Self {
            program: PathBuf::from("claude"),
        }
    }

    /// Uses a specific binary, for users who installed it somewhere unusual.
    pub fn at(program: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
        }
    }

    /// Finds out whether `claude` is installed and logged in.
    ///
    /// Two cheap commands: `claude --version`, then `claude auth status
    /// --json`. There is no `ConfigBroken` here: Lita runs `claude` with
    /// `--setting-sources ""`, so the user's own settings cannot break a run.
    pub fn preflight(&self) -> Result<Preflight> {
        let out = match Command::new(&self.program)
            .arg("--version")
            .stdin(Stdio::null())
            .output()
        {
            Ok(out) => out,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Preflight::NotInstalled);
            }
            Err(e) => return Err(e).context("failed to run `claude --version`"),
        };
        if !out.status.success() {
            anyhow::bail!("`claude --version` exited with {:?}", out.status.code());
        }
        let stdout = String::from_utf8_lossy(&out.stdout);
        let version = stdout
            .split_whitespace()
            .next()
            .context("`claude --version` printed nothing")?
            .to_string();

        // The error text names only the command, never its output: the output
        // of `auth status` carries the user's email address and organisation.
        let logged_in = self
            .auth_logged_in()
            .context("failed to run `claude auth status --json`")?
            .context("`claude auth status --json` did not report loggedIn")?;
        Ok(if logged_in {
            Preflight::Ready { version }
        } else {
            Preflight::NotLoggedIn { version }
        })
    }

    /// Whether the CLI has credentials, according to `claude auth status`.
    ///
    /// Returns `None` when the command cannot be run or its answer cannot be
    /// understood; callers then carry on and let the run itself say.
    pub fn logged_in(&self) -> Option<bool> {
        self.auth_logged_in().ok().flatten()
    }

    /// Runs `claude auth status --json` and reads `loggedIn` out of it.
    ///
    /// The exit code is ignored on purpose: a logged-out CLI exits 1 and still
    /// prints the JSON. `loggedIn` is the only field read. Everything else in
    /// that output (email, organisation) is personal and must never be copied
    /// into an error, a log line or a `RunFailure`.
    fn auth_logged_in(&self) -> std::io::Result<Option<bool>> {
        let out = Command::new(&self.program)
            .args(["auth", "status", "--json"])
            .stdin(Stdio::null())
            .output()?;
        Ok(serde_json::from_slice::<Value>(&out.stdout)
            .ok()
            .and_then(|v| v["loggedIn"].as_bool()))
    }

    /// Runs one prompt and deserializes the structured output into `T`.
    ///
    /// Claude Code prints one JSON result at the end, so there are no events to
    /// report and `on_event` is never called.
    pub fn run_typed<T: DeserializeOwned>(
        &self,
        req: &Request,
        on_event: impl FnMut(&Event),
    ) -> Result<Run<T>> {
        self.run_typed_cancellable(req, on_event, Arc::new(AtomicBool::new(false)))
    }

    /// Like `run_typed`, but stops (killing the CLI and everything it started)
    /// as soon as `cancel` becomes true. A cancelled run fails with
    /// `FailureKind::Cancelled`.
    pub fn run_typed_cancellable<T: DeserializeOwned>(
        &self,
        req: &Request,
        _on_event: impl FnMut(&Event),
        cancel: Arc<AtomicBool>,
    ) -> Result<Run<T>> {
        // Spare a run that would only fail with a 401. `None` (could not tell)
        // is not a reason to refuse: let the run report what is wrong.
        if self.logged_in() == Some(false) {
            return Err(failure(FailureKind::NotLoggedIn, None, None, String::new()).into());
        }

        let started = Instant::now();
        let mut child = self
            .command(req)
            .spawn()
            .with_context(|| format!("failed to spawn {}", self.program.display()))?;

        // Hand over the prompt and close stdin, or the CLI waits forever. This
        // is a thread of its own so that a CLI which stops reading cannot block
        // us (or a cancel) on a full pipe. A failed write is not an error by
        // itself: the CLI may have exited early, and its exit status and output
        // say more than a BrokenPipe does.
        let mut stdin = child.stdin.take().expect("stdin was piped");
        let prompt = req.prompt.clone().into_bytes();
        std::thread::spawn(move || {
            let _ = stdin.write_all(&prompt);
        });

        // Drain stderr on its own thread so a full pipe cannot stall the CLI.
        let mut stderr = child.stderr.take().expect("stderr was piped");
        let stderr_reader = std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = stderr.read_to_end(&mut buf);
            String::from_utf8_lossy(&buf).into_owned()
        });
        let mut stdout = child.stdout.take().expect("stdout was piped");

        // A watcher stops the CLI when asked to cancel; that closes stdout and
        // stderr, which ends the reads below.
        let child = Arc::new(Mutex::new(child));
        let finished = Arc::new(AtomicBool::new(false));
        let watcher = {
            let child = Arc::clone(&child);
            let finished = Arc::clone(&finished);
            let cancel = Arc::clone(&cancel);
            std::thread::spawn(move || {
                while !finished.load(Ordering::Relaxed) {
                    if cancel.load(Ordering::Relaxed) {
                        let mut c = child.lock().unwrap_or_else(|e| e.into_inner());
                        // Only act on a child that has not been reaped yet, so
                        // the pid cannot belong to something else by now.
                        if matches!(c.try_wait(), Ok(None)) {
                            kill_tree(&mut c);
                        }
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
            })
        };

        let mut raw = Vec::new();
        // Read until EOF: it arrives when the CLI exits, or when a cancel
        // kills it. A read error just ends the output early; the exit status
        // below decides what that means.
        let _ = stdout.read_to_end(&mut raw);

        // Poll instead of blocking in `wait()`: blocking would hold the lock
        // the watcher needs to kill a child that closed stdout but lives on.
        let status = loop {
            let polled = child
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .try_wait();
            match polled {
                Ok(Some(status)) => break Ok(status),
                Ok(None) => std::thread::sleep(Duration::from_millis(10)),
                Err(e) => break Err(e),
            }
        };
        finished.store(true, Ordering::Relaxed);
        let _ = watcher.join();
        let status = status.context("failed to wait for the Claude Code CLI")?;
        let stderr = stderr_reader.join().unwrap_or_default();
        let elapsed = started.elapsed();

        if cancel.load(Ordering::Relaxed) {
            return Err(failure(FailureKind::Cancelled, status.code(), None, String::new()).into());
        }

        let text = String::from_utf8_lossy(&raw);
        let result = parse_result(&text);
        let is_error = result
            .as_ref()
            .is_some_and(|v| v["is_error"].as_bool() == Some(true));

        if !status.success() || is_error {
            let message = result
                .as_ref()
                .and_then(|v| v["result"].as_str())
                .map(str::to_string);
            let kind = result
                .as_ref()
                .map_or(FailureKind::Unknown, classify);
            return Err(failure(kind, status.code(), message, tail(&stderr)).into());
        }

        let mismatch = |why: String| {
            failure(
                FailureKind::SchemaMismatch,
                status.code(),
                Some(why),
                tail(&stderr),
            )
        };
        let Some(result) = result else {
            return Err(mismatch("the output was not a JSON result".into()).into());
        };
        let output = &result["structured_output"];
        if output.is_null() {
            return Err(mismatch("the result carried no structured_output".into()).into());
        }
        let value = serde_json::from_value::<T>(output.clone()).map_err(|e| {
            mismatch(format!(
                "structured_output did not match the schema we asked for: {e}"
            ))
        })?;

        Ok(Run {
            value,
            events: Vec::new(),
            elapsed,
        })
    }

    fn command(&self, req: &Request) -> Command {
        let mut cmd = Command::new(&self.program);
        cmd.arg("-p")
            // One JSON result at the end; the typed value is read from it.
            .arg("--output-format")
            .arg("json")
            // Force the final answer to satisfy our schema.
            .arg("--json-schema")
            .arg(req.schema.to_string())
            // Writing needs no tools, and tools would mean permission prompts a
            // non-interactive run cannot answer.
            .arg("--tools")
            .arg("")
            // Lita's output must not depend on the user's CLAUDE.md, hooks or
            // plugins. Not `--bare`: it skips OAuth and the keychain, so a
            // user who logged in with claude.ai could no longer run anything.
            .arg("--setting-sources")
            .arg("")
            .arg("--strict-mcp-config")
            .arg("--disable-slash-commands")
            // Keep the user's drafts out of their Claude Code session history.
            .arg("--no-session-persistence")
            .arg("--effort")
            .arg(req.effort.as_config_value());

        if let Some(model) = &req.model {
            cmd.arg("--model").arg(model);
        }

        // The prompt arrives on stdin.
        cmd.current_dir(&req.working_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        // Own process group, so a cancel can stop the CLI and every process it
        // started (a shell, a node helper) in one go. Killing only the CLI
        // would leave those running.
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            cmd.process_group(0);
        }
        cmd
    }
}

fn failure(
    kind: FailureKind,
    exit_code: Option<i32>,
    message: Option<String>,
    stderr_tail: String,
) -> RunFailure {
    RunFailure {
        engine: EngineKind::Claude,
        kind,
        exit_code,
        message,
        stderr_tail,
    }
}

/// Kills the CLI and everything in its process group.
///
/// The child leads its own group (`process_group(0)`), so its pid is the group
/// id and `kill -KILL -<pgid>` reaches every descendant that did not leave the
/// group. If that fails, fall back to killing just the child.
fn kill_tree(child: &mut Child) {
    #[cfg(unix)]
    {
        let stopped = Command::new("/bin/kill")
            .arg("-KILL")
            .arg(format!("-{}", child.id()))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        if stopped {
            return;
        }
    }
    let _ = child.kill();
}

/// Reads the result object from the CLI's stdout: all of it as one JSON
/// value, else the last non-empty line (in case something printed ahead of it).
fn parse_result(stdout: &str) -> Option<Value> {
    if let Ok(v) = serde_json::from_str::<Value>(stdout.trim()) {
        return Some(v);
    }
    let last = stdout.lines().rev().find(|l| !l.trim().is_empty())?;
    serde_json::from_str::<Value>(last.trim()).ok()
}

/// Maps a failed result to something Lita can act on. The first match wins.
///
/// Only "not logged in" was measured (claude 2.1.296, 2026-10-13: exit 1,
/// `is_error: true`, `result: "Not logged in · Please run /login"`). The usage
/// limit and schema words come from the design (basic design 3.3) and could
/// not be reproduced on demand.
fn classify(result: &Value) -> FailureKind {
    let status = result["api_error_status"].as_i64();
    let text = result["result"].as_str().unwrap_or("").to_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| text.contains(w));

    if status == Some(401)
        || has(&[
            "not logged in",
            "/login",
            "401",
            "unauthorized",
            "invalid api key",
            "oauth token",
        ])
    {
        FailureKind::NotLoggedIn
    } else if status == Some(429)
        || has(&[
            "usage limit",
            "limit reached",
            "hit your limit",
            "rate limit",
            "quota",
        ])
    {
        FailureKind::QuotaExhausted
    } else if result["subtype"].as_str() == Some("error_max_structured_output_retries")
        || has(&["structured output", "schema"])
    {
        FailureKind::SchemaMismatch
    } else {
        FailureKind::Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_result_reads_the_whole_output_or_its_last_line() {
        assert_eq!(parse_result(" {\"a\":1}\n"), Some(json!({"a": 1})));
        assert_eq!(
            parse_result("warning: something\n{\"a\":2}\n\n"),
            Some(json!({"a": 2}))
        );
        assert_eq!(parse_result("not json"), None);
        assert_eq!(parse_result(""), None);
    }

    #[test]
    fn the_first_matching_class_wins() {
        // Mentions both a login problem and a limit: login comes first.
        let both = json!({"result": "401 Unauthorized, usage limit"});
        assert_eq!(classify(&both), FailureKind::NotLoggedIn);
        let limit_and_schema = json!({"result": "quota exceeded for schema"});
        assert_eq!(classify(&limit_and_schema), FailureKind::QuotaExhausted);
        assert_eq!(classify(&json!({"result": "boom"})), FailureKind::Unknown);
    }
}
