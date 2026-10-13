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

#[cfg(test)]
mod tests {
    use super::*;
    use lita_engine::EngineKind;

    fn failure(engine: EngineKind, kind: FailureKind) -> RunFailure {
        RunFailure {
            engine,
            kind,
            exit_code: Some(1),
            message: Some("the CLI said so".to_string()),
            stderr_tail: "last words".to_string(),
        }
    }

    /// Every (engine, kind) pair and the code the UI must receive for it.
    fn expected_codes() -> [(EngineKind, FailureKind, &'static str); 10] {
        [
            (
                EngineKind::Codex,
                FailureKind::NotLoggedIn,
                "codex_not_logged_in",
            ),
            (
                EngineKind::Codex,
                FailureKind::QuotaExhausted,
                "codex_quota",
            ),
            (
                EngineKind::Codex,
                FailureKind::SchemaMismatch,
                "codex_schema",
            ),
            (EngineKind::Codex, FailureKind::Unknown, "codex_failed"),
            (EngineKind::Codex, FailureKind::Cancelled, "cancelled"),
            (
                EngineKind::Claude,
                FailureKind::NotLoggedIn,
                "claude_not_logged_in",
            ),
            (
                EngineKind::Claude,
                FailureKind::QuotaExhausted,
                "claude_quota",
            ),
            (
                EngineKind::Claude,
                FailureKind::SchemaMismatch,
                "claude_schema",
            ),
            (EngineKind::Claude, FailureKind::Unknown, "claude_failed"),
            (EngineKind::Claude, FailureKind::Cancelled, "cancelled"),
        ]
    }

    fn assert_codes_for(engine: EngineKind) {
        let wrong: Vec<String> = expected_codes()
            .into_iter()
            .filter(|(e, _, _)| *e == engine)
            .filter_map(|(e, kind, want)| {
                let got = UiError::from(anyhow::Error::new(failure(e, kind))).code;
                (got != want).then(|| format!("{e:?}/{kind:?}: want {want}, got {got}"))
            })
            .collect();
        assert!(wrong.is_empty(), "wrong codes: {wrong:#?}");
    }

    #[test]
    fn codex_run_failures_keep_their_codes() {
        assert_codes_for(EngineKind::Codex);
    }

    #[test]
    fn claude_run_failures_get_claude_codes() {
        assert_codes_for(EngineKind::Claude);
    }

    #[test]
    fn claude_not_logged_in_code() {
        let ui = UiError::from(anyhow::Error::new(failure(
            EngineKind::Claude,
            FailureKind::NotLoggedIn,
        )));
        assert_eq!(ui.code, "claude_not_logged_in");
    }

    #[test]
    fn claude_quota_code() {
        let ui = UiError::from(anyhow::Error::new(failure(
            EngineKind::Claude,
            FailureKind::QuotaExhausted,
        )));
        assert_eq!(ui.code, "claude_quota");
    }

    #[test]
    fn claude_schema_code() {
        let ui = UiError::from(anyhow::Error::new(failure(
            EngineKind::Claude,
            FailureKind::SchemaMismatch,
        )));
        assert_eq!(ui.code, "claude_schema");
    }

    #[test]
    fn claude_unknown_code() {
        let ui = UiError::from(anyhow::Error::new(failure(
            EngineKind::Claude,
            FailureKind::Unknown,
        )));
        assert_eq!(ui.code, "claude_failed");
    }

    #[test]
    fn cancelled_is_the_same_for_both_engines() {
        for engine in [EngineKind::Codex, EngineKind::Claude] {
            let ui = UiError::from(anyhow::Error::new(failure(engine, FailureKind::Cancelled)));
            assert_eq!(ui.code, "cancelled", "engine {engine:?}");
        }
    }

    #[test]
    fn run_failure_wrapped_in_context_keeps_its_code() {
        let ui = UiError::from(
            anyhow::Error::new(failure(EngineKind::Claude, FailureKind::QuotaExhausted))
                .context("writing the draft"),
        );
        assert_eq!(ui.code, "claude_quota");
        assert!(ui.detail.contains("writing the draft"), "{}", ui.detail);
    }
}
