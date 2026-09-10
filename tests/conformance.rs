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
use std::sync::atomic::{AtomicU64, Ordering};

const BIN: &str = env!("CARGO_BIN_EXE_rf");
const TOKEN: &str = "MAGIC_TOKEN_XYZ";
const KEYS: [&str; 7] = ["ok", "tool_version", "data", "meta", "warnings", "commands", "errors"];
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn unique_temp(label: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("rf-{label}-{}-{nanos}-{sequence}", std::process::id()));
    std::fs::create_dir(&path).unwrap();
    path
}

/// A self-cleaning corpus that exercises every content layer once.
struct Corpus {
    path: PathBuf,
}

impl Corpus {
    fn new() -> Corpus {
        let path = unique_temp("conf");
        std::fs::create_dir_all(path.join("src")).unwrap();

        let w = |rel: &str, bytes: &[u8]| std::fs::write(path.join(rel), bytes).unwrap();
        w("src/app.py", format!("print('{TOKEN}')\n").as_bytes()); // default
        w("secrets.env", format!("KEY={TOKEN}\n").as_bytes()); // vcs_ignore
        w(".gitignore", b"*.env\n");
        w(".hidden.txt", format!("{TOKEN}\n").as_bytes()); // hidden
        w("blob.dat", format!("pre\0{TOKEN}\n").as_bytes()); // binary (NUL before match)
        // binary trap #2: NUL (so quit-on-NUL layers abort) THEN a match line
        // whose bytes are not valid UTF-8 (0xda continuation byte before the
        // token, then 0x80 after) — the real .pyc string-table shape. Only the
        // binary layer reaches it, and only a byte-oriented sink reports it: the
        // UTF8 sink errors on decode and silently drops the match.
        let mut blob = vec![b'h', b'e', b'a', b'd', 0x00, 0xda];
        blob.extend_from_slice(TOKEN.as_bytes());
        blob.extend_from_slice(&[0x80, b'\n']);
        w("blob_bin.dat", &blob); // binary (non-UTF-8 match line)
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
        let path = unique_temp("find");
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

/// The error contract has a second required channel: structured output remains
/// on stdout while the first error message is mirrored to stderr.
fn rf_with_stderr(
    args: &[&str],
    cwd: Option<&Path>,
    env: &[(&str, &str)],
) -> (i32, Value, String, String) {
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
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    let json = serde_json::from_str(&stdout).expect("machine output parses");
    (out.status.code().unwrap_or(-1), json, stdout, stderr)
}

#[test]
fn error_envelopes_and_global_json_are_total() {
    let corpus = Corpus::new();
    let cwd = Some(corpus.path.as_path());
    let error_keys = [
        "code",
        "message",
        "path",
        "remediation",
        "did_you_mean",
        "exit_code",
    ];

    for args in [
        vec!["--json", "content", "(", "."],
        vec!["content", "(", ".", "--json"],
        vec!["--json", "content"],
    ] {
        let (code, env, _stdout, stderr) = rf_with_stderr(&args, cwd, &[]);
        assert_eq!(code, 1, "{args:?}");
        assert_eq!(env["ok"], Value::from(false));
        assert!(env["data"].is_null());
        assert!(KEYS.iter().all(|key| env.get(*key).is_some()));
        assert!(error_keys.iter().all(|key| env["errors"][0].get(*key).is_some()));
        assert_eq!(env["errors"][0]["exit_code"], Value::from(1));
        assert!(!stderr.trim().is_empty());
    }

    // A global flag is valid in both parser positions. A flag after `--` is
    // literal pattern data and cannot select JSON mode by bootstrap accident.
    for args in [
        vec!["--json", "content", TOKEN, "."],
        vec!["content", TOKEN, ".", "--json"],
        vec!["content", "--", "--json"],
    ] {
        let (code, env, _stdout, _stderr) = rf_with_stderr(&args, cwd, &[]);
        assert_eq!(code, 0, "{args:?}");
        assert_eq!(env["ok"], Value::from(true));
    }

    // Stage-order probes: parser global flags beat a bad verb, verb resolution
    // beats later local syntax, and `--` prevents flag recognition.
    for (args, want) in [
        (vec!["--json", "--bogus", "badverb"], "UNKNOWN_FLAG"),
        (vec!["--json", "badverb", "--bogus"], "UNKNOWN_COMMAND"),
        (vec!["--json", "content", "--bogus"], "UNKNOWN_FLAG"),
    ] {
        let (code, env, _stdout, _stderr) = rf_with_stderr(&args, cwd, &[]);
        assert_eq!(code, 1, "{args:?}");
        assert_eq!(env["errors"][0]["code"], Value::from(want));
    }
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
            e["meta"]["contract_version"] == "2",
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
    check("content: 7 matched", e["meta"]["matched_files"] == 7, format!("{}", e["meta"]));
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
        ("blob_bin.dat", "binary"),
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

// --- P-e: golden verdict pins ---------------------------------------------
//
// The `conformance` test above asserts *structure* (floor keys, sorting, no
// fail). These two pins assert *instance -> verdict*: the exact machine contract
// and the exact case->verdict map. They catch silent drift a structural check
// cannot — a verb quietly added/removed, an exit or error code renamed, a case
// flipping pass<->fail<->n/a, or an n/a reason string changing.
//
// The goldens are generated from real binary output, never hand-written. When a
// change to either surface is intentional, re-pin (commands below) and commit
// the new golden as part of that change.

#[cfg(not(feature = "fault-injection"))]
const CAPS_GOLDEN: &str = include_str!("golden/capabilities.data.json");
#[cfg(not(feature = "fault-injection"))]
const VERDICTS_GOLDEN: &str = include_str!("golden/conformance.verdicts.json");

/// tool_version tracks CARGO_PKG_VERSION and bumps every release; the pin guards
/// the *contract*, so normalize it out. contract_version stays real — it is the
/// gate that must move deliberately.
#[cfg(not(feature = "fault-injection"))]
fn normalize_caps(mut d: Value) -> Value {
    if let Some(o) = d.as_object_mut() {
        o.insert("tool_version".into(), Value::from("PINNED"));
    }
    d
}

// Pinned against the release contract. A fault-injection build discloses RF_FAULT
// in env_vars and flips several conformance verdicts, so its output is a different
// (also-honest) contract; the pins guard the canonical release build only.
#[cfg(not(feature = "fault-injection"))]
#[test]
fn golden_capabilities_contract() {
    let want: Value = serde_json::from_str(CAPS_GOLDEN).expect("golden parses");
    let (_, e, _) = rf(&["capabilities", "--json"], None, &[("SOURCE_DATE_EPOCH", "0")]);
    let got = normalize_caps(e["data"][0].clone());
    assert!(
        got == want,
        "capabilities contract drifted from the golden pin.\n\
         If this change is intentional, re-pin:\n  \
         SOURCE_DATE_EPOCH=0 cargo run -q -- capabilities --json \\\n    \
         | python3 -c \"import sys,json; d=json.load(sys.stdin)['data'][0]; d['tool_version']='PINNED'; \
print(json.dumps(d,indent=2,sort_keys=True))\" > tests/golden/capabilities.data.json\n\n\
         observed (normalized):\n{}",
        serde_json::to_string_pretty(&got).unwrap_or_default()
    );
}

#[cfg(not(feature = "fault-injection"))]
#[test]
fn golden_conformance_verdicts() {
    let want: Value = serde_json::from_str(VERDICTS_GOLDEN).expect("golden parses");
    let (_, e, _) = rf(&["conformance", "--json"], None, &[("SOURCE_DATE_EPOCH", "0")]);
    let p = &e["data"][0];
    // Project to the pinned shape: request_id is per-host (embeds argv[0]) and
    // target is derivable from case_id, so neither is pinned; verdict + reason are.
    let cases: Vec<Value> = p["cases"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|c| {
            let mut m = serde_json::Map::new();
            m.insert("case_id".into(), c["case_id"].clone());
            m.insert("verdict".into(), c["verdict"].clone());
            m.insert("reason".into(), c["reason"].clone());
            Value::from(m)
        })
        .collect();
    let mut got = serde_json::Map::new();
    got.insert("profile".into(), p["profile"].clone());
    got.insert("counts".into(), p["counts"].clone());
    got.insert("cases".into(), Value::from(cases));
    let got = Value::from(got);
    assert!(
        got == want,
        "conformance verdict set drifted from the golden pin.\n\
         If this change is intentional, re-pin:\n  \
         SOURCE_DATE_EPOCH=0 cargo run -q -- conformance --json \\\n    \
         | python3 -c \"import sys,json; p=json.load(sys.stdin)['data'][0]; \
print(json.dumps({{'profile':p['profile'],'counts':p['counts'],\
'cases':[{{'case_id':c['case_id'],'verdict':c['verdict'],'reason':c['reason']}} for c in p['cases']]}},\
indent=2,sort_keys=True))\" > tests/golden/conformance.verdicts.json\n\n\
         observed:\n{}",
        serde_json::to_string_pretty(&got).unwrap_or_default()
    );
}

// --- P-d: the per-stage fault-injection seam ------------------------------
//
// Two complementary proofs, one per build. The release build proves the seam is
// truly compiled out (RF_FAULT is inert); the fault-injection build proves the
// totality wrapper converts a real per-stage panic into a total error envelope.
// Exactly one runs per `cargo test` invocation, chosen by the feature flag.

/// argv that reaches each stage's seam with otherwise-valid input.
fn stage_argv(stage: &str) -> Vec<&'static str> {
    match stage {
        "content" | "engine" => vec!["content", "zzq_no_such_token", "/rf-conformance-empty", "--json"],
        "find" => vec!["find", "zzq_no_such_token", "/rf-conformance-empty", "--name", "conf", "--json"],
        _ => vec!["doctor", ".", "--json"],
    }
}

/// Verdict of a single conformance case_id (exact match), or "<absent>".
fn verdict_of(e: &Value, case_id: &str) -> String {
    e["data"][0]["cases"]
        .as_array()
        .and_then(|cs| cs.iter().find(|c| c["case_id"] == case_id))
        .and_then(|c| c["verdict"].as_str())
        .unwrap_or("<absent>")
        .to_string()
}

#[cfg(not(feature = "fault-injection"))]
#[test]
fn fault_seam_absent_in_release() {
    // RF_FAULT must be inert: the seam is not compiled in, so a normal search runs.
    for stage in ["content", "find", "doctor", "engine"] {
        let (code, e, _) = rf(&stage_argv(stage), None, &[("RF_FAULT", stage)]);
        assert_eq!(code, 0, "RF_FAULT={stage} should be inert in a release build");
        assert_eq!(e["ok"], Value::from(true), "RF_FAULT={stage} perturbed output");
    }
    // And the self-check reports the totality cases not-applicable, not pass.
    let (_, e, _) = rf(&["conformance", "--json"], None, &[]);
    for id in ["S-01", "S-02"] {
        assert_eq!(verdict_of(&e, id), "not_applicable", "{id} in release build");
    }
    assert_eq!(verdict_of(&e, "X-03::posture=compiled-out"), "pass");
}

#[cfg(feature = "fault-injection")]
#[test]
fn fault_seam_proves_totality() {
    // Injecting a fault at each stage must yield a TOTAL error envelope: the
    // catch_unwind wrapper turns the panic into exit 6 + INTERNAL + seven keys,
    // never an unmediated crash (which would be exit 101 with no JSON).
    for stage in ["content", "find", "doctor", "engine"] {
        let (code, e, _) = rf(&stage_argv(stage), None, &[("RF_FAULT", stage)]);
        assert_eq!(code, 6, "stage {stage}: fault must exit 6 (internal), not crash");
        assert_eq!(e["ok"], Value::from(false), "stage {stage}: ok must be false");
        assert_eq!(e["errors"][0]["code"], Value::from("INTERNAL"), "stage {stage}");
        assert!(
            e.is_object() && KEYS.iter().all(|k| e.get(*k).is_some()),
            "stage {stage}: envelope lost a key under fault",
        );
    }
    // The in-situ self-check now adjudicates the totality cases for real.
    let (code, e, _) = rf(&["conformance", "--json"], None, &[]);
    assert_eq!(code, 0, "self-check must still be all-pass under the seam build");
    for id in ["S-01::stage=content", "S-02::stage=find", "X-02::stages=all"] {
        assert_eq!(verdict_of(&e, id), "pass", "{id} should pass with the seam in");
    }
    assert_eq!(verdict_of(&e, "X-03"), "not_applicable", "X-03 not release posture");
    assert_eq!(e["data"][0]["counts"]["fail"], Value::from(0));
}
