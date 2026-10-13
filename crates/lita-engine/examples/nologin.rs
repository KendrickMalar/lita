//! Reproduces the "not logged in" failure without touching the network.
//!
//! Points Codex at an empty CODEX_HOME so there are no credentials, then
//! asks for a run. Expected: `FailureKind::NotLoggedIn` within milliseconds,
//! before `codex exec` is spawned. Run with `cargo run -p lita-codex --example nologin`.

use lita_codex::{CodexCli, FailureKind, Request, RunFailure};
use std::time::Instant;

fn main() {
    let home = tempfile::tempdir().expect("tempdir");
    // SAFETY: single-threaded example; set before any child is spawned.
    unsafe { std::env::set_var("CODEX_HOME", home.path()) };

    let cli = CodexCli::on_path();
    let req = Request {
        prompt: "say hi".into(),
        schema: serde_json::json!({"type":"object","properties":{"text":{"type":"string"}},"required":["text"],"additionalProperties":false}),
        model: None,
        effort: Default::default(),
        working_dir: home.path().to_path_buf(),
    };
    let started = Instant::now();
    let result = cli.run_typed::<serde_json::Value>(&req, |_| {});
    let elapsed = started.elapsed();
    match result {
        Err(e) => match e.downcast_ref::<RunFailure>() {
            Some(f) if f.kind == FailureKind::NotLoggedIn => println!("ok: NotLoggedIn in {elapsed:?} (message: {:?})", f.message),
            Some(f) => { println!("unexpected kind {:?} in {elapsed:?}: {f}", f.kind); std::process::exit(1) }
            None => { println!("unexpected error: {e:#}"); std::process::exit(1) }
        },
        Ok(_) => { println!("unexpected success"); std::process::exit(1) }
    }
}
