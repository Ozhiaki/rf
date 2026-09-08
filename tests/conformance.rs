//! Conformance harness for the `rf` binary — the executable contract that
//! travels with the production artifact. Asserts the contract properties on the
//! Rust binary itself. `cargo test` builds `rf` and points CARGO_BIN_EXE_rf at it.
//!
//! It builds its own self-contained corpus in a temp dir (git-init'd so
//! .gitignore rules are active), so it depends on no external fixtures.

use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_rf");
const TOKEN: &str = "MAGIC_TOKEN_XYZ";
const KEYS: [&str; 7] = ["ok", "tool_version", "data", "meta", "warnings", "commands", "errors"];

/// A self-cleaning corpus that exercises every content layer once.
struct Corpus {
    path: PathBuf,
}

impl Corpus {
    fn new() -> Corpus {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("rf-conf-{}-{}", std::process::id(), nanos));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(path.join("src")).unwrap();

        let w = |rel: &str, bytes: &[u8]| std::fs::write(path.join(rel), bytes).unwrap();
        w("src/app.py", format!("print('{TOKEN}')\n").as_bytes()); // default
        w("secrets.env", format!("KEY={TOKEN}\n").as_bytes()); // vcs_ignore
        w(".gitignore", b"*.env\n");
        w(".hidden.txt", format!("{TOKEN}\n").as_bytes()); // hidden
        w("blob.dat", format!("pre\0{TOKEN}\n").as_bytes()); // binary (NUL before match)
        w("lower.txt", TOKEN.to_lowercase().as_bytes()); // case
        // encoding: UTF-16LE, no BOM. rg's UTF-8 assumption + NUL binary-detection
        // miss it entirely; only the forced-decoder probe surfaces it.
        let utf16le: Vec<u8> = format!("setting = {TOKEN}\n")
            .chars()
            .flat_map(|c| (c as u16).to_le_bytes())
            .collect();
        w("config_utf16.txt", &utf16le); // encoding_utf16
        w("decoy.txt", b"nothing to see\n"); // true negative

        // require_git defaults true: .gitignore only applies inside a work tree.
        Command::new("git")
            .args(["init", "-q"])
            .current_dir(&path)
            .status()
            .expect("git init");
        Corpus { path }
    }
}

impl Drop for Corpus {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// A self-cleaning corpus that plants one trap for every `find` stage. Task is
/// "find TOKEN in *.config". Needs two commits (for the git-history scrub) so it
/// builds its own git identity to stay CI-portable.
struct FindCorpus {
    path: PathBuf,
}

impl FindCorpus {
    fn new() -> FindCorpus {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("rf-find-{}-{}", std::process::id(), nanos));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(path.join("src")).unwrap();
        std::fs::create_dir_all(path.join("gen")).unwrap();

        let w = |rel: &str, bytes: &[u8]| std::fs::write(path.join(rel), bytes).unwrap();
        w("src/app.config", format!("[db]\nKEY={TOKEN}\n").as_bytes()); // found
        w("settings.conf", format!("KEY={TOKEN}\n").as_bytes()); // fd_name (ext .conf)
        w(".hidden.config", format!("{TOKEN}\n").as_bytes()); // fd_hidden
        w("gen/build.config", format!("{TOKEN}\n").as_bytes()); // fd_ignore
        w(".gitignore", b"gen/\n");
        w("bin.config", format!("head\0{TOKEN}\n").as_bytes()); // rg_binary (NUL)
        w("empty.config", b"[db]\nOTHER=1\n"); // true negative
        w("history.config", format!("[db]\nKEY={TOKEN}\n").as_bytes()); // -> git_deleted
        // db.connect($$$): a node with no fixed literal form; carries NO TOKEN so
        // it stays orthogonal to the config/TOKEN task.
        w("conn.py", b"import db\n\n\ndef get():\n    return db.connect(dsn, timeout=30)\n"); // ast_structural
        w("other.py", b"import db\n\n\ndef ping():\n    return db.ping()\n"); // ast true negative

