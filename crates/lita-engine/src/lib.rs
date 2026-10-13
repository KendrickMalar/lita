//! Drives the user's coding-agent CLIs (Codex and Claude Code) as subprocesses.
//!
//! Lita never reads, stores, or refreshes the user's credentials, whether
//! OpenAI's or Anthropic's. It spawns the `codex` or `claude` binary the user
//! has already logged into and reads structured output back from it.
//! Everything in this crate is built on that rule: if a change here would
//! require touching `~/.codex/auth.json` or a Claude Code credential, it is the
//! wrong change.

pub mod choice;
pub mod claude;
pub mod codex;
pub mod engine;
pub mod post;
pub mod sample;
pub mod topics;
pub mod presets;
pub mod voice;

use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

use serde_json::Value;

pub use choice::{load_choice, save_choice};
pub use claude::ClaudeCli;
pub use codex::{CodexCli, TurnFailure};
pub use engine::{Engine, EngineKind};

/// How many trailing bytes of stderr to keep for diagnostics.
pub(crate) const STDERR_TAIL_BYTES: usize = 4096;

/// What a preflight check found. Lita shows the user a different next step for
/// each of these, so they stay distinct rather than collapsing into one error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Preflight {
    /// Installed, and credentials are configured.
    Ready { version: String },
    /// Installed, but there are no credentials. The user needs `codex login`.
    NotLoggedIn { version: String },
    /// No `codex` binary was found.
    NotInstalled,
    /// Installed, but the CLI cannot load its own configuration (for
    /// example a duplicate key in `~/.codex/config.toml`), so nothing else
    /// can be checked. `detail` is the CLI's own summary.
    ConfigBroken { version: String, detail: String },
}

/// One line of the CLI's JSONL event stream, reduced to what a progress UI needs.
#[derive(Debug, Clone)]
pub struct Event {
    /// The event's `type` field, e.g. `thread.started` or `turn.completed`.
    pub kind: String,
    pub raw: Value,
}

/// Why a run failed, in terms Lita can turn into a useful message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureKind {
    /// Credentials are missing or expired.
    NotLoggedIn,
    /// The account's plan limit was reached.
    QuotaExhausted,
    /// The final message never satisfied the schema we asked for.
    SchemaMismatch,
    /// The caller asked for the run to stop (see `run_typed_cancellable`).
    Cancelled,
    /// Anything we could not classify. Show the stderr tail and move on.
    Unknown,
}

#[derive(Debug)]
pub struct RunFailure {
    /// Which CLI was driven when the run failed.
    pub engine: EngineKind,
    pub kind: FailureKind,
    pub exit_code: Option<i32>,
    /// What the CLI said about the failure, when it got that far.
    pub message: Option<String>,
    pub stderr_tail: String,
}

impl fmt::Display for RunFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let what = match (self.engine, self.kind) {
            (EngineKind::Codex, FailureKind::NotLoggedIn) => "Codex has no usable credentials",
            (EngineKind::Codex, FailureKind::QuotaExhausted) => {
                "the account's Codex usage limit was reached"
            }
            (EngineKind::Codex, FailureKind::SchemaMismatch) => {
                "Codex never returned output matching the schema"
            }
            (EngineKind::Codex, FailureKind::Unknown) => "the Codex CLI failed",
            (EngineKind::Claude, FailureKind::NotLoggedIn) => {
                "Claude Code has no usable credentials"
            }
            (EngineKind::Claude, FailureKind::QuotaExhausted) => {
                "the account's Claude Code usage limit was reached"
            }
            (EngineKind::Claude, FailureKind::SchemaMismatch) => {
                "Claude Code never returned output matching the schema"
            }
            (EngineKind::Claude, FailureKind::Unknown) => "the Claude Code CLI failed",
            (_, FailureKind::Cancelled) => "the run was cancelled",
        };
        write!(f, "{what} (exit {:?})", self.exit_code)?;
        if let Some(m) = &self.message {
            write!(f, "\n{m}")?;
        }
        if !self.stderr_tail.is_empty() {
            write!(f, "\n{}", self.stderr_tail)?;
        }
        Ok(())
    }
}

impl std::error::Error for RunFailure {}

/// How hard the model should think before answering.
///
/// Measured on a short X post: `Fast` returned in 6.6s, `Quality` in 10.8s,
/// with no visible quality difference at that length. Longer pieces may tell a
/// different story, which is why this is the user's choice and not ours.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Effort {
    /// Lower latency. Reasoning is turned down.
    Fast,
    /// Codex's own default.
    #[default]
    Quality,
    /// As much reasoning as Codex allows. Articles are always written with
    /// this (D-63, 2026-09-24): the person asked for the best, not a choice.
    Best,
}

