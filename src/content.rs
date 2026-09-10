//! The `content` verb — in-process content search with native layer
//! attribution.
//!
//! Ripgrep's default answer to a naive query hides matches behind four
//! independent filters (vcs-ignore, hidden, binary, case). We peel them as
//! cumulative layers and attribute every recovered file to the exact filter
//! that hid it. There is no subprocess: the
//! `ignore` walker and `grep` searcher run in-process, so a file's membership
//! in each layer is a native read, not a diff of two stdout dumps.
//!
//! Encoding is deliberately NOT a cumulative layer. Forcing a decoder (utf-16)
//! makes the searcher misread every UTF-8 file, so a cumulative layer would
//! DROP the matches the earlier layers found. Instead it is a parallel PROBE:
//! run with the forced encoding, diff against the same config WITHOUT it, and
//! keep only the files the decoder alone surfaces.

use crate::engine::{content_matches, SearchCfg};
use crate::envelope::{envelope, err, warn};
use serde_json::{json, Map, Value};
use std::collections::BTreeSet;

struct Cfg {
    ignore_files: bool, // honor .gitignore/.ignore
    hidden: bool,       // skip hidden/dotfiles
    binary_as_text: bool,
    case_insensitive: bool,
    encoding: Option<&'static str>, // forced decoder label, or None for auto
}

struct Layer {
    name: &'static str,
    cfg: Cfg,
    code: Option<&'static str>,
    hint: Option<&'static str>,
    flags: &'static str, // paste-ready ripgrep flags for the correction command
}

fn layers() -> Vec<Layer> {
    vec![
        Layer { name: "default",    cfg: Cfg { ignore_files: true,  hidden: true,  binary_as_text: false, case_insensitive: false, encoding: None }, code: None, hint: None, flags: "" },
        Layer { name: "vcs_ignore", cfg: Cfg { ignore_files: false, hidden: true,  binary_as_text: false, case_insensitive: false, encoding: None }, code: Some("IGNORE_VCS"),     hint: Some("add -u (ignore .gitignore/.ignore rules)"), flags: "-u" },
        Layer { name: "hidden",     cfg: Cfg { ignore_files: false, hidden: false, binary_as_text: false, case_insensitive: false, encoding: None }, code: Some("HIDDEN_SKIPPED"), hint: Some("add -uu (also search hidden/dotfiles)"), flags: "-uu" },
        Layer { name: "binary",     cfg: Cfg { ignore_files: false, hidden: false, binary_as_text: true,  case_insensitive: false, encoding: None }, code: Some("BINARY_SKIPPED"), hint: Some("add -uu -a (treat binary files as text)"), flags: "-uu -a" },
        Layer { name: "case",       cfg: Cfg { ignore_files: false, hidden: false, binary_as_text: true,  case_insensitive: true,  encoding: None }, code: Some("CASE_SENSITIVE"), hint: Some("add -i (case-insensitive)"), flags: "-uu -a -i" },
    ]
}

/// Parallel (non-cumulative) probes. `base` is the config the probe is diffed
/// against — the -uu -a config without the forced decoder — so only files the
/// decoder alone surfaces are attributed here.
struct Probe {
    name: &'static str,
    encoding: &'static str,
    code: &'static str,
    hint: &'static str,
    flags: &'static str,
}

fn probes() -> Vec<Probe> {
    vec![Probe {
        name: "encoding_utf16",
        encoding: "utf-16",
        code: "ENCODING_MISS",
        hint: "add --encoding utf-16 (non-UTF-8 file)",
        flags: "-uu -a --encoding utf-16",
    }]
}

fn probe_base_cfg(encoding: Option<&'static str>) -> Cfg {
    // matches the `binary` layer (-uu -a): ignore off, hidden off, binary as text.
    Cfg { ignore_files: false, hidden: false, binary_as_text: true, case_insensitive: false, encoding }
}

/// Files under `path` containing `pattern` under one filter configuration.
/// Delegates to the shared engine, so `content` and `find` search identically.
fn matches_for(pattern: &str, path: &str, cfg: &Cfg) -> Result<BTreeSet<String>, String> {
    content_matches(
        path,
        pattern,
        &SearchCfg {
            use_ignore: cfg.ignore_files,
            skip_hidden: cfg.hidden,
            binary_as_text: cfg.binary_as_text,
            case_insensitive: cfg.case_insensitive,
            encoding: cfg.encoding,
        },
    )
}

