//! The only place that reads which engine the person picked. Every
//! generation goes through [`resolve`] (or [`resolve_for_queue`]), once per
//! run, and then keeps that `Engine` until the run ends.
//!
//! Nothing here switches to the other engine: not when the chosen one is
//! missing, not when it is logged out, not when it fails. The person chooses.

use std::path::{Path, PathBuf};

use lita_engine::{Engine, EngineKind, Preflight, load_choice, save_choice};
use serde::Serialize;
use tauri::Manager;

use crate::errors::UiError;

/// The per-device choice file, inside the app config directory.
pub const CHOICE_FILE: &str = "engine.json";

pub fn choice_path_in(config_dir: &Path) -> PathBuf {
    config_dir.join(CHOICE_FILE)
}

pub fn choice_path(app: &tauri::AppHandle) -> Result<PathBuf, UiError> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|e| UiError::unknown(e.to_string()))?;
    Ok(choice_path_in(&dir))
}

/// The saved choice; Codex when nothing usable is saved.
pub fn selected(choice_path: &Path) -> EngineKind {
    load_choice(choice_path)
}

pub fn select(choice_path: &Path, kind: EngineKind) -> Result<(), UiError> {
    save_choice(choice_path, kind).map_err(|e| UiError::unknown(format!("{e:#}")))
}

/// The engine the person has chosen right now. Reads the file on every call,
/// so a switch applies to the next run and never to one already under way.
pub fn resolve(choice_path: &Path) -> Engine {
    Engine::on_path(load_choice(choice_path))
}

/// Preflight of one named engine, for the status screen only (it shows both).
pub fn preflight_of(kind: EngineKind) -> anyhow::Result<Preflight> {
    Engine::on_path(kind).preflight()
}

/// What the UI needs to know about an engine before anything else can happen.
/// Each variant maps to a different next step for the user, which is why
/// they stay distinct instead of collapsing into ok/error.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum EngineStatus {
    Ready { version: String },
    NotLoggedIn { version: String },
    NotInstalled,
    ConfigBroken { version: String, message: String },
    Error { message: String },
}

impl From<Preflight> for EngineStatus {
    fn from(p: Preflight) -> Self {
        match p {
            Preflight::Ready { version } => EngineStatus::Ready { version },
            Preflight::NotLoggedIn { version } => EngineStatus::NotLoggedIn { version },
            Preflight::NotInstalled => EngineStatus::NotInstalled,
            Preflight::ConfigBroken { version, detail } => EngineStatus::ConfigBroken {
                version,
                message: detail,
            },
        }
    }
}

impl EngineStatus {
    pub fn from_result(result: anyhow::Result<Preflight>) -> Self {
        match result {
            Ok(preflight) => preflight.into(),
            Err(e) => EngineStatus::Error {
                message: format!("{e:#}"),
            },
        }
    }
}

/// Both engines' state plus the one the person picked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EnginesStatus {
    pub selected: EngineKind,
    pub codex: EngineStatus,
    pub claude: EngineStatus,
}

fn stop(code: &'static str, detail: impl Into<String>) -> UiError {
    UiError {
        code,
        detail: detail.into(),
    }
}

/// The error that stops the whole queue when the chosen engine is not usable,
/// or `None` when it is ready. A problem here hits every Article alike.
pub fn queue_stop_error(kind: EngineKind, preflight: Preflight) -> Option<UiError> {
    match (kind, preflight) {
        (_, Preflight::Ready { .. }) => None,
        (EngineKind::Codex, Preflight::NotInstalled) => Some(stop("codex_not_installed", "")),
        (EngineKind::Codex, Preflight::NotLoggedIn { .. }) => Some(stop("codex_not_logged_in", "")),
        (EngineKind::Codex, Preflight::ConfigBroken { detail, .. }) => {
            Some(stop("codex_config_broken", detail))
        }
        (EngineKind::Claude, Preflight::NotInstalled) => Some(stop("claude_not_installed", "")),
        (EngineKind::Claude, Preflight::NotLoggedIn { .. }) => {
            Some(stop("claude_not_logged_in", ""))
        }
        (EngineKind::Claude, Preflight::ConfigBroken { detail, .. }) => {
            Some(stop("unknown", detail))
        }
    }
}

/// Resolves the engine for one Article of the queue: reads the choice, checks
/// that engine with `preflight`, and hands back the very same `Engine` so the
/// Article is written by the engine that was checked.
pub fn resolve_for_queue(
    choice_path: &Path,
    preflight: impl FnOnce(&Engine) -> anyhow::Result<Preflight>,
) -> Result<Engine, UiError> {
    let engine = resolve(choice_path);
    let result = preflight(&engine).map_err(UiError::from)?;
    match queue_stop_error(engine.kind(), result) {
        Some(error) => Err(error),
        None => Ok(engine),
    }
}
