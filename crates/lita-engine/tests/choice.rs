//! The per-device engine choice: which CLI Lita drives, Codex or Claude Code.
//!
//! File format: `{"version":1,"engine":"codex"|"claude"}`. Anything that is not
//! exactly that (missing file, unreadable, not JSON, another version, an
//! unknown engine) reads as Codex, never as an error. Saving creates missing
//! parent directories and replaces the file through a temp file and a rename.

use std::fs;
use std::path::{Path, PathBuf};

use lita_engine::{load_choice, save_choice, EngineKind};
use serde_json::{json, Value};

fn choice_path(dir: &tempfile::TempDir) -> PathBuf {
    dir.path().join("engine.json")
}

fn write_raw(path: &Path, text: &str) {
    fs::write(path, text).unwrap();
}

fn entries(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn engine_kind_defaults_to_codex() {
    assert_eq!(EngineKind::default(), EngineKind::Codex);
}

#[test]
fn engine_kind_is_copy_and_eq() {
    fn assert_copy_eq<T: Copy + Eq>() {}
    assert_copy_eq::<EngineKind>();
    let a = EngineKind::Claude;
    let b = a; // Copy: `a` is still usable.
    assert_eq!(a, b);
    assert_ne!(EngineKind::Codex, EngineKind::Claude);
}

#[test]
fn engine_kind_display_names() {
    assert_eq!(EngineKind::Codex.display_name(), "Codex");
    assert_eq!(EngineKind::Claude.display_name(), "Claude Code");
}

#[test]
fn engine_kind_serializes_as_lowercase() {
    assert_eq!(serde_json::to_string(&EngineKind::Codex).unwrap(), "\"codex\"");
    assert_eq!(serde_json::to_string(&EngineKind::Claude).unwrap(), "\"claude\"");
    assert_eq!(serde_json::from_str::<EngineKind>("\"codex\"").unwrap(), EngineKind::Codex);
    assert_eq!(serde_json::from_str::<EngineKind>("\"claude\"").unwrap(), EngineKind::Claude);
    assert!(serde_json::from_str::<EngineKind>("\"Claude\"").is_err());
    assert!(serde_json::from_str::<EngineKind>("\"gemini\"").is_err());
}

#[test]
fn choice_unsaved_is_codex() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(load_choice(&choice_path(&dir)), EngineKind::Codex);
}

#[test]
fn choice_missing_parent_directory_is_codex() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(load_choice(&dir.path().join("no/such/dir/engine.json")), EngineKind::Codex);
}

#[test]
fn choice_unreadable_path_is_codex() {
    // A directory where the file should be: reading it fails.
    let dir = tempfile::tempdir().unwrap();
    let path = choice_path(&dir);
    fs::create_dir(&path).unwrap();
    assert_eq!(load_choice(&path), EngineKind::Codex);
}

#[test]
fn choice_empty_file_is_codex() {
    let dir = tempfile::tempdir().unwrap();
    let path = choice_path(&dir);
    write_raw(&path, "");
    assert_eq!(load_choice(&path), EngineKind::Codex);
}

#[test]
fn choice_broken_json_is_codex() {
    let dir = tempfile::tempdir().unwrap();
    let path = choice_path(&dir);
    for broken in ["{", "not json", "{\"version\":1,\"engine\":\"claude\"", "[1,2,3]", "\"claude\"", "null"] {
        write_raw(&path, broken);
        assert_eq!(load_choice(&path), EngineKind::Codex, "input: {broken}");
    }
}

#[test]
fn choice_unknown_engine_is_codex() {
    let dir = tempfile::tempdir().unwrap();
    let path = choice_path(&dir);
    for text in [
        r#"{"version":1,"engine":"gemini"}"#,
        r#"{"version":1,"engine":"Claude"}"#,
        r#"{"version":1,"engine":""}"#,
        r#"{"version":1,"engine":null}"#,
        r#"{"version":1,"engine":7}"#,
        r#"{"version":1}"#,
    ] {
        write_raw(&path, text);
        assert_eq!(load_choice(&path), EngineKind::Codex, "input: {text}");
    }
}

