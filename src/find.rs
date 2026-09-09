//! The `find` verb — staged cross-source discovery, each match attributed to
//! the one stage that hid it. Four independent sources:
//!
//!   1. fd name filters   -> `ignore`-walker classifications (in-process)
//!   2. rg binary skip     -> `grep` binary-detection read   (in-process)
//!   3. git history        -> `git` plumbing                 (subprocess)
//!   4. ast-grep structural-> `ast-grep run --json`          (subprocess)
//!
//! Sources 1 and 2 are the fd|rg pipe, read natively in-process via the shared
//! engine. Sources 3 and 4 are genuinely external tools with no Rust binding;
//! shelling out to them is not pipe-reconstruction, it is reading a different
//! oracle. Each source contributes nothing (not a crash) when unavailable, so
//! totality holds.

use crate::engine::{content_matches, name_matches, rel, SearchCfg};
use crate::envelope::{envelope, err, warn};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::process::Command;

/// -uu -a config: the content the pipe surfaces if nothing filters it (no
/// ignore, hidden shown, binary read as text). content_all uses this; the
/// per-file attribution then explains why the plain default missed each file.
fn cfg_all_text() -> SearchCfg {
    SearchCfg { use_ignore: false, skip_hidden: false, binary_as_text: true, case_insensitive: false, encoding: None }
}

/// ripgrep's own default: honor ignore + hidden, quit on NUL.
fn cfg_default() -> SearchCfg {
    SearchCfg { use_ignore: true, skip_hidden: true, binary_as_text: false, case_insensitive: false, encoding: None }
}

/// git plumbing: files matching *.ext that held `pattern` in ANY committed tree.
/// Empty (never an error) when `root` is not a work tree or git is absent.
fn git_ever_matched(pattern: &str, root: &str, ext: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let probe = Command::new("git")
        .args(["-C", root, "rev-parse", "--is-inside-work-tree"])
        .output();
    let in_tree = matches!(probe, Ok(ref o) if o.status.success()
        && String::from_utf8_lossy(&o.stdout).trim() == "true");
    if !in_tree {
        return out;
    }
    let revs_out = match Command::new("git").args(["-C", root, "rev-list", "--all"]).output() {
        Ok(o) => o,
        Err(_) => return out,
    };
    let glob = format!("*.{ext}");
    for rev in String::from_utf8_lossy(&revs_out.stdout).split_whitespace() {
        let g = match Command::new("git")
            .args(["-C", root, "grep", "-l", "-e", pattern, rev, "--", &glob])
            .output()
        {
            Ok(o) => o,
            Err(_) => continue,
        };
        for ln in String::from_utf8_lossy(&g.stdout).lines() {
            // "<rev>:<path>"
            if let Some((_, path)) = ln.split_once(':') {
                out.insert(path.to_string());
            }
        }
    }
    out
}

/// Short hash of the most recent commit that changed `pattern`'s count in
/// `path` — the scrub commit. Volatile; used only in a warning, never in `data`.
fn git_scrub_commit(pattern: &str, root: &str, path: &str) -> String {
    let sflag = format!("-S{pattern}");
    let out = Command::new("git")
        .args(["-C", root, "log", "-1", "--format=%h", &sflag, "--", path])
        .output();
    match out {
        Ok(o) => {
            let h = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if h.is_empty() { "unknown".into() } else { h }
        }
        Err(_) => "unknown".into(),
    }
}

/// Files whose SYNTAX matches the structural pattern (ast-grep). Returns
/// (files, available); available is false only when ast-grep is absent, so the
/// stage contributes nothing and totality holds.
fn ast_files(structural: &str, lang: &str, root: &str) -> (BTreeSet<String>, bool) {
    let out = Command::new("ast-grep")
        .args(["run", "--pattern", structural, "--lang", lang, "--json", "."])
        .current_dir(root)
        .output();
    let o = match out {
        Ok(o) => o,
        Err(_) => return (BTreeSet::new(), false), // binary not found
    };
    if !o.status.success() {
        return (BTreeSet::new(), true); // present, but bad pattern/lang: no hits
    }
    let text = String::from_utf8_lossy(&o.stdout);
    let hits: Value = serde_json::from_str(if text.trim().is_empty() { "[]" } else { &text })
        .unwrap_or(Value::Array(vec![]));
    let mut files = BTreeSet::new();
    if let Some(arr) = hits.as_array() {
        for m in arr {
            if let Some(f) = m.get("file").and_then(|v| v.as_str()) {
                files.insert(f.strip_prefix("./").unwrap_or(f).to_string());
            }
        }
    }
    (files, true)
}