        let git = |args: &[&str]| {
            Command::new("git")
                .args(["-c", "user.name=t", "-c", "user.email=t@t"])
                .args(args)
                .current_dir(&path)
                .output()
                .expect("git");
        };
        git(&["init", "-q"]);
        git(&["add", "-A"]);
        git(&["commit", "-q", "-m", "seed (history.config has the token)"]);
        // scrub the token from history.config: it now lives only in commit 1.
        w("history.config", b"[db]\n# rotated out\n");
        git(&["add", "-A"]);
        git(&["commit", "-q", "-m", "rotate token out of history.config"]);
        FindCorpus { path }
    }
}

impl Drop for FindCorpus {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// Run rf; return (exit_code, parsed_json_stdout, raw_stdout).
fn rf(args: &[&str], cwd: Option<&Path>, env: &[(&str, &str)]) -> (i32, Value, String) {
    let mut c = Command::new(BIN);
    c.args(args);
    if let Some(d) = cwd {
        c.current_dir(d);
    }
    for (k, v) in env {
        c.env(k, v);
    }
    let out = c.output().expect("spawn rf");
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let json = serde_json::from_str(&stdout).unwrap_or(Value::Null);
    (out.status.code().unwrap_or(-1), json, stdout)
}

#[test]
fn conformance() {
    let corpus = Corpus::new();
    let cd = Some(corpus.path.as_path());
    let mut results: Vec<(String, bool, String)> = Vec::new();
    let mut check = |name: &str, cond: bool, detail: String| {
        results.push((name.to_string(), cond, detail));
    };

    // --- envelope shape: all seven keys + contract_version on every verb ---
    let verbs: [&[&str]; 4] = [
        &["capabilities", "--json"],
        &["content", TOKEN, ".", "--json"],
        &["find", TOKEN, ".", "--name", "config", "--json"],
        &["doctor", ".", "--json"],
    ];
    for v in verbs {
        let (_, e, _) = rf(v, cd, &[]);
        let keys_ok = e.is_object() && KEYS.iter().all(|k| e.get(*k).is_some())
            && e.as_object().unwrap().len() == KEYS.len();
        check(&format!("envelope keys: {}", v[0]), keys_ok, format!("{:?}", e.as_object().map(|o| o.keys().cloned().collect::<Vec<_>>())));
        check(
            &format!("meta.contract_version: {}", v[0]),
            e["meta"]["contract_version"] == 1,
            String::new(),
        );
    }

    // --- exit-code dictionary ---
    check("exit 0 on empty result", rf(&["content", "NOPE_NONE", ".", "--json"], cd, &[]).0 == 0, String::new());
    check("exit 1 on usage error (missing arg)", rf(&["content"], cd, &[]).0 == 1, String::new());
    check("exit 1 on bad regex", rf(&["content", "(", ".", "--json"], cd, &[]).0 == 1, String::new());
    check("exit 1 on --structural without --lang",
          rf(&["find", TOKEN, ".", "--name", "config", "--structural", "x($$$)", "--json"], cd, &[]).0 == 1, String::new());

    // --- forensic correctness: 5 planted, 1 by default, each layer attributed ---
    let (code, e, _) = rf(&["content", TOKEN, ".", "--json"], cd, &[]);
    check("content: exit 0", code == 0, String::new());
    check("content: 6 matched", e["meta"]["matched_files"] == 6, format!("{}", e["meta"]));
    check("content: 1 by default", e["meta"]["default_matched_files"] == 1, format!("{}", e["meta"]));
    let by_file: BTreeMap<String, String> = e["data"]
        .as_array()
        .unwrap_or(&vec![])
        .iter()
        .map(|d| (d["file"].as_str().unwrap_or("").to_string(), d["surfaced_by"].as_str().unwrap_or("").to_string()))
        .collect();
    for (file, layer) in [
        ("src/app.py", "default"),
        ("secrets.env", "vcs_ignore"),
        (".hidden.txt", "hidden"),
        ("blob.dat", "binary"),
        ("lower.txt", "case"),
        ("config_utf16.txt", "encoding_utf16"),
    ] {
        check(&format!("content: {file} attributed to {layer}"),
              by_file.get(file).map(|s| s.as_str()) == Some(layer),
              format!("{:?}", by_file.get(file)));
    }
    // true negative must not be surfaced at all
    check("content: true negative not flagged", !by_file.contains_key("decoy.txt"), String::new());
    // each non-default layer + the encoding probe emits a warning + correction
    let (_, e2, _) = rf(&["content", TOKEN, ".", "--json"], cd, &[]);
    check("content: 5 filter warnings", e2["warnings"].as_array().map(|a| a.len()) == Some(5), format!("{}", e2["warnings"]));
    check("content: 5 correction commands", e2["commands"].as_array().map(|a| a.len()) == Some(5), format!("{}", e2["commands"]));

    // --- determinism: byte-identical stdout under a pinned epoch ---
    let a = rf(&["content", TOKEN, ".", "--json"], cd, &[("SOURCE_DATE_EPOCH", "0")]).2;
    let b = rf(&["content", TOKEN, ".", "--json"], cd, &[("SOURCE_DATE_EPOCH", "0")]).2;
    check("content: deterministic (byte-identical)", a == b, String::new());
    check("content: ts honors SOURCE_DATE_EPOCH",
          rf(&["content", TOKEN, ".", "--json"], cd, &[("SOURCE_DATE_EPOCH", "0")]).1["meta"]["ts_iso"] == "1970-01-01T00:00:00Z",
          String::new());

    // --- doctor: git repo -> ignore mode ACTIVE ---
    let (_, e, _) = rf(&["doctor", ".", "--json"], cd, &[]);
    check("doctor: repo -> ignore ACTIVE",
          e["data"][0]["ignore_mode"].as_str().unwrap_or("").contains("ACTIVE"),
          format!("{}", e["data"][0]["ignore_mode"]));

    // --- capabilities: advertises the verbs and the exit-code dictionary ---
    let (_, e, _) = rf(&["capabilities", "--json"], cd, &[]);
    let caps = &e["data"][0];
    check("capabilities: all verbs advertised",
          ["capabilities", "content", "find", "doctor"].iter().all(|v| caps["verbs"].get(*v).is_some()),
          String::new());
    check("capabilities: exit codes 0/1/3 documented",
          ["0", "1", "3"].iter().all(|c| caps["exit_codes"].get(*c).is_some()),
          String::new());
    check("capabilities: engine is in-process",
          caps["engine"].as_str().unwrap_or("").contains("in-process"), String::new());
    check("capabilities: conformance verb advertised (P-f discovery)",
          caps["verbs"].get("conformance").is_some(), String::new());
    check("capabilities: error codes registered (X-06)",
          ["USAGE", "BAD_PATTERN", "INTERNAL"].iter().all(|c| caps["error_codes"].get(*c).is_some()),
          String::new());

    // --- conformance: the in-situ self-check meets the P-f schema floor ---
    let (code, e, _) = rf(&["conformance", "--json"], cd, &[]);
    let d = &e["data"][0];
    check("conformance: exit 0 with no failures", code == 0 && e["ok"] == true, format!("code={code}"));
    check("conformance: profile is release-self-check",
          d["profile"] == "release-self-check", String::new());
    check("conformance: counts + cases present",
          d["counts"].get("pass").is_some() && d["counts"].get("fail").is_some()
              && d["counts"].get("not_applicable").is_some() && d["cases"].is_array(),
          String::new());
    let cases = d["cases"].as_array().cloned().unwrap_or_default();
    check("conformance: every case has the five floor keys",
          cases.iter().all(|c| ["case_id", "verdict", "reason", "request_id", "target"]
              .iter().all(|k| c.get(*k).is_some())),
          String::new());
    check("conformance: cases sorted by case_id",
          cases.windows(2).all(|w| w[0]["case_id"].as_str().unwrap_or("") <= w[1]["case_id"].as_str().unwrap_or("")),
          String::new());
    check("conformance: no case failed",
          d["counts"]["fail"].as_i64() == Some(0), format!("{}", d["counts"]["fail"]));
    check("conformance: not-applicable reasons are non-null",
          cases.iter().filter(|c| c["verdict"] == "not_applicable").all(|c| c["reason"].is_string()),
          String::new());

    // --- find: four independent sources, each miss attributed to one stage ---
    let fc = FindCorpus::new();
    let fd = Some(fc.path.as_path());
    let ast_available = Command::new("ast-grep")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    let (code, e, _) = rf(
        &["find", TOKEN, ".", "--name", "config", "--structural", "db.connect($$$)", "--lang", "python", "--json"],
        fd, &[],
    );
    check("find: exit 0", code == 0, format!("code={code}"));
    let stage: BTreeMap<String, String> = e["data"]
        .as_array()
        .unwrap_or(&vec![])
        .iter()
        .map(|d| (d["file"].as_str().unwrap_or("").to_string(), d["stage"].as_str().unwrap_or("").to_string()))
        .collect();
    for (file, want) in [
        ("src/app.config", "found"),
        ("settings.conf", "fd_name"),
        (".hidden.config", "fd_hidden"),
        ("gen/build.config", "fd_ignore"),
        ("bin.config", "rg_binary"),
        ("history.config", "git_deleted"),
    ] {
        check(&format!("find: {file} -> {want}"),
              stage.get(file).map(|s| s.as_str()) == Some(want),
              format!("{:?}", stage.get(file)));
    }
    // true negatives: correct name but no token, and the ast decoy, stay unflagged
    check("find: empty.config not flagged", !stage.contains_key("empty.config"), String::new());
    check("find: other.py not flagged (ast decoy)", !stage.contains_key("other.py"), String::new());
    // pipe accounting mirrors the reference corpus
    check("find: fd_default_files == 4", e["meta"]["fd_default_files"] == 4, format!("{}", e["meta"]));
    check("find: pipe_matched == 1", e["meta"]["pipe_matched"] == 1, format!("{}", e["meta"]));
    check("find: content_total == 5", e["meta"]["content_total"] == 5, format!("{}", e["meta"]));
    check("find: history_matches == 1", e["meta"]["history_matches"] == 1, format!("{}", e["meta"]));
    check("find: headline is PARTIAL",
          e["meta"]["headline"].as_str().unwrap_or("").starts_with("PARTIAL"),
          format!("{}", e["meta"]["headline"]));

    // structural source is conditional on ast-grep; both branches must be total
    let warns: Vec<String> = e["warnings"].as_array().unwrap_or(&vec![])
        .iter().map(|w| w["code"].as_str().unwrap_or("").to_string()).collect();
    if ast_available {
        check("find: conn.py -> ast_structural",
              stage.get("conn.py").map(|s| s.as_str()) == Some("ast_structural"),
              format!("{:?}", stage.get("conn.py")));
        check("find: structural_matches == 1", e["meta"]["structural_matches"] == 1, format!("{}", e["meta"]));
    } else {
        check("find: STRUCTURAL_UNAVAILABLE warned (ast-grep absent)",
              warns.iter().any(|c| c == "STRUCTURAL_UNAVAILABLE"), format!("{warns:?}"));
        check("find: no crash without ast-grep (exit 0)", code == 0, String::new());
    }

    // --- report (visible with `cargo test -- --nocapture`) ---
    let mut fails = Vec::new();
    for (name, cond, detail) in &results {
        println!("  {}  {name}{}", if *cond { "PASS" } else { "FAIL" },
                 if !cond && !detail.is_empty() { format!("   [{detail}]") } else { String::new() });
        if !cond {
            fails.push(name.clone());
        }
    }
    println!("\n{}/{} passed", results.len() - fails.len(), results.len());
    assert!(fails.is_empty(), "conformance failures: {fails:?}");
}
