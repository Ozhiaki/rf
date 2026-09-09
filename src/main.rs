//! rf — an agent-first forensic search envelope that fuses ripgrep and fd.
//!
//! In-process design: the `ignore` walker and `grep` searcher (ripgrep's and
//! fd's own crates) run linked in-process, so every match keeps its full stage
//! provenance natively instead of being reconstructed from a shell pipe. Every
//! verb emits the universal machine-first envelope; see `capabilities`.

mod capabilities;
mod conformance;
mod content;
mod doctor;
mod engine;
mod envelope;
mod fault;
mod find;
mod manifest;

use clap::{Parser, Subcommand};
use envelope::{envelope, err};
use serde_json::{Map, Value};
use std::io::IsTerminal;

#[derive(Parser)]
#[command(name = "rf", version, about = "agent-first forensic search over ripgrep + fd", disable_help_subcommand = true)]
struct Cli {
    #[command(subcommand)]
    verb: Verb,
}

#[derive(Subcommand)]
enum Verb {
    /// Emit the machine contract.
    Capabilities {
        #[arg(long)]
        json: bool,
    },
    /// Content search with per-filter attribution.
    Content {
        pattern: String,
        #[arg(default_value = ".")]
        path: String,
        #[arg(long)]
        json: bool,
    },
    /// Staged cross-source discovery (port in progress).
    Find {
        pattern: String,
        #[arg(default_value = ".")]
        path: String,
        #[arg(long)]
        name: String,
        #[arg(long)]
        structural: Option<String>,
        #[arg(long)]
        lang: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Diagnose the environment and active ignore mode.
    Doctor {
        #[arg(default_value = ".")]
        path: String,
        #[arg(long)]
        json: bool,
    },
    /// Run the release self-check profile against this binary.
    Conformance {
        #[arg(long)]
        json: bool,
    },
}

fn wants_json(v: &Verb) -> bool {
    let flag = match v {
        Verb::Capabilities { json }
        | Verb::Content { json, .. }
        | Verb::Find { json, .. }
        | Verb::Doctor { json, .. }
        | Verb::Conformance { json } => *json,
    };
    flag || !std::io::stdout().is_terminal()
}

fn dispatch(v: &Verb) -> (Value, i32) {
    match v {
        Verb::Capabilities { .. } => capabilities::run(),
        Verb::Content { pattern, path, .. } => content::run(pattern, path),
        Verb::Find { pattern, path, name, structural, lang, .. } => {
            find::run(pattern, path, name, structural.as_deref(), lang.as_deref())
        }
        Verb::Doctor { path, .. } => doctor::run(path),
        Verb::Conformance { .. } => conformance::run(),
    }
}

/// Minimal human rendering for a TTY; JSON is the machine default.
fn render_human(env: &Value) -> String {
    let meta = &env["meta"];
    let verb = meta["verb"].as_str().unwrap_or("");
    let mut out: Vec<String> = Vec::new();
    match verb {
        "content" => {
            out.push(format!(
                "content '{}' in {}: {} file(s), {} by default, {} hidden by filters",
                meta["pattern"].as_str().unwrap_or(""),
                meta["path"].as_str().unwrap_or(""),
                meta["matched_files"], meta["default_matched_files"], meta["hidden_by_filters"]
            ));
            for d in env["data"].as_array().unwrap_or(&vec![]) {
                out.push(format!(
                    "  {:<12} {}",
                    d["surfaced_by"].as_str().unwrap_or(""),
                    d["file"].as_str().unwrap_or("")
                ));
            }
        }
        "doctor" => {
            let d = &env["data"][0];
            out.push(format!("rf: {}", d["rf_version"].as_str().unwrap_or("")));
            out.push(format!("engine: {}", d["engine"].as_str().unwrap_or("")));
            out.push(format!("ignore_mode: {}", d["ignore_mode"].as_str().unwrap_or("")));
        }
        "conformance" => {
            let d = &env["data"][0];
            out.push(format!(
                "conformance [{}]: {}",
                d["profile"].as_str().unwrap_or(""),
                meta["headline"].as_str().unwrap_or("")
            ));
            for c in d["cases"].as_array().unwrap_or(&vec![]) {
                let v = c["verdict"].as_str().unwrap_or("");
                if v == "pass" {
                    continue;
                }
                out.push(format!(
                    "  {:<15} {} {}",
                    c["case_id"].as_str().unwrap_or(""),
                    v,
                    c["reason"].as_str().unwrap_or("")
                ));
            }
        }
        _ => return serde_json::to_string_pretty(env).unwrap_or_default(),
    }
    for w in env["warnings"].as_array().unwrap_or(&vec![]) {
        out.push(format!("  ! {}: {}", w["code"].as_str().unwrap_or(""), w["msg"].as_str().unwrap_or("")));
    }
    for c in env["commands"].as_array().unwrap_or(&vec![]) {
        out.push(format!("  $ {}", c.as_str().unwrap_or("")));
    }
    out.join("\n")
}

fn emit(env: &Value, code: i32, json: bool) -> ! {
    if json {
        println!("{}", serde_json::to_string_pretty(env).unwrap_or_default());
    } else {
        println!("{}", render_human(env));
        for e in env["errors"].as_array().unwrap_or(&vec![]) {
            eprintln!("error: {}: {}", e["code"].as_str().unwrap_or(""), e["msg"].as_str().unwrap_or(""));
        }
    }
    std::process::exit(code);
}

fn main() {
    // Parse. clap exits 0 on --help/--version; remap its bad-args exit to the
    // the user-input-error code (1) with an envelope.
    let cli = match Cli::try_parse() {
        Ok(c) => c,
        Err(e) => {
            use clap::error::ErrorKind;
            if matches!(e.kind(), ErrorKind::DisplayHelp | ErrorKind::DisplayVersion) {
                e.print().ok();
                std::process::exit(0);
            }
            let mut meta = Map::new();
            meta.insert("verb".into(), Value::Null);
            let env = envelope(false, vec![], meta, vec![], vec![], vec![err("USAGE", "invalid arguments; see --help")]);
            if std::io::stderr().is_terminal() {
                eprintln!("error: USAGE: invalid arguments; see --help");
            } else {
                println!("{}", serde_json::to_string_pretty(&env).unwrap_or_default());
            }
            std::process::exit(1);
        }
    };

    let json = wants_json(&cli.verb);

    // Totality: no backend panic reaches the user as an unmediated crash.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| dispatch(&cli.verb)));
    match result {
        Ok((env, code)) => emit(&env, code, json),
        Err(_) => {
            let mut meta = Map::new();
            meta.insert("verb".into(), Value::Null);
            let env = envelope(false, vec![], meta, vec![], vec![], vec![err("INTERNAL", "internal error (panic caught)")]);
            emit(&env, 3, json);
        }
    }
}
