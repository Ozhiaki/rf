//! Contract guardrail for the crate documentation tree.
//!
//! Two entry points, invoked through the repo-root cargo alias:
//!
//!   cargo xtask check      assert every registered tier-P claim still equals
//!                          its field in the committed release-candidate fixture,
//!                          and that no crate surface carries an unrendered
//!                          GENERATED block (the crate ships none by design).
//!   cargo xtask capture    re-run `rf capabilities --json` under a frozen source
//!                          epoch and rewrite the committed fixture.
//!   cargo xtask capture --check
//!                          re-capture into memory and assert the committed
//!                          fixture is not stale (the pre-package CI gate),
//!                          without rewriting it.
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

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().map(String::as_str).unwrap_or("");
    let result = match cmd {
        "check" => cmd_check(),
        "capture" => cmd_capture(args.get(1).map(String::as_str) == Some("--check")),
        _ => {
            eprintln!("usage: cargo xtask <check | capture [--check]>");
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

    // The crate documentation tree carries no GENERATED blocks: the README defers
    // every full contract table to `rf capabilities`. Assert that stays true, so
    // a stray unrendered block cannot slip in without a renderer to keep it fresh.
    for surface in SURFACES {
        let p = root.join(surface);
        if !p.exists() {
            continue;
        }
        let text = std::fs::read_to_string(&p).map_err(|e| format!("read {surface}: {e}"))?;
        if text.contains("BEGIN GENERATED:") {
            failures.push(format!(
                "[{surface}] contains a GENERATED block, but the crate tree has no renderers; \
                 move contract tables to the site tree or add a crate renderer"
            ));
        }
    }

    if failures.is_empty() {
        println!(
            "contract-guard: {} registered claim(s) match the committed fixture",
            claims.len()
        );
        Ok(())
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
    let root = repo_root();
    let registry = read_json(&root.join(REGISTRY))?;
    let fixture_rel = registry["fixture"]
        .as_str()
        .ok_or("registry: missing string field `fixture`")?;
    let fixture_path = root.join(fixture_rel);

    // Build the binary once, then run it under a frozen epoch so ts_iso and
    // elapsed_ms are deterministic; request_id and data_hash are content-derived.
    let status = Command::new(env!("CARGO"))
        .args(["build", "--quiet", "--bin", "rf"])
        .current_dir(&root)
        .status()
        .map_err(|e| format!("cargo build rf: {e}"))?;
    if !status.success() {
        return Err("cargo build rf failed".into());
    }
    let bin = root.join("target/debug/rf");
    let out = Command::new(&bin)
        .args(["capabilities", "--json"])
        .env("SOURCE_DATE_EPOCH", "0")
        .current_dir(&root)
        .output()
        .map_err(|e| format!("run {}: {e}", bin.display()))?;
    if !out.status.success() {
        return Err(format!(
            "rf capabilities exited {}",
            out.status.code().unwrap_or(-1)
        ));
    }
    let captured: Value = serde_json::from_slice(&out.stdout)
        .map_err(|e| format!("captured output is not valid JSON: {e}"))?;

    if check_only {
        let committed = read_json(&fixture_path)?;
        let norm_fields: Vec<String> = registry["normalize_meta"]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default();
        if normalized(&captured, &norm_fields) == normalized(&committed, &norm_fields) {
            println!("contract-guard: committed fixture is current (contract-equivalent to a fresh capture)");
            Ok(())
        } else {
            Err(format!(
                "committed fixture {fixture_rel} is STALE: a fresh capture differs. \
                 Run `cargo xtask capture` and reconcile the docs."
            ))
        }
    } else {
        let mut text = serde_json::to_string_pretty(&captured)
            .map_err(|e| format!("serialize: {e}"))?;
        text.push('\n');
        std::fs::write(&fixture_path, text)
            .map_err(|e| format!("write {fixture_rel}: {e}"))?;
        println!("contract-guard: rewrote {fixture_rel} from a fresh capture");
        Ok(())
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
