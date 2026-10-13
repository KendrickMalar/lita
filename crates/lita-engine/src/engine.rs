//! Which coding-agent CLI Lita drives.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use anyhow::Result;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::{ClaudeCli, CodexCli, Event, Preflight, Request, Run};

/// The CLI a run goes through. Serialized in lowercase (`"codex"`, `"claude"`)
/// because that is what the per-device choice file stores.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EngineKind {
    #[default]
    Codex,
    Claude,
}

impl EngineKind {
    /// The product's name as the person knows it.
    pub fn display_name(self) -> &'static str {
        match self {
            EngineKind::Codex => "Codex",
            EngineKind::Claude => "Claude Code",
        }
    }
}

/// One coding-agent CLI, ready to be driven.
///
/// Every method hands the call to the wrapped CLI and nothing else. There is
/// deliberately no path from one variant to the other: not on failure, not on
/// a missing login, not as a fallback. Which engine runs is the caller's choice
/// alone.
#[derive(Debug, Clone)]
pub enum Engine {
    Codex(CodexCli),
    Claude(ClaudeCli),
}

impl Engine {
    /// The engine of this kind, resolved from `PATH`. Starts nothing.
    pub fn on_path(kind: EngineKind) -> Self {
        match kind {
            EngineKind::Codex => Engine::Codex(CodexCli::on_path()),
            EngineKind::Claude => Engine::Claude(ClaudeCli::on_path()),
        }
    }

    /// Which CLI this engine drives.
    pub fn kind(&self) -> EngineKind {
        match self {
            Engine::Codex(_) => EngineKind::Codex,
            Engine::Claude(_) => EngineKind::Claude,
        }
    }

    /// See `CodexCli::preflight` and `ClaudeCli::preflight`.
    pub fn preflight(&self) -> Result<Preflight> {
        match self {
            Engine::Codex(cli) => cli.preflight(),
            Engine::Claude(cli) => cli.preflight(),
        }
    }

    /// Runs one prompt and deserializes the structured output into `T`.
    pub fn run_typed<T: DeserializeOwned>(
        &self,
        req: &Request,
        on_event: impl FnMut(&Event),
    ) -> Result<Run<T>> {
        match self {
            Engine::Codex(cli) => cli.run_typed(req, on_event),
            Engine::Claude(cli) => cli.run_typed(req, on_event),
        }
    }

    /// Like `run_typed`, but stops as soon as `cancel` becomes true.
    pub fn run_typed_cancellable<T: DeserializeOwned>(
        &self,
        req: &Request,
        on_event: impl FnMut(&Event),
        cancel: Arc<AtomicBool>,
    ) -> Result<Run<T>> {
        match self {
            Engine::Codex(cli) => cli.run_typed_cancellable(req, on_event, cancel),
            Engine::Claude(cli) => cli.run_typed_cancellable(req, on_event, cancel),
        }
    }
}
