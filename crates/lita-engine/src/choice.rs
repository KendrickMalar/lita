//! The per-device choice of engine, kept in a small JSON file.
//!
//! Format: `{"version":1,"engine":"codex"|"claude"}`. Reading never fails: a
//! missing, unreadable, malformed, other-version or unknown-engine file all
//! mean "Codex", which is also what a fresh device gets.

use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result};
use serde_json::{Value, json};
use tempfile::NamedTempFile;

use crate::EngineKind;

const VERSION: u64 = 1;

/// Reads the saved choice, falling back to [`EngineKind::Codex`] on anything
/// that is not exactly a version 1 file naming a known engine.
pub fn load_choice(path: &Path) -> EngineKind {
    read_choice(path).unwrap_or_default()
}

fn read_choice(path: &Path) -> Option<EngineKind> {
    let text = std::fs::read_to_string(path).ok()?;
    let value: Value = serde_json::from_str(&text).ok()?;
    if value.get("version")?.as_u64()? != VERSION {
        return None;
    }
    serde_json::from_value(value.get("engine")?.clone()).ok()
}

/// Saves the choice, creating missing parent directories. The file is written
/// to a temporary file in the same directory and renamed into place, so a
/// reader never sees a half-written file.
pub fn save_choice(path: &Path, kind: EngineKind) -> Result<()> {
    let dir = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    std::fs::create_dir_all(dir)
        .with_context(|| format!("failed to create {}", dir.display()))?;
    let mut tmp = NamedTempFile::new_in(dir)
        .with_context(|| format!("failed to create a temporary file in {}", dir.display()))?;
    let body = json!({ "version": VERSION, "engine": kind });
    tmp.write_all(serde_json::to_string(&body)?.as_bytes())
        .context("failed to write the engine choice")?;
    tmp.persist(path)
        .map_err(|e| e.error)
        .with_context(|| format!("failed to save {}", path.display()))?;
    Ok(())
}