impl Effort {
    pub(crate) fn as_config_value(self) -> &'static str {
        match self {
            Effort::Fast => "low",
            Effort::Quality => "medium",
            Effort::Best => "high",
        }
    }
}

/// A single generation request.
pub struct Request {
    /// The full prompt, handed to the CLI on stdin.
    pub prompt: String,
    /// A JSON Schema the final message must satisfy.
    pub schema: Value,
    /// Optional model override; `None` uses Codex's default.
    pub model: Option<String>,
    /// How hard to think. Surfaced to the user as a speed/quality switch.
    pub effort: Effort,
    /// Where to run. Deliberately does not have to be a git repository —
    /// Lita's data directory never will be.
    pub working_dir: PathBuf,
}

/// A completed run.
pub struct Run<T> {
    pub value: T,
    pub events: Vec<Event>,
    pub elapsed: Duration,
}

pub(crate) fn tail(s: &str) -> String {
    if s.len() <= STDERR_TAIL_BYTES {
        return s.to_string();
    }
    let start = s
        .char_indices()
        .rev()
        .map(|(i, _)| i)
        .find(|i| s.len() - i >= STDERR_TAIL_BYTES)
        .unwrap_or(0);
    format!("…{}", &s[start..])
}

/// Panics if a prompt is ever built from something that looks like a token.
///
/// Lita's prompts are assembled from the user's own writing, and the whole
/// premise is that credentials never enter this path.
pub fn assert_no_credentials(prompt: &str) {
    for marker in ["sk-", "xoxp-", "xoxb-", "eyJ"] {
        assert!(
            !prompt.contains(marker),
            "prompt contains something token-shaped: {marker}"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effort_maps_to_codex_reasoning_levels() {
        assert_eq!(Effort::Fast.as_config_value(), "low");
        assert_eq!(Effort::Quality.as_config_value(), "medium");
        assert_eq!(Effort::Best.as_config_value(), "high");
        assert_eq!(Effort::default(), Effort::Quality);
    }

    #[test]
    fn tail_keeps_the_end_of_a_long_string() {
        let long = "x".repeat(STDERR_TAIL_BYTES * 2);
        let t = tail(&long);
        assert!(t.starts_with('…'));
        assert!(t.len() <= STDERR_TAIL_BYTES + 8);
    }

    #[test]
    fn tail_leaves_a_short_string_alone() {
        assert_eq!(tail("short"), "short");
    }

    fn failure(engine: EngineKind, kind: FailureKind) -> RunFailure {
        RunFailure { engine, kind, exit_code: Some(1), message: None, stderr_tail: String::new() }
    }

    #[test]
    fn codex_failures_read_exactly_as_before() {
        let cases = [
            (FailureKind::NotLoggedIn, "Codex has no usable credentials (exit Some(1))"),
            (FailureKind::QuotaExhausted, "the account's Codex usage limit was reached (exit Some(1))"),
            (FailureKind::SchemaMismatch, "Codex never returned output matching the schema (exit Some(1))"),
            (FailureKind::Cancelled, "the run was cancelled (exit Some(1))"),
            (FailureKind::Unknown, "the Codex CLI failed (exit Some(1))"),
        ];
        for (kind, text) in cases {
            assert_eq!(failure(EngineKind::Codex, kind).to_string(), text);
        }
    }

    #[test]
    fn claude_failures_name_claude_code() {
        let cases = [
            (FailureKind::NotLoggedIn, "Claude Code has no usable credentials (exit Some(1))"),
            (FailureKind::QuotaExhausted, "the account's Claude Code usage limit was reached (exit Some(1))"),
            (FailureKind::SchemaMismatch, "Claude Code never returned output matching the schema (exit Some(1))"),
            (FailureKind::Cancelled, "the run was cancelled (exit Some(1))"),
            (FailureKind::Unknown, "the Claude Code CLI failed (exit Some(1))"),
        ];
        for (kind, text) in cases {
            assert_eq!(failure(EngineKind::Claude, kind).to_string(), text);
        }
    }

    #[test]
    fn failure_display_appends_the_message_then_the_stderr_tail() {
        let f = RunFailure {
            engine: EngineKind::Claude,
            kind: FailureKind::Unknown,
            exit_code: None,
            message: Some("said this".into()),
            stderr_tail: "and this".into(),
        };
        assert_eq!(f.to_string(), "the Claude Code CLI failed (exit None)\nsaid this\nand this");
    }
}
