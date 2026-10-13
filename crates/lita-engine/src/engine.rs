//! Which coding-agent CLI Lita drives.

use serde::{Deserialize, Serialize};

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
