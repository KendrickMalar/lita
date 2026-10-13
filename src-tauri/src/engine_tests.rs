//! Tests for `crate::engine`: the per-device engine choice, its JSON shapes
//! for the UI, and the per-Article resolution the queue relies on.
//!
//! Nothing here starts a real `codex` or `claude`: preflight results are
//! injected through closures, and `resolve` only builds an `Engine`.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use lita_engine::{EngineKind, Preflight};
use lita_store::QueueState;
use serde_json::{Value, json};

use crate::engine::{
    CHOICE_FILE, EngineStatus, EnginesStatus, choice_path_in, queue_stop_error, resolve,
    resolve_for_queue, select, selected,
};
use crate::errors::UiError;
use crate::{queue_error_mark, stops_the_queue};

static NEXT_DIR: AtomicUsize = AtomicUsize::new(0);

/// A directory unique to one test, removed when the test ends.
struct TestDir {
    path: PathBuf,
}

impl TestDir {
    fn new() -> Self {
        let n = NEXT_DIR.fetch_add(1, Ordering::SeqCst);
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../target/test-tmp")
            .join(format!("engine-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&path).expect("create the test directory");
        Self { path }
    }

    fn choice_path(&self) -> PathBuf {
        choice_path_in(&self.path)
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn ready() -> Preflight {
    Preflight::Ready {
        version: "1.0".into(),
    }
}

fn not_logged_in() -> Preflight {
    Preflight::NotLoggedIn {
        version: "1.0".into(),
    }
}

fn config_broken(detail: &str) -> Preflight {
    Preflight::ConfigBroken {
        version: "1.0".into(),
        detail: detail.into(),
    }
}

// 1. Nothing saved, or something unreadable: Codex.

#[test]
fn engine_unsaved_choice_is_codex() {
    let dir = TestDir::new();
    let path = dir.choice_path();
    assert!(!path.exists());
    assert_eq!(selected(&path), EngineKind::Codex);
    assert_eq!(resolve(&path).kind(), EngineKind::Codex);
}

#[test]
fn engine_broken_choice_file_is_codex() {
    let dir = TestDir::new();
    let path = dir.choice_path();
    std::fs::write(&path, "{").expect("write a broken choice file");
    assert_eq!(selected(&path), EngineKind::Codex);
    assert_eq!(resolve(&path).kind(), EngineKind::Codex);
}

#[test]
fn engine_choice_path_is_engine_json_in_the_config_dir() {
    assert_eq!(CHOICE_FILE, "engine.json");
    let dir = Path::new("some").join("config");
    assert_eq!(choice_path_in(&dir), dir.join("engine.json"));
}

// 2. select / selected round trip and the saved file.

#[test]
fn engine_select_round_trips_and_shows_in_engines_status() {
    let dir = TestDir::new();
    let path = dir.choice_path();

    select(&path, EngineKind::Claude).expect("save Claude");
    assert_eq!(selected(&path), EngineKind::Claude);

    let status = EnginesStatus {
        selected: selected(&path),
        codex: EngineStatus::NotInstalled,
        claude: EngineStatus::from(ready()),
    };
    let value = serde_json::to_value(&status).expect("serialize");
    assert_eq!(value["selected"], json!("claude"));

    select(&path, EngineKind::Codex).expect("save Codex");
    assert_eq!(selected(&path), EngineKind::Codex);
}

#[test]
fn engine_select_writes_a_version_1_choice_file() {
    let dir = TestDir::new();
    let path = dir.choice_path();
    select(&path, EngineKind::Claude).expect("save Claude");
    let text = std::fs::read_to_string(&path).expect("read the saved file");
    let saved: Value = serde_json::from_str(&text).expect("the saved file is JSON");
    assert_eq!(saved, json!({ "version": 1, "engine": "claude" }));
}

// 3. Switching does not change an engine that was already resolved.

#[test]
fn engine_switch_leaves_a_resolved_engine_alone() {
    let dir = TestDir::new();
    let path = dir.choice_path();

    let first = resolve(&path);
    select(&path, EngineKind::Claude).expect("save Claude");
    let second = resolve(&path);

    assert_eq!(second.kind(), EngineKind::Claude);
    assert_eq!(first.kind(), EngineKind::Codex);
}

// 4. The JSON the UI receives.

#[test]
fn engine_status_from_preflight_has_the_ui_shapes() {
    assert_eq!(
        serde_json::to_value(EngineStatus::from(ready())).expect("serialize"),
        json!({ "status": "ready", "version": "1.0" })
    );
    assert_eq!(
        serde_json::to_value(EngineStatus::from(not_logged_in())).expect("serialize"),
        json!({ "status": "not_logged_in", "version": "1.0" })
    );
    assert_eq!(
        serde_json::to_value(EngineStatus::from(Preflight::NotInstalled)).expect("serialize"),
        json!({ "status": "not_installed" })
    );
    assert_eq!(
        serde_json::to_value(EngineStatus::from(config_broken("duplicate key")))
            .expect("serialize"),
        json!({ "status": "config_broken", "version": "1.0", "message": "duplicate key" })
    );
}

#[test]
fn engine_status_from_result_maps_an_error_to_the_error_shape() {
    let failed = EngineStatus::from_result(Err(anyhow::anyhow!("could not run")));
    assert_eq!(
        serde_json::to_value(failed).expect("serialize"),
        json!({ "status": "error", "message": "could not run" })
    );

    let chained = EngineStatus::from_result(Err(
        anyhow::anyhow!("spawn failed").context("preflight failed")
    ));
    assert_eq!(
        chained,
        EngineStatus::Error {
            message: "preflight failed: spawn failed".into()
        }
    );

    assert_eq!(
        EngineStatus::from_result(Ok(ready())),
        EngineStatus::Ready {
            version: "1.0".into()
        }
    );
}

#[test]
fn engine_engines_status_has_the_ui_shape() {
    let status = EnginesStatus {
        selected: EngineKind::Codex,
        codex: EngineStatus::NotInstalled,
        claude: EngineStatus::from(not_logged_in()),
    };
    assert_eq!(
        serde_json::to_value(status).expect("serialize"),
        json!({
            "selected": "codex",
            "codex": { "status": "not_installed" },
            "claude": { "status": "not_logged_in", "version": "1.0" },
        })
    );
}

// 5. The queue re-reads the choice for every Article.

#[test]
fn engine_queue_resolve_rereads_the_choice_each_call() {
    let dir = TestDir::new();
    let path = dir.choice_path();

    select(&path, EngineKind::Codex).expect("save Codex");
    let mut seen_first = None;
    let first = resolve_for_queue(&path, |engine| {
        seen_first = Some(engine.kind());
        Ok(ready())
    })
    .expect("Ready lets the queue go on");
    assert_eq!(seen_first, Some(EngineKind::Codex));
    assert_eq!(first.kind(), EngineKind::Codex);

    select(&path, EngineKind::Claude).expect("save Claude");
    let mut seen_second = None;
    let second = resolve_for_queue(&path, |engine| {
        seen_second = Some(engine.kind());
        Ok(ready())
    })
    .expect("Ready lets the queue go on");
    assert_eq!(seen_second, Some(EngineKind::Claude));
    assert_eq!(second.kind(), EngineKind::Claude);
}

// 6. What stops the queue, and the error it reports.

fn assert_stop(kind: EngineKind, preflight: Preflight, code: &str, detail: &str) {
    let error = queue_stop_error(kind, preflight.clone())
        .unwrap_or_else(|| panic!("{kind:?} / {preflight:?} should stop the queue"));
    assert_eq!(error.code, code, "{kind:?} / {preflight:?}");
    assert_eq!(error.detail, detail, "{kind:?} / {preflight:?}");
}

#[test]
fn engine_queue_stop_error_for_codex() {
    assert!(queue_stop_error(EngineKind::Codex, ready()).is_none());
    assert_stop(
        EngineKind::Codex,
        Preflight::NotInstalled,
        "codex_not_installed",
        "",
    );
    assert_stop(
        EngineKind::Codex,
        not_logged_in(),
        "codex_not_logged_in",
        "",
    );
    assert_stop(
        EngineKind::Codex,
        config_broken("duplicate key in config.toml"),
        "codex_config_broken",
        "duplicate key in config.toml",
    );
}

#[test]
fn engine_queue_stop_error_for_claude() {
    assert!(queue_stop_error(EngineKind::Claude, ready()).is_none());
    assert_stop(
        EngineKind::Claude,
        Preflight::NotInstalled,
        "claude_not_installed",
        "",
    );
    assert_stop(
        EngineKind::Claude,
        not_logged_in(),
        "claude_not_logged_in",
        "",
    );
}

#[test]
fn engine_queue_stop_error_for_claude_config_broken_is_unknown() {
    let error = queue_stop_error(EngineKind::Claude, config_broken("bad settings"))
        .expect("a broken Claude config should stop the queue");
    assert_eq!(error.code, "unknown");
}

#[test]
fn engine_queue_resolve_stops_on_claude_not_logged_in() {
    let dir = TestDir::new();
    let path = dir.choice_path();
    select(&path, EngineKind::Claude).expect("save Claude");

    let mut seen = None;
    let result = resolve_for_queue(&path, |engine| {
        seen = Some(engine.kind());
        Ok(not_logged_in())
    });
    assert_eq!(seen, Some(EngineKind::Claude));
    match result {
        Ok(_) => panic!("a Claude that is not logged in must stop the queue"),
        Err(error) => {
            assert_eq!(error.code, "claude_not_logged_in");
            assert_eq!(error.detail, "");
        }
    }
}

#[test]
fn engine_queue_resolve_stops_on_codex_config_broken_with_its_detail() {
    let dir = TestDir::new();
    let path = dir.choice_path();
    select(&path, EngineKind::Codex).expect("save Codex");

    let result = resolve_for_queue(&path, |_| Ok(config_broken("duplicate key `model`")));
    match result {
        Ok(_) => panic!("a broken Codex config must stop the queue"),
        Err(error) => {
            assert_eq!(error.code, "codex_config_broken");
            assert_eq!(error.detail, "duplicate key `model`");
        }
    }
}

#[test]
fn engine_queue_resolve_passes_a_preflight_failure_through_as_a_ui_error() {
    let dir = TestDir::new();
    let path = dir.choice_path();

    let expected = UiError::from(anyhow::anyhow!("preflight exploded"));
    let result = resolve_for_queue(&path, |_| Err(anyhow::anyhow!("preflight exploded")));
    match result {
        Ok(_) => panic!("a failed preflight must stop the queue"),
        Err(error) => {
            assert_eq!(error.code, expected.code);
            assert_eq!(error.detail, expected.detail);
            assert_eq!(error.detail, "preflight exploded");
        }
    }
}

// 7. The queue-level classification of Claude's error codes.

#[test]
fn engine_queue_stops_the_queue_for_claude_and_codex_codes() {
    for code in [
        "claude_not_logged_in",
        "claude_not_installed",
        "claude_quota",
        "codex_not_logged_in",
        "codex_not_installed",
        "codex_config_broken",
        "codex_quota",
    ] {
        assert!(stops_the_queue(code), "{code} should stop the queue");
    }
    for code in ["claude_schema", "claude_failed"] {
        assert!(!stops_the_queue(code), "{code} should not stop the queue");
    }
}

#[test]
fn engine_queue_error_mark_for_claude_codes() {
    let mark = |code: &'static str| {
        queue_error_mark(&UiError {
            code,
            detail: String::new(),
        })
    };
    assert_eq!(mark("claude_quota"), Some(QueueState::Waiting));
    assert_eq!(mark("claude_not_logged_in"), Some(QueueState::Waiting));
    assert_eq!(mark("claude_not_installed"), Some(QueueState::Waiting));
    assert_eq!(mark("claude_failed"), Some(QueueState::Failed));
    assert_eq!(mark("claude_schema"), Some(QueueState::Failed));
}
