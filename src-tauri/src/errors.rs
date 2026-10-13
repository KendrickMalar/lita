//! Errors as the UI receives them: a stable `code` the UI turns into a
//! sentence in the person's language, plus the raw `detail` for a
//! "show details" disclosure. Never the only thing shown (R-5).

use lita_engine::{FailureKind, RunFailure};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct UiError {
    pub code: &'static str,
    pub detail: String,
}

impl UiError {
    pub fn not_signed_in() -> Self {
        Self {
            code: "not_signed_in",
            detail: String::new(),
        }
    }
    pub fn invalid(detail: impl Into<String>) -> Self {
        Self {
            code: "invalid_input",
            detail: detail.into(),
        }
    }
    pub fn unknown(detail: impl Into<String>) -> Self {
        Self {
            code: "unknown",
            detail: detail.into(),
        }
    }
}

impl From<anyhow::Error> for UiError {
    fn from(e: anyhow::Error) -> Self {
        let detail = format!("{e:#}");
        if let Some(run) = e.downcast_ref::<RunFailure>() {
            let code = match run.kind {
                FailureKind::NotLoggedIn => "codex_not_logged_in",
                FailureKind::QuotaExhausted => "codex_quota",
                FailureKind::SchemaMismatch => "codex_schema",
                FailureKind::Cancelled => "cancelled",
                FailureKind::Unknown => "codex_failed",
            };
            return Self { code, detail };
        }
        let lower = detail.to_lowercase();
        let code = if lower.contains("cancelled") {
            "cancelled"
        } else if lower.contains("timed out waiting for the browser") {
            "sign_in_timeout"
        } else if lower.contains("no note creator") || lower.contains("no medium account") {
            "account_not_found"
        } else if lower.contains("does not look like")
            || lower.contains("does not contain a json array")
            || lower.contains("has no posts")
        {
            "invalid_input"
        } else if lower.contains("jwt") || lower.contains("401") || lower.contains("not signed in")
        {
            "session_expired"
        } else if e.chain().any(|c| {
            c.downcast_ref::<reqwest::Error>()
                .is_some_and(|r| r.is_connect() || r.is_timeout())
        }) || lower.contains("reaching ")
            || lower.contains("dns")
        {
            "network"
        } else {
            "unknown"
        };
        Self { code, detail }
    }
}
