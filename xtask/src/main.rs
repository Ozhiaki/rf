//! Contract guardrail for the crate documentation tree.
//!
//! Entry points, invoked through the repo-root cargo alias:
//!
//!   cargo xtask check      assert every registered tier-P claim still equals
//!                          its field in the committed release-candidate fixture,
//!                          and that every GENERATED block on a crate surface has
//!                          a known id (an unknown id has no renderer to keep it
//!                          fresh and is rejected).
//!   cargo xtask capture    re-run `rf capabilities --json` under a frozen source
//!                          epoch and rewrite the committed fixture, then re-render
//!                          the README recovery example from the same binary so the
//!                          doc's one live example stays a real capture.
//!   cargo xtask capture --check
//!                          re-capture into memory and assert the committed
//!                          fixture is not stale (the pre-package CI gate),
//!                          without rewriting it.
//!   cargo xtask preflight  the publish stop-sign: run every gate that must hold
//!                          at the moment of `cargo publish`, and refuse (non-zero
//!                          exit) unless all pass. crates.io is write-once, so this
//!                          is the last point at which docs and code can be forced
//!                          into lock step. Gates: (1) the worktree is clean and
//!                          committed; (2) the committed fixture matches a fresh
//!                          capture from the binary being packaged; (3) the docs
//!                          match the fixture with no unknown GENERATED block; (4)
//!                          the packaged file list stays within the include
//!                          allowlist, so no guard or config path enters the crate;
//!                          (5) the README recovery example matches a fresh render
//!                          from the binary, so the one live example is never stale.
//!
//! The registry (tests/fixtures/contract/coverage-registry.json) is the machine-
//! readable successor to the prose coverage-matrix. Interpretive prose is not
//! registered and is not checked: it is human-review by construction. Only the
//! crate half lives here; the site half is a Node port sharing the same registry
//! schema and GENERATED-marker format.

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