/// Does the file at root/rel_path contain `needle` as a literal byte substring?
/// The in-process equivalent of `rg -F -uu -a` restricted to the ast hits: a
/// file where this is false but ast matched is surfaced only structurally.
fn file_contains_literal(root: &str, rel_path: &str, needle: &str) -> bool {
    let full = if root == "." || root.is_empty() {
        rel_path.to_string()
    } else {
        format!("{}/{}", root.trim_end_matches('/'), rel_path)
    };
    match std::fs::read(&full) {
        Ok(bytes) => bytes.windows(needle.len().max(1)).any(|w| w == needle.as_bytes()),
        Err(_) => false,
    }
}

pub fn run(pattern: &str, path: &str, name: &str, structural: Option<&str>, lang: Option<&str>) -> (Value, i32) {
    // Guard the port keeps stable across the contract: --structural needs --lang.
    if structural.is_some() && lang.is_none() {
        let mut meta = Map::new();
        meta.insert("verb".into(), Value::from("find"));
        return (
            envelope(false, vec![], meta, vec![], vec![],
                     vec![err("USAGE", "--structural requires --lang (ast-grep needs a language)")]),
            1,
        );
    }
    crate::fault::maybe_fault("find");
    let ext = name;
    let root = path;
    let relset = |s: BTreeSet<String>| -> BTreeSet<String> { s.into_iter().map(|f| rel(root, &f)).collect() };

    // --- source 1: fd name-filter stages (in-process walker) ---
    let fd_default = relset(name_matches(root, ext, true, true));
    let fd_hidden = relset(name_matches(root, ext, true, false));
    let fd_ignore = relset(name_matches(root, ext, false, true));
    let fd_all = relset(name_matches(root, ext, false, false));
    let dropped_hidden: BTreeSet<_> = fd_hidden.difference(&fd_default).cloned().collect();
    let dropped_ignore: BTreeSet<_> = fd_ignore.difference(&fd_default).cloned().collect();

    // --- source 1+2: the content the pipe reads (in-process searcher) ---
    let content_all = match content_matches(root, pattern, &cfg_all_text()) {
        Ok(s) => relset(s),
        Err(e) => {
            let mut meta = Map::new();
            meta.insert("verb".into(), Value::from("find"));
            return (envelope(false, vec![], meta, vec![], vec![], vec![err("BAD_PATTERN", e)]), 1);
        }
    };
    let rg_default = match content_matches(root, pattern, &cfg_default()) {
        Ok(s) => relset(s),
        Err(e) => {
            let mut meta = Map::new();
            meta.insert("verb".into(), Value::from("find"));
            return (envelope(false, vec![], meta, vec![], vec![], vec![err("BAD_PATTERN", e)]), 1);
        }
    };
    let pipe_found: BTreeSet<_> = fd_default.intersection(&rg_default).cloned().collect();

    // --- per-file stage attribution over the content the pipe surfaced ---
    let mut data: Vec<Value> = Vec::new();
    let mut stage_counts: BTreeMap<String, i64> = BTreeMap::new();
    let bump = |m: &mut BTreeMap<String, i64>, s: &str| { *m.entry(s.to_string()).or_insert(0) += 1; };
    let row = |file: &str, stage: &str, fix: Option<String>| -> Value {
        let mut r = Map::new();
        r.insert("file".into(), Value::from(file.to_string()));
        r.insert("stage".into(), Value::from(stage.to_string()));
        r.insert("fix".into(), fix.map(Value::from).unwrap_or(Value::Null));
        Value::from(r)
    };

    for f in &content_all {
        let (stage, fix): (&str, Option<String>) = if fd_default.contains(f) {
            if rg_default.contains(f) {
                ("found", None)
            } else {
                ("rg_binary", Some("add rg -a (fd passed it; rg suppressed a binary file)".into()))
            }
        } else if fd_all.contains(f) {
            if dropped_hidden.contains(f) {
                ("fd_hidden", Some("add fd -H (hidden dotfile matches the name filter)".into()))
            } else if dropped_ignore.contains(f) {
                ("fd_ignore", Some("add fd -I (gitignored file matches the name filter)".into()))
            } else {
                ("fd_filter", Some("peel fd filters (fd -u)".into()))
            }
        } else {
            ("fd_name", Some(format!("widen name filter (matches '{pattern}' but not *.{ext})")))
        };
        data.push(row(f, stage, fix));
        bump(&mut stage_counts, stage);
    }

    // --- source 3: git history (scrubbed from the tree) ---
    let ever = git_ever_matched(pattern, root, ext);
    let git_deleted: Vec<String> = ever.difference(&content_all).cloned().collect(); // BTreeSet -> sorted
    let mut scrub_commits: BTreeMap<String, String> = BTreeMap::new();
    for f in &git_deleted {
        scrub_commits.insert(f.clone(), git_scrub_commit(pattern, root, f));
        data.push(row(f, "git_deleted", Some(format!("recover from history: git log -S{pattern} -- {f}"))));
        bump(&mut stage_counts, "git_deleted");
    }

    // --- source 4: ast-grep structural (a construct with no literal form) ---
    let mut ast_only: Vec<String> = Vec::new();
    let mut ast_unavailable = false;
    if let Some(sp) = structural {
        let lg = lang.unwrap_or("");
        let (ast_hits, available) = ast_files(sp, lg, root);
        if !available {
            ast_unavailable = true;
        } else {
            // ast_only = files ast matched but a literal search for the pattern
            // text does NOT — surfaced only structurally.
            ast_only = ast_hits
                .into_iter()
                .filter(|f| !file_contains_literal(root, f, sp))
                .collect(); // came from a BTreeSet -> already sorted
            for f in &ast_only {
                data.push(row(
                    f,
                    "ast_structural",
                    Some(format!("literal search finds 0; match is structural: ast-grep run -p '{sp}' -l {lg} .")),
                ));
                bump(&mut stage_counts, "ast_structural");
            }
        }
    }

    // --- headline: one line an agent can branch on ---
    let missed: Vec<&Value> = data.iter().filter(|d| d["stage"] != "found").collect();
    let tree_missed: Vec<&&Value> = missed
        .iter()
        .filter(|d| d["stage"] != "git_deleted" && d["stage"] != "ast_structural")
        .collect();
    let mut hist = String::new();
    if !git_deleted.is_empty() {
        hist.push_str(&format!(" (+{} in git history only)", git_deleted.len()));
    }
    if !ast_only.is_empty() {
        hist.push_str(&format!(" (+{} structural-only via ast-grep)", ast_only.len()));
    }
    let headline = if fd_default.is_empty() {
        format!("FD_EMPTY: name filter matched no files; nothing reached rg{hist}")
    } else if pipe_found.is_empty() && !content_all.is_empty() {
        format!("PIPE_EMPTY: fd found files but pipe surfaced no matches; see stage attribution{hist}")
    } else if !tree_missed.is_empty() || !git_deleted.is_empty() || !ast_only.is_empty() {
        format!(
            "PARTIAL: pipe surfaced {}/{} in the tree; {} hidden by stage filters{hist}",
            pipe_found.len(), content_all.len(), tree_missed.len()
        )
    } else {
        "COMPLETE: pipe surfaced all matches".to_string()
    };

    // --- warnings: one per miss, carrying the paste-ready fix ---
    let mut warnings: Vec<Value> = Vec::new();
    if ast_unavailable {
        warnings.push(warn(
            "STRUCTURAL_UNAVAILABLE",
            "--structural given but ast-grep not found; structural source skipped (brew install ast-grep)",
            vec![],
        ));
    }
    for d in &missed {
        let stage = d["stage"].as_str().unwrap_or("");
        let file = d["file"].as_str().unwrap_or("").to_string();
        let mut msg = d["fix"].as_str().unwrap_or("").to_string();
        if stage == "git_deleted" {
            let sc = scrub_commits.get(&file).cloned().unwrap_or_else(|| "unknown".into());
            msg.push_str(&format!(" (scrubbed in {sc})"));
        }
        warnings.push(warn(&stage.to_uppercase(), msg, vec![file]));
    }

    // --- commands: correction recipes, one per distinct miss kind ---
    let mut commands: Vec<String> = Vec::new();
    let has = |s: &str| missed.iter().any(|d| d["stage"] == s);
    if has("fd_hidden") || has("fd_ignore") || has("fd_filter") {
        commands.push(format!("fd -u -e {ext} . | xargs rg -a '{pattern}'"));
    }
    if has("fd_name") {
        commands.push(format!("rg -uu '{pattern}'   # drop the name filter"));
    }
    if has("rg_binary") {
        commands.push(format!("fd -e {ext} . | xargs rg -a '{pattern}'"));
    }
    if !git_deleted.is_empty() {
        commands.push(format!("git log -S'{pattern}' --oneline --all   # matches scrubbed from the tree"));
    }
    if !ast_only.is_empty() {
        let sp = structural.unwrap_or("");
        let lg = lang.unwrap_or("");
        commands.push(format!("ast-grep run -p '{sp}' -l {lg} {root}   # structural matches literal search misses"));
    }

    // --- meta ---
    let mut meta = Map::new();
    meta.insert("verb".into(), Value::from("find"));
    meta.insert("pattern".into(), Value::from(pattern));
    meta.insert("name_filter".into(), Value::from(format!("*.{ext}")));
    meta.insert("path".into(), Value::from(root));
    meta.insert("headline".into(), Value::from(headline));
    meta.insert("fd_default_files".into(), Value::from(fd_default.len()));
    meta.insert("pipe_matched".into(), Value::from(pipe_found.len()));
    meta.insert("content_total".into(), Value::from(content_all.len()));
    meta.insert("history_matches".into(), Value::from(git_deleted.len()));
    meta.insert("structural_matches".into(), Value::from(ast_only.len()));
    let counts: Map<String, Value> = stage_counts.into_iter().map(|(k, v)| (k, Value::from(v))).collect();
    meta.insert("hidden_by_stage".into(), Value::from(counts));

    (envelope(true, data, meta, warnings, commands, vec![]), 0)
}