#[test]
fn choice_other_or_missing_version_is_codex() {
    let dir = tempfile::tempdir().unwrap();
    let path = choice_path(&dir);
    for text in [
        r#"{"version":2,"engine":"claude"}"#,
        r#"{"version":0,"engine":"claude"}"#,
        r#"{"version":"1","engine":"claude"}"#,
        r#"{"version":null,"engine":"claude"}"#,
        r#"{"engine":"claude"}"#,
    ] {
        write_raw(&path, text);
        assert_eq!(load_choice(&path), EngineKind::Codex, "input: {text}");
    }
}

#[test]
fn choice_valid_files_are_read() {
    let dir = tempfile::tempdir().unwrap();
    let path = choice_path(&dir);
    write_raw(&path, r#"{"version":1,"engine":"claude"}"#);
    assert_eq!(load_choice(&path), EngineKind::Claude);
    write_raw(&path, r#"{"version":1,"engine":"codex"}"#);
    assert_eq!(load_choice(&path), EngineKind::Codex);
}

#[test]
fn choice_saved_claude_is_read_back_and_stays_after_reloading() {
    let dir = tempfile::tempdir().unwrap();
    let path = choice_path(&dir);
    save_choice(&path, EngineKind::Claude).unwrap();
    assert_eq!(load_choice(&path), EngineKind::Claude);
    // Reading again (a reload) gives the same answer.
    assert_eq!(load_choice(&path), EngineKind::Claude);
}

#[test]
fn choice_can_be_overwritten_back_to_codex() {
    let dir = tempfile::tempdir().unwrap();
    let path = choice_path(&dir);
    save_choice(&path, EngineKind::Claude).unwrap();
    save_choice(&path, EngineKind::Codex).unwrap();
    assert_eq!(load_choice(&path), EngineKind::Codex);
    save_choice(&path, EngineKind::Claude).unwrap();
    assert_eq!(load_choice(&path), EngineKind::Claude);
}

#[test]
fn choice_save_replaces_a_broken_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = choice_path(&dir);
    write_raw(&path, "garbage");
    save_choice(&path, EngineKind::Claude).unwrap();
    assert_eq!(load_choice(&path), EngineKind::Claude);
}

#[test]
fn choice_save_creates_missing_parent_directories() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a/b/c/engine.json");
    assert!(!path.parent().unwrap().exists());
    save_choice(&path, EngineKind::Claude).unwrap();
    assert!(path.is_file());
    assert_eq!(load_choice(&path), EngineKind::Claude);
}

#[test]
fn choice_saved_file_has_the_documented_json_shape() {
    let dir = tempfile::tempdir().unwrap();
    let path = choice_path(&dir);
    save_choice(&path, EngineKind::Claude).unwrap();
    let value: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(value["version"], json!(1));
    assert_eq!(value["engine"], json!("claude"));
    assert_eq!(value, json!({"version":1,"engine":"claude"}));

    save_choice(&path, EngineKind::Codex).unwrap();
    let value: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(value, json!({"version":1,"engine":"codex"}));
}

#[test]
fn choice_save_leaves_no_temp_file_behind() {
    let dir = tempfile::tempdir().unwrap();
    let path = choice_path(&dir);
    save_choice(&path, EngineKind::Claude).unwrap();
    save_choice(&path, EngineKind::Codex).unwrap();
    assert_eq!(entries(dir.path()), vec!["engine.json".to_string()]);

    // Also when the parent directory had to be created.
    let nested = dir.path().join("nested/dir/engine.json");
    save_choice(&nested, EngineKind::Claude).unwrap();
    assert_eq!(entries(nested.parent().unwrap()), vec!["engine.json".to_string()]);
}

#[test]
fn choice_save_fails_when_the_parent_is_a_file() {
    let dir = tempfile::tempdir().unwrap();
    let blocker = dir.path().join("blocker");
    fs::write(&blocker, "x").unwrap();
    assert!(save_choice(&blocker.join("engine.json"), EngineKind::Claude).is_err());
    // A failed save does not leave anything else behind.
    assert_eq!(entries(dir.path()), vec!["blocker".to_string()]);
}
