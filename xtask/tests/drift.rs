//! Seeded-drift tests for the crate contract guardrail.
//!
//! Each test builds a minimal temp documentation tree (registry + fixture +
//! empty surfaces), points the guard at it with `XTASK_ROOT`, and asserts the
//! guard passes on a reconciled tree and fails on a seeded drift. This is the
//! acceptance evidence for rf-guy.7: "the check fails on a seeded drift in each
//! registered fact and passes on a reconciled tree."

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    // tests/ -> xtask/ -> repo root
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

/// Lay down a self-contained tree the guard can check: the real registry and
/// fixture at their committed subpaths, plus empty crate surfaces.
fn scaffold(dir: &Path) {
    let src = repo_root();
    let contract = dir.join("tests/fixtures/contract");
    fs::create_dir_all(&contract).unwrap();
    for name in ["coverage-registry.json", "capabilities.rc.json"] {
        fs::copy(
            src.join("tests/fixtures/contract").join(name),
            contract.join(name),
        )
        .unwrap();
    }
    fs::write(dir.join("README.md"), "# rf\n\nprose only, no tables.\n").unwrap();
    fs::write(dir.join("CHANGELOG.md"), "# changelog\n").unwrap();
}

fn run_check(root: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_xtask"))
        .arg("check")
        .env("XTASK_ROOT", root)
        .output()
        .expect("run xtask check")
}

/// A unique temp dir under the target dir (no external tempfile dependency).
fn temp_tree(tag: &str) -> PathBuf {
    let base = std::env::temp_dir().join(format!(
        "rf-xtask-drift-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&base).unwrap();
    base
}

fn mutate_fixture(root: &Path, f: impl FnOnce(&mut serde_json::Value)) {
    let p = root.join("tests/fixtures/contract/capabilities.rc.json");
    let mut v: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&p).unwrap()).unwrap();
    f(&mut v);
    fs::write(&p, serde_json::to_string_pretty(&v).unwrap()).unwrap();
}

#[test]
fn passes_on_reconciled_tree() {
    let dir = temp_tree("clean");
    scaffold(&dir);
    let out = run_check(&dir);
    assert!(
        out.status.success(),
        "expected pass on reconciled tree, got:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn fails_on_drifted_enum() {
    let dir = temp_tree("enum");
    scaffold(&dir);
    // Drop a find.stage value -> the registered enum fact must no longer match.
    mutate_fixture(&dir, |v| {
        v["data"][0]["verbs"]["find"]["output_schema"]["data[]"]["stage"] =
            serde_json::json!("enum[found,fd_name]");
    });
    let out = run_check(&dir);
    assert!(!out.status.success(), "expected failure on drifted enum");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("find_stage_enum"), "diagnostic names claim: {err}");
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn fails_on_dropped_exit_code() {
    let dir = temp_tree("exit");
    scaffold(&dir);
    mutate_fixture(&dir, |v| {
        v["data"][0]["exit_codes"]
            .as_object_mut()
            .unwrap()
            .remove("5");
    });
    let out = run_check(&dir);
    assert!(!out.status.success(), "expected failure on dropped exit code");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("exit_code_set"), "diagnostic names claim: {err}");
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn fails_on_changed_warning_set() {
    let dir = temp_tree("warn");
    scaffold(&dir);
    mutate_fixture(&dir, |v| {
        v["data"][0]["warning_codes"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!("NEW_UNDOCUMENTED_CODE"));
    });
    let out = run_check(&dir);
    assert!(!out.status.success(), "expected failure on changed warning set");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("warning_code_set"), "diagnostic names claim: {err}");
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn fails_on_stray_generated_block() {
    let dir = temp_tree("block");
    scaffold(&dir);
    // The crate tree ships no GENERATED blocks; a stray one with no renderer
    // must be rejected rather than silently going stale.
    fs::write(
        dir.join("README.md"),
        "# rf\n\n<!-- BEGIN GENERATED:exit-codes-table -->\nstale\n<!-- END GENERATED:exit-codes-table -->\n",
    )
    .unwrap();
    let out = run_check(&dir);
    assert!(!out.status.success(), "expected failure on stray generated block");
    fs::remove_dir_all(&dir).ok();
}