const REGISTRY: &str = "tests/fixtures/contract/coverage-registry.json";
// Crate documentation surfaces scanned for stray GENERATED markers.
const SURFACES: &[&str] = &["README.md", "CHANGELOG.md"];
// GENERATED-block ids the crate tree is allowed to carry. Each must have a
// renderer that keeps it fresh (see `capture`); the stray-block scan rejects
// any id not on this list, so a block can never go stale with no renderer.
const KNOWN_GENERATED: &[&str] = &["readme-example"];

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().map(String::as_str).unwrap_or("");
    let result = match cmd {
        "check" => cmd_check(),
        "capture" => cmd_capture(args.get(1).map(String::as_str) == Some("--check")),
        "preflight" => cmd_preflight(),
        _ => {
            eprintln!("usage: cargo xtask <check | capture [--check] | preflight>");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("xtask {cmd}: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Repo root is the parent of this xtask member's manifest dir. `XTASK_ROOT`
/// overrides it so the seeded-drift tests can point the guard at a temp tree.
fn repo_root() -> PathBuf {
    if let Some(root) = std::env::var_os("XTASK_ROOT") {
        return PathBuf::from(root);
    }
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest.parent().map(Path::to_path_buf).unwrap_or(manifest)
}

fn read_json(path: &Path) -> Result<Value, String> {
    let text =
        std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("parse {}: {e}", path.display()))
}

// ---- check -----------------------------------------------------------------

fn cmd_check() -> Result<(), String> {
    let n = check_docs_against_fixture()?;
    println!("contract-guard: {n} registered claim(s) match the committed fixture");
    Ok(())
}

/// Core of `check`, without printing: assert every registered claim equals its
/// fixture field and no surface carries a stray GENERATED block. Returns the
/// number of claims checked. Shared by `cmd_check` and `cmd_preflight`.
fn check_docs_against_fixture() -> Result<usize, String> {
    let root = repo_root();
    let registry = read_json(&root.join(REGISTRY))?;
    let fixture_rel = registry["fixture"]
        .as_str()
        .ok_or("registry: missing string field `fixture`")?;
    let fixture = read_json(&root.join(fixture_rel))?;
    let claims = registry["claims"]
        .as_array()
        .ok_or("registry: `claims` is not an array")?;

    let mut failures: Vec<String> = Vec::new();

    for claim in claims {
        let id = claim["id"].as_str().unwrap_or("<unnamed>");
        let path = claim["fixture_path"].as_str().unwrap_or("");
        let mode = claim["mode"].as_str().unwrap_or("value");
        let expected = &claim["expected"];

        let resolved = match resolve(&fixture, path) {
            Ok(v) => v,
            Err(e) => {
                failures.push(format!("[{id}] fixture path `{path}` did not resolve: {e}"));
                continue;
            }
        };

        let (actual_repr, ok) = match mode {
            "value" => (json_compact(resolved), resolved == expected),
            "keys" => {
                let got = object_keys_sorted(resolved);
                match got {
                    Ok(keys) => {
                        let want = string_vec(expected);
                        let repr = json_list(&keys);
                        (repr, Some(keys) == want)
                    }
                    Err(e) => {
                        failures.push(format!("[{id}] mode=keys: {e}"));
                        continue;
                    }
                }
            }
            "set" => {
                let got = array_strings_sorted(resolved);
                match got {
                    Ok(vals) => {
                        let want = string_vec(expected);
                        let repr = json_list(&vals);
                        (repr, Some(vals) == want)
                    }
                    Err(e) => {
                        failures.push(format!("[{id}] mode=set: {e}"));
                        continue;
                    }
                }
            }
            other => {
                failures.push(format!("[{id}] unknown mode `{other}`"));
                continue;
            }
        };

        if !ok {
            failures.push(format!(
                "[{id}] drift: expected {}, fixture has {}",
                json_compact(expected),
                actual_repr
            ));
        }
    }

    // Every GENERATED block on a crate surface must have a known id, so it has a
    // renderer that keeps it fresh. An unknown id is a block with nothing to
    // regenerate it — reject it rather than let it go silently stale.
    for surface in SURFACES {
        let p = root.join(surface);
        if !p.exists() {
            continue;
        }
        let text = std::fs::read_to_string(&p).map_err(|e| format!("read {surface}: {e}"))?;
        for id in generated_ids(&text) {
            if !KNOWN_GENERATED.contains(&id.as_str()) {
                failures.push(format!(
                    "[{surface}] GENERATED block `{id}` has no renderer; add one and register \
                     the id in KNOWN_GENERATED, or remove the block"
                ));
            }
        }
    }

    if failures.is_empty() {
        Ok(claims.len())
    } else {
        Err(format!(
            "{} claim(s) failed:\n  {}",
            failures.len(),
            failures.join("\n  ")
        ))
    }
}

// ---- capture ---------------------------------------------------------------

fn cmd_capture(check_only: bool) -> Result<(), String> {
    if check_only {
        fixture_matches_binary()?;
        println!("contract-guard: committed fixture is current (contract-equivalent to a fresh capture)");
        return Ok(());
    }
    let root = repo_root();
    let registry = read_json(&root.join(REGISTRY))?;
    let fixture_rel = registry["fixture"]
        .as_str()
        .ok_or("registry: missing string field `fixture`")?;
    let fixture_path = root.join(fixture_rel);
    let captured = capture_from_binary(&root)?;
    let mut text =
        serde_json::to_string_pretty(&captured).map_err(|e| format!("serialize: {e}"))?;
    text.push('\n');
    std::fs::write(&fixture_path, text).map_err(|e| format!("write {fixture_rel}: {e}"))?;
    println!("contract-guard: rewrote {fixture_rel} from a fresh capture");

    // Regenerate the README recovery example from the same binary, so the doc's
    // one live example stays a real capture and cannot drift from the fixture.
    let readme_path = root.join("README.md");
    let readme = std::fs::read_to_string(&readme_path).map_err(|e| format!("read README.md: {e}"))?;
    let block = render_readme_example(&root)?;
    let updated = replace_generated(&readme, "readme-example", &block)?;
    if updated != readme {
        std::fs::write(&readme_path, updated).map_err(|e| format!("write README.md: {e}"))?;
        println!("contract-guard: regenerated the README readme-example block");
    } else {
        println!("contract-guard: README readme-example block already current");
    }
    Ok(())
}

/// Build the `rf` binary and run `rf capabilities --json` under a frozen source
/// epoch (so ts_iso and elapsed_ms are deterministic; request_id and data_hash
/// are content-derived). Returns the captured envelope.
fn capture_from_binary(root: &Path) -> Result<Value, String> {
    build_rf(root)?;
    let bin = root.join("target/debug/rf");
    let out = Command::new(&bin)
        .args(["capabilities", "--json"])
        .env("SOURCE_DATE_EPOCH", "0")
        .current_dir(root)
        .output()
        .map_err(|e| format!("run {}: {e}", bin.display()))?;
    if !out.status.success() {
        return Err(format!(
            "rf capabilities exited {}",
            out.status.code().unwrap_or(-1)
        ));
    }
    serde_json::from_slice(&out.stdout)
        .map_err(|e| format!("captured output is not valid JSON: {e}"))
}

/// Build the `rf` binary once, in the debug profile the guard runs against.
/// Shared by every gate that needs a fresh binary.
fn build_rf(root: &Path) -> Result<(), String> {
    let status = Command::new(env!("CARGO"))
        .args(["build", "--quiet", "--bin", "rf"])
        .current_dir(root)
        .status()
        .map_err(|e| format!("cargo build rf: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err("cargo build rf failed".into())
    }
}

// ---- README recovery example ----------------------------------------------

/// Render the README recovery example from a real run of the binary, so the
/// documented output is a capture and not hand-typed. Builds `rf`, lays down the
/// three-file recovery case (a tracked file, a hidden dotfile, a gitignored
/// file) in a throwaway git tree, runs `rf content timeout . --human` under a
/// frozen epoch, and returns the full fenced block for the readme-example
/// markers (the `$ rf content timeout .` prompt line plus the render).
fn render_readme_example(root: &Path) -> Result<String, String> {
    build_rf(root)?;
    let bin = root.join("target/debug/rf");
    let tree = make_sample_tree()?;
    let result = Command::new(&bin)
        .args(["content", "timeout", ".", "--human"])
        .env("SOURCE_DATE_EPOCH", "0")
        .env("NO_COLOR", "1")
        .current_dir(&tree)
        .output()
        .map_err(|e| format!("run rf content: {e}"));
    std::fs::remove_dir_all(&tree).ok();
    let out = result?;
    if !out.status.success() {
        return Err(format!(
            "rf content exited {}",
            out.status.code().unwrap_or(-1)
        ));
    }
    let rendered = String::from_utf8(out.stdout).map_err(|e| format!("rf output not UTF-8: {e}"))?;
    let body = rendered.trim_end_matches('\n');
    Ok(format!("```\n$ rf content timeout .\n{body}\n```"))
}

/// Lay down the README's three-file recovery case in a fresh temp dir, inside a
/// git repo so the vcs_ignore classification is live. Returns the tree root; the
/// caller removes it.
fn make_sample_tree() -> Result<PathBuf, String> {
    let dir = std::env::temp_dir().join(format!(
        "rf-readme-example-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(dir.join("cache")).map_err(|e| format!("mkdir sample tree: {e}"))?;
    let init = Command::new("git")
        .args(["init", "-q"])
        .current_dir(&dir)
        .status()
        .map_err(|e| format!("git init: {e}"))?;
    if !init.success() {
        std::fs::remove_dir_all(&dir).ok();
        return Err("git init failed in sample tree".into());
    }
    let files = [
        ("config.py", "timeout = 30\n"),
        (".env.local", "timeout = 5      # hidden\n"),
        ("cache/build.py", "timeout = 999    # gitignored\n"),
        (".gitignore", "cache/\n"),
    ];
    for (name, contents) in files {
        std::fs::write(dir.join(name), contents).map_err(|e| format!("write {name}: {e}"))?;
    }
    Ok(dir)
}

// ---- GENERATED-marker helpers ----------------------------------------------

fn begin_marker(id: &str) -> String {
    format!("<!-- BEGIN GENERATED:{id} -->")
}

fn end_marker(id: &str) -> String {
    format!("<!-- END GENERATED:{id} -->")
}

/// Return the body between the BEGIN/END markers for `id`, excluding the marker
/// lines and the single newline bounding the body on each side. None if the
/// block is absent or malformed.
fn extract_generated<'a>(text: &'a str, id: &str) -> Option<&'a str> {
    let begin = begin_marker(id);
    let end = end_marker(id);
    let after_begin = text.find(&begin)? + begin.len();
    let body_start = after_begin + text[after_begin..].starts_with('\n').then_some(1)?;
    let estart = text[body_start..].find(&end)? + body_start;
    let body_end = text[..estart].strip_suffix('\n')?.len();
    Some(&text[body_start..body_end])
}

/// Replace the body between the markers for `id` with `body`, preserving the
/// marker lines. Errors if either marker is absent.
fn replace_generated(text: &str, id: &str, body: &str) -> Result<String, String> {
    let begin = begin_marker(id);
    let end = end_marker(id);
    let after_begin =
        text.find(&begin).ok_or_else(|| format!("BEGIN marker for `{id}` not found"))? + begin.len();
    let estart = text[after_begin..]
        .find(&end)
        .ok_or_else(|| format!("END marker for `{id}` not found"))?
        + after_begin;
    Ok(format!("{}\n{body}\n{}", &text[..after_begin], &text[estart..]))
}

/// Every GENERATED-block id present in `text`, in order of appearance.
fn generated_ids(text: &str) -> Vec<String> {
    const NEEDLE: &str = "BEGIN GENERATED:";
    let mut ids = Vec::new();
    let mut rest = text;
    while let Some(i) = rest.find(NEEDLE) {
        let after = &rest[i + NEEDLE.len()..];
        let id = after
            .split([' ', '\n', '\r', '\t'])
            .next()
            .unwrap_or("");
        if !id.is_empty() {
            ids.push(id.to_string());
        }
        rest = after;
    }
    ids
}

/// Assert the committed fixture is contract-equivalent to a fresh capture from
/// the binary being packaged. Non-printing; shared by `capture --check` and
/// `preflight`.
fn fixture_matches_binary() -> Result<(), String> {
    let root = repo_root();
    let registry = read_json(&root.join(REGISTRY))?;
    let fixture_rel = registry["fixture"]
        .as_str()
        .ok_or("registry: missing string field `fixture`")?;
    let captured = capture_from_binary(&root)?;
    let committed = read_json(&root.join(fixture_rel))?;
    let norm_fields: Vec<String> = registry["normalize_meta"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    if normalized(&captured, &norm_fields) == normalized(&committed, &norm_fields) {
        Ok(())
    } else {
        Err(format!(
            "committed fixture {fixture_rel} is STALE: a fresh capture differs. \
             Run `cargo xtask capture` and reconcile the docs."
        ))
    }
}

// ---- preflight (the publish stop-sign) -------------------------------------

fn cmd_preflight() -> Result<(), String> {
    println!("preflight: publish gate for crate `rf` (crates.io is write-once)");
    let mut failures = 0usize;
    let mut step = 0usize;
    let mut report = |label: &str, res: Result<String, String>| {
        step += 1;
        match res {
            Ok(note) => println!("  [{step}/5] PASS  {label} — {note}"),
            Err(e) => {
                failures += 1;
                println!("  [{step}/5] FAIL  {label}");
                for line in e.lines() {
                    println!("            {line}");
                }
            }
        }
    };

    // Run every gate (do not stop at the first failure) so one run surfaces
    // everything that must be fixed before publishing.
    report("worktree clean and committed", gate_worktree_clean());
    report(
        "committed fixture matches the binary",
        fixture_matches_binary().map(|()| "a fresh capture equals the committed fixture".into()),
    );
    report(
        "docs match the fixture (no stray blocks)",
        check_docs_against_fixture().map(|n| format!("{n} claim(s) match; no stray GENERATED block")),
    );
    report(
        "packaged files within the include allowlist",
        packaged_within_allowlist(),
    );
    report(
        "README example matches the binary",
        readme_example_fresh(),
    );

    if failures == 0 {
        println!("preflight: OK — every gate passed; safe to `cargo publish`");
        Ok(())
    } else {
        Err(format!(
            "{failures} gate(s) failed; do NOT `cargo publish` until each is green"
        ))
    }
}

/// Gate: the git worktree has no uncommitted changes. `cargo publish` packages
/// the working tree, so an untidy tree could ship files that were never
/// committed or reviewed.
fn gate_worktree_clean() -> Result<String, String> {
    let root = repo_root();
    let out = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(&root)
        .output()
        .map_err(|e| format!("run git status: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "git status exited {}: {}",
            out.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let listing = String::from_utf8_lossy(&out.stdout);
    let dirty: Vec<&str> = listing.lines().filter(|l| !l.trim().is_empty()).collect();
    if dirty.is_empty() {
        Ok("no uncommitted changes".into())
    } else {
        Err(format!(
            "{} uncommitted path(s); commit or stash before publishing:\n{}",
            dirty.len(),
            dirty.join("\n")
        ))
    }
}

/// Gate: every file `cargo package` would ship stays within the crate's include
/// allowlist, so no guard tool, cargo config, test fixture, or CI file leaks
/// into the published archive.
fn packaged_within_allowlist() -> Result<String, String> {
    let root = repo_root();
    let out = Command::new(env!("CARGO"))
        .args(["package", "--list", "--quiet"])
        .current_dir(&root)
        .output()
        .map_err(|e| format!("run cargo package --list: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "cargo package --list failed:\n{}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let listing = String::from_utf8_lossy(&out.stdout);
    let mut count = 0usize;
    let mut stray: Vec<String> = Vec::new();
    for f in listing.lines().map(str::trim).filter(|l| !l.is_empty()) {
        count += 1;
        if !path_is_allowed(f) {
            stray.push(f.to_string());
        }
    }
    if stray.is_empty() {
        Ok(format!("{count} file(s), all within the allowlist"))
    } else {
        Err(format!(
            "{} packaged file(s) outside the allowlist (guard/config must not ship):\n{}",
            stray.len(),
            stray.join("\n")
        ))
    }
}

/// A packaged path is allowed if it is one of cargo's own generated metadata
/// files, one of the top-level allowlisted docs, or a Rust source under `src/`.
/// Mirrors the `include` list in Cargo.toml.
fn path_is_allowed(p: &str) -> bool {
    const CARGO_META: &[&str] = &[
        "Cargo.toml",
        "Cargo.toml.orig",
        "Cargo.lock",
        ".cargo_vcs_info.json",
    ];
    if CARGO_META.contains(&p) || matches!(p, "README.md" | "CHANGELOG.md" | "LICENSE") {
        return true;
    }
    match p.strip_prefix("src/") {
        Some(rest) => !rest.is_empty() && rest.ends_with(".rs"),
        None => false,
    }
}

/// Gate: the README recovery example equals a fresh render from the binary.
/// crates.io ships the README verbatim and write-once, so an example that no
/// longer matches real output would mislead every reader of the published page.
fn readme_example_fresh() -> Result<String, String> {
    let root = repo_root();
    let readme = root.join("README.md");
    let text = std::fs::read_to_string(&readme).map_err(|e| format!("read README.md: {e}"))?;
    let committed = extract_generated(&text, "readme-example")
        .ok_or("README.md has no readme-example GENERATED block")?;
    let fresh = render_readme_example(&root)?;
    if committed == fresh {
        Ok("the documented example equals a fresh run".into())
    } else {
        Err(format!(
            "README example is STALE; run `cargo xtask capture` to regenerate it.\n\
             --- committed ---\n{committed}\n--- fresh ---\n{fresh}"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{extract_generated, generated_ids, path_is_allowed, replace_generated};

    #[test]
    fn allowlist_admits_only_sources_docs_and_cargo_meta() {
        for ok in [
            "Cargo.toml",
            "Cargo.toml.orig",
            "Cargo.lock",
            ".cargo_vcs_info.json",
            "README.md",
            "CHANGELOG.md",
            "LICENSE",
            "src/main.rs",
            "src/verbs/find.rs",
        ] {
            assert!(path_is_allowed(ok), "should be allowed: {ok}");
        }
        for bad in [
            "xtask/src/main.rs",
            "xtask/Cargo.toml",
            ".cargo/config.toml",
            ".github/workflows/contract-guard.yml",
            "tests/fixtures/contract/capabilities.rc.json",
            "src/",
            "src/notes.txt",
            "deploy/build-and-deploy.sh",
        ] {
            assert!(!path_is_allowed(bad), "should be rejected: {bad}");
        }
    }

    const SAMPLE: &str =
        "pre\n<!-- BEGIN GENERATED:readme-example -->\nold body\nline2\n<!-- END GENERATED:readme-example -->\npost\n";

    #[test]
    fn extract_returns_body_without_marker_lines() {
        assert_eq!(extract_generated(SAMPLE, "readme-example"), Some("old body\nline2"));
    }

    #[test]
    fn extract_absent_block_is_none() {
        assert_eq!(extract_generated("no markers here", "readme-example"), None);
    }

    #[test]
    fn replace_preserves_surroundings_and_round_trips() {
        let updated = replace_generated(SAMPLE, "readme-example", "new body\nnew line2").unwrap();
        assert!(updated.starts_with("pre\n"));
        assert!(updated.ends_with("post\n"));
        assert_eq!(
            extract_generated(&updated, "readme-example"),
            Some("new body\nnew line2")
        );
    }

    #[test]
    fn replace_errors_when_marker_absent() {
        assert!(replace_generated("no markers", "readme-example", "x").is_err());
    }

    #[test]
    fn generated_ids_lists_hyphenated_ids_in_order() {
        let text = "<!-- BEGIN GENERATED:readme-example -->\nx\n<!-- END GENERATED:readme-example -->\n\
                    <!-- BEGIN GENERATED:exit-codes-table -->\ny\n<!-- END GENERATED:exit-codes-table -->\n";
        assert_eq!(
            generated_ids(text),
            vec!["readme-example".to_string(), "exit-codes-table".to_string()]
        );
    }
}

/// Drop volatile meta fields so two captures compare on contract content only.
fn normalized(doc: &Value, fields: &[String]) -> Value {
    let mut d = doc.clone();
    if let Some(meta) = d.get_mut("meta").and_then(Value::as_object_mut) {
        for f in fields {
            meta.remove(f);
        }
    }
    d
}

// ---- fixture-path resolver -------------------------------------------------

/// Resolve a dotted path over the fixture. An empty path is the root. A numeric
/// segment indexes an array; any other segment is an object key (keys that
/// themselves contain no dot, e.g. `data[]`, resolve as a single segment).
fn resolve<'a>(root: &'a Value, path: &str) -> Result<&'a Value, String> {
    if path.is_empty() {
        return Ok(root);
    }
    let mut cur = root;
    for seg in path.split('.') {
        cur = match cur {
            Value::Array(items) => {
                let idx: usize = seg
                    .parse()
                    .map_err(|_| format!("segment `{seg}` is not an array index"))?;
                items
                    .get(idx)
                    .ok_or_else(|| format!("index {idx} out of range"))?
            }
            Value::Object(map) => map
                .get(seg)
                .ok_or_else(|| format!("key `{seg}` absent"))?,
            _ => return Err(format!("cannot descend into scalar at `{seg}`")),
        };
    }
    Ok(cur)
}

fn object_keys_sorted(v: &Value) -> Result<Vec<String>, String> {
    let obj = v.as_object().ok_or("resolved value is not an object")?;
    let mut keys: Vec<String> = obj.keys().cloned().collect();
    keys.sort();
    Ok(keys)
}

fn array_strings_sorted(v: &Value) -> Result<Vec<String>, String> {
    let arr = v.as_array().ok_or("resolved value is not an array")?;
    let mut out = Vec::with_capacity(arr.len());
    for item in arr {
        out.push(
            item.as_str()
                .ok_or("array element is not a string")?
                .to_string(),
        );
    }
    out.sort();
    Ok(out)
}

/// Expected-side helper: read a JSON array of strings, sorted, for keys/set modes.
fn string_vec(expected: &Value) -> Option<Vec<String>> {
    let arr = expected.as_array()?;
    let mut out = Vec::with_capacity(arr.len());
    for item in arr {
        out.push(item.as_str()?.to_string());
    }
    out.sort();
    Some(out)
}

fn json_compact(v: &Value) -> String {
    serde_json::to_string(v).unwrap_or_else(|_| "<unserializable>".into())
}

fn json_list(v: &[String]) -> String {
    json_compact(&Value::from(v.to_vec()))
}
