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

use crate::engine::{content_matches, content_matches_selected, SearchCfg};
use crate::envelope::{envelope, err, warn};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::{Component, Path, PathBuf};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SelectionMode {
    NulStdin,
    Envelope,
}

impl SelectionMode {
    fn name(self) -> &'static str {
        match self {
            Self::NulStdin => "stdin-nul",
            Self::Envelope => "rf-envelope",
        }
    }
}

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
fn search_cfg(cfg: &Cfg) -> SearchCfg {
    SearchCfg {
        use_ignore: cfg.ignore_files,
        skip_hidden: cfg.hidden,
        binary_as_text: cfg.binary_as_text,
        case_insensitive: cfg.case_insensitive,
        encoding: cfg.encoding,
    }
}

fn matches_for(
    pattern: &str,
    path: &str,
    cfg: &Cfg,
    selected: Option<&BTreeMap<String, PathBuf>>,
) -> Result<BTreeSet<String>, String> {
    let cfg = search_cfg(cfg);
    match selected {
        Some(files) => content_matches_selected(path, pattern, &cfg, files),
        None => content_matches(path, pattern, &cfg),
    }
}

fn has_git_component(path: &Path) -> bool {
    path.components().any(|c| c.as_os_str() == ".git")
}

fn selection_error(message: impl Into<String>) -> (Value, i32) {
    let mut meta = Map::new();
    meta.insert("verb".into(), Value::from("content"));
    (
        envelope(false, vec![], meta, vec![], vec![], vec![err("INVALID_SELECTION", message)]),
        1,
    )
}

fn parse_selected_paths(mode: SelectionMode) -> Result<Vec<String>, String> {
    let mut bytes = Vec::new();
    std::io::stdin()
        .read_to_end(&mut bytes)
        .map_err(|e| format!("cannot read selected paths from stdin: {e}"))?;
    match mode {
        SelectionMode::NulStdin => {
            if bytes.is_empty() {
                return Err("selected path input is empty".into());
            }
            let mut paths = Vec::new();
            let parts: Vec<&[u8]> = bytes.split(|b| *b == 0).collect();
            for (index, part) in parts.iter().enumerate() {
                if part.is_empty() {
                    if index + 1 == parts.len() && bytes.ends_with(&[0]) {
                        continue; // a trailing NUL is the normal stream terminator
                    }
                    return Err("selected path input contains an empty path".into());
                }
                paths.push(
                    std::str::from_utf8(part)
                        .map_err(|_| "selected path input is not valid UTF-8")?
                        .to_string(),
                );
            }
            if paths.is_empty() {
                Err("selected path input contains no paths".into())
            } else {
                Ok(paths)
            }
        }
        SelectionMode::Envelope => {
            let value: Value = serde_json::from_slice(&bytes)
                .map_err(|_| "selected envelope input is not valid JSON")?;
            let data = value.get("data").and_then(Value::as_array)
                .ok_or("selected envelope must contain a data array")?;
            if data.is_empty() {
                return Err("selected envelope contains no files".into());
            }
            data.iter().map(|row| {
                row.get("file").and_then(Value::as_str)
                    .filter(|file| !file.is_empty())
                    .map(str::to_string)
                    .ok_or_else(|| "each selected envelope row must contain a non-empty file string".into())
            }).collect()
        }
    }
}

fn validate_selected_paths(path: &str, inputs: Vec<String>) -> Result<BTreeMap<String, PathBuf>, String> {
    let root = std::fs::canonicalize(path)
        .map_err(|_| "query root does not exist or cannot be resolved")?;
    if !root.is_dir() {
        return Err("query root must be a directory for selected-input search".into());
    }
    let mut selected = BTreeMap::new();
    for input in inputs {
        let supplied = Path::new(&input);
        if supplied.as_os_str().is_empty() {
            return Err("selected path is empty".into());
        }
        let candidate = if supplied.is_absolute() { supplied.to_path_buf() } else { root.join(supplied) };
        let canonical = std::fs::canonicalize(&candidate)
            .map_err(|_| format!("selected path is missing or unreadable: {input}"))?;
        if !canonical.starts_with(&root) {
            return Err(format!("selected path is outside the query root: {input}"));
        }
        if has_git_component(canonical.strip_prefix(&root).unwrap_or(&canonical)) {
            return Err(format!("selected path is inside .git: {input}"));
        }
        if !canonical.is_file() {
            return Err(format!("selected path is not a regular file: {input}"));
        }
        std::fs::File::open(&canonical)
            .map_err(|_| format!("selected path is unreadable: {input}"))?;
        let relative = canonical.strip_prefix(&root).unwrap_or(&canonical);
        if relative.components().any(|part| matches!(part, Component::ParentDir)) {
            return Err(format!("selected path is outside the query root: {input}"));
        }
        let display = if path == "." {
            relative.to_string_lossy().to_string()
        } else {
            Path::new(path).join(relative).to_string_lossy().to_string()
        };
        if selected.insert(display, canonical).is_some() {
            return Err(format!("selected path is duplicated: {input}"));
        }
    }
    Ok(selected)
}

pub fn run(
    pattern: &str,
    path: &str,
    limit: usize,
    cursor: Option<&str>,
    selection_mode: Option<SelectionMode>,
) -> (Value, i32) {
    crate::fault::maybe_fault("content");
    let selected = match selection_mode {
        Some(mode) => match parse_selected_paths(mode).and_then(|inputs| validate_selected_paths(path, inputs)) {
            Ok(paths) => Some(paths),
            Err(message) => return selection_error(message),
        },
        None => None,
    };
    let ls = layers();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut surfaced_by: Vec<(String, String)> = Vec::new(); // (file, layer)
    let mut default_files: BTreeSet<String> = BTreeSet::new();
    let mut warnings: Vec<Value> = Vec::new();
    let mut commands: Vec<String> = Vec::new();

    for layer in &ls {
        let files = match matches_for(pattern, path, &layer.cfg, selected.as_ref()) {
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
        let base = match matches_for(pattern, path, &probe_base_cfg(None), selected.as_ref()) {
            Ok(f) => f,
            Err(_) => continue,
        };
        let probed = match matches_for(pattern, path, &probe_base_cfg(Some(p.encoding)), selected.as_ref()) {
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
            if selection_mode.is_some() {
                m.insert("selection".into(), Value::from("selected"));
            }
            m.insert("surfaced_by".into(), Value::from(layer.clone()));
            Value::from(m)
        })
        .collect();

    let total = seen.len();
    let mut meta = Map::new();
    meta.insert("verb".into(), Value::from("content"));
    meta.insert("pattern".into(), Value::from(pattern));
    meta.insert("path".into(), Value::from(path));
    meta.insert("selection".into(), match (selection_mode, selected.as_ref()) {
        (Some(mode), Some(files)) => json!({"mode": mode.name(), "selected_files": files.len()}),
        _ => json!({"mode": "root-walk"}),
    });
    meta.insert("matched_files".into(), Value::from(total));
    meta.insert(
        "default_matched_files".into(),
        Value::from(default_files.len()),
    );
    meta.insert(
        "hidden_by_filters".into(),
        Value::from(total - default_files.len()),
    );

    let query = json!({"verb": "content", "pattern": pattern, "path": path, "selection": selection_mode.map(SelectionMode::name)});
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