pub fn run(pattern: &str, path: &str, limit: usize, cursor: Option<&str>) -> (Value, i32) {
    crate::fault::maybe_fault("content");
    let ls = layers();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut surfaced_by: Vec<(String, String)> = Vec::new(); // (file, layer)
    let mut default_files: BTreeSet<String> = BTreeSet::new();
    let mut warnings: Vec<Value> = Vec::new();
    let mut commands: Vec<String> = Vec::new();

    for layer in &ls {
        let files = match matches_for(pattern, path, &layer.cfg) {
            Ok(f) => f,
            Err(e) => {
                let mut meta = Map::new();
                meta.insert("verb".into(), Value::from("content"));
                return (
                    envelope(false, vec![], meta, vec![], vec![], vec![err("BAD_PATTERN", e)]),
                    1,
                );
            }
        };
        if layer.name == "default" {
            default_files = files.clone();
        }
        let new: Vec<String> = files.difference(&seen).cloned().collect();
        for f in &new {
            surfaced_by.push((f.clone(), layer.name.to_string()));
        }
        seen.extend(files);
        if layer.name != "default" && !new.is_empty() {
            if let (Some(code), Some(hint)) = (layer.code, layer.hint) {
                warnings.push(warn(
                    code,
                    format!("{} match(es) hidden by default; {hint}", new.len()),
                    new.clone(),
                ));
                let mut args: Vec<String> = layer.flags.split_whitespace().map(String::from).collect();
                args.extend(["-e".into(), pattern.into(), "--".into(), path.into()]);
                commands.push(crate::command::shell("rg", &args));
            }
        }
    }

    // Parallel probes: each forced decoder is diffed against its own base (same
    // config, no encoding), so we add only files the decoder alone surfaces and
    // never lose the UTF-8 matches the cumulative layers already found.
    for p in probes() {
        let base = match matches_for(pattern, path, &probe_base_cfg(None)) {
            Ok(f) => f,
            Err(_) => continue,
        };
        let probed = match matches_for(pattern, path, &probe_base_cfg(Some(p.encoding))) {
            Ok(f) => f,
            Err(_) => continue, // a broken decoder contributes nothing (totality)
        };
        let new: Vec<String> = probed
            .difference(&base)
            .filter(|f| !seen.contains(*f))
            .cloned()
            .collect();
        for f in &new {
            surfaced_by.push((f.clone(), p.name.to_string()));
        }
        seen.extend(probed);
        if !new.is_empty() {
            warnings.push(warn(
                p.code,
                format!("{} match(es) hidden by default; {}", new.len(), p.hint),
                new.clone(),
            ));
            let mut args: Vec<String> = p.flags.split_whitespace().map(String::from).collect();
            args.extend(["-e".into(), pattern.into(), "--".into(), path.into()]);
            commands.push(crate::command::shell("rg", &args));
        }
    }

    surfaced_by.sort();
    let data: Vec<Value> = surfaced_by
        .iter()
        .map(|(f, layer)| {
            let mut m = Map::new();
            m.insert("file".into(), Value::from(f.clone()));
            m.insert("surfaced_by".into(), Value::from(layer.clone()));
            Value::from(m)
        })
        .collect();

    let total = seen.len();
    let mut meta = Map::new();
    meta.insert("verb".into(), Value::from("content"));
    meta.insert("pattern".into(), Value::from(pattern));
    meta.insert("path".into(), Value::from(path));
    meta.insert("matched_files".into(), Value::from(total));
    meta.insert(
        "default_matched_files".into(),
        Value::from(default_files.len()),
    );
    meta.insert(
        "hidden_by_filters".into(),
        Value::from(total - default_files.len()),
    );

    let query = json!({"verb": "content", "pattern": pattern, "path": path});
    match crate::pagination::page(data, &query, limit, cursor) {
        Ok(page) => {
            let has_more = page.next_cursor.is_some();
            meta.insert("pagination".into(), json!({
                "limit": limit,
                "returned": page.data.len(),
                "total": page.total,
                "truncated": has_more,
                "has_more": has_more,
                "cursor": page.next_cursor,
                "snapshot_hash": page.snapshot_hash,
            }));
            (envelope(true, page.data, meta, warnings, commands, vec![]), 0)
        }
        Err(error) => {
            let mut failure_meta = Map::new();
            failure_meta.insert("verb".into(), Value::from("content"));
            let (code, message, exit, restart) = match error {
                crate::pagination::Error::InvalidCursor => ("INVALID_INPUT", "cursor is malformed or does not match this query", 1, vec![]),
                crate::pagination::Error::Conflict => (
                    "CONFLICT",
                    "the result snapshot changed; restart the query",
                    5,
                    vec![crate::command::shell("rf", &[
                        "content".into(), "--limit".into(), limit.to_string(), pattern.into(), "--".into(), path.into(),
                    ])],
                ),
            };
            (envelope(false, vec![], failure_meta, vec![], restart, vec![err(code, message)]), exit)
        }
    }
}
