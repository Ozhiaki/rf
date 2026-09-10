//! rf — an agent-first forensic search envelope that fuses ripgrep and fd.
//!
//! In-process design: the `ignore` walker and `grep` searcher (ripgrep's and
//! fd's own crates) run linked in-process, so every match keeps its full stage
//! provenance natively instead of being reconstructed from a shell pipe. Every
//! verb emits the universal machine-first envelope; see `capabilities`.

mod capabilities;
mod command;
mod conformance;
mod content;
mod doctor;
mod engine;
mod envelope;
mod fault;
mod find;
mod manifest;
mod pagination;

use clap::{Parser, Subcommand};
use envelope::{envelope, err};
use serde_json::{Map, Value};
use std::io::IsTerminal;
use std::time::Instant;

#[derive(Parser)]
#[command(name = "rf", version, about = "agent-first forensic search over ripgrep + fd", disable_help_subcommand = true, after_help = "Machine contract: rf capabilities --json\nAutomation: read the JSON envelope before you use a follow-up command.\nExit: 0 success; 1 input error; 3 environment error; 5 snapshot conflict; 6 internal error.\nWorkflow guides: none are released in contract version 2.")]
struct Cli {
    /// Emit the machine-readable envelope. Accepted before or after a verb.
    #[arg(long, global = true)]
    json: bool,
    /// Disable terminal decoration. rf currently emits no ANSI decoration, but
    /// the accepted global flag is part of the stable parser contract.
    #[arg(long = "no-color", global = true)]
    no_color: bool,
    #[command(subcommand)]
    verb: Verb,
}

fn extension(value: &str) -> Result<String, String> {
    if value.is_empty() || value.contains('.') || value.contains('/') || value.contains('\\') {
        Err("file extension must be non-empty and contain no dot or path separator".into())
    } else {
        Ok(value.into())
    }
}

#[derive(Subcommand)]
enum Verb {
    /// Emit the machine contract.
    Capabilities {
    },
    /// Content search with per-filter attribution.
    Content {
        pattern: String,
        #[arg(default_value = ".")]
        path: String,
        #[arg(long, default_value_t = pagination::DEFAULT_LIMIT, value_parser = pagination::limit)]
        limit: usize,
        #[arg(long)]
        cursor: Option<String>,
    },
    /// Staged cross-source discovery (port in progress).
    Find {
        pattern: String,
        #[arg(default_value = ".")]
        path: String,
        #[arg(long)]
        #[arg(value_parser = extension)]
        name: String,
        #[arg(long)]
        structural: Option<String>,
        #[arg(long)]
        lang: Option<String>,
        #[arg(long, default_value_t = pagination::DEFAULT_LIMIT, value_parser = pagination::limit)]
        limit: usize,
        #[arg(long)]
        cursor: Option<String>,
    },
    /// Diagnose the environment and active ignore mode.
    Doctor {
        #[arg(default_value = ".")]
        path: String,
    },
    /// Run the release self-check profile against this binary.
    Conformance {
    },
}

fn bootstrap_json() -> bool {
    // This scan is deliberately lexical and stops at `--`: a later `--json` is
    // data, not a global option. It decides the error-rendering mode before
    // clap can reject malformed argv.
    std::env::args().skip(1).take_while(|a| a != "--").any(|a| a == "--json")
        || !std::io::stdout().is_terminal()
}

fn dispatch(v: &Verb) -> (Value, i32) {
    match v {
        Verb::Capabilities { .. } => capabilities::run(),
        Verb::Content { pattern, path, limit, cursor, .. } => content::run(pattern, path, *limit, cursor.as_deref()),
        Verb::Find { pattern, path, name, structural, lang, limit, cursor, .. } => {
            find::run(pattern, path, name, structural.as_deref(), lang.as_deref(), *limit, cursor.as_deref())
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

fn emit(mut env: Value, code: i32, json: bool, started: Instant) -> ! {
    if let Some(meta) = env.get_mut("meta").and_then(Value::as_object_mut) {
        // A fixed source epoch is the explicit reproducible-output mode. It
        // freezes the timing field too, so fixtures can compare complete JSON.
        let elapsed = if std::env::var_os("SOURCE_DATE_EPOCH").is_some() {
            0
        } else {
            started.elapsed().as_millis() as u64
        };
        meta.insert("elapsed_ms".into(), Value::from(elapsed));
    }
    if !env["ok"].as_bool().unwrap_or(false) {
        if let Some(errors) = env.get_mut("errors").and_then(Value::as_array_mut) {
            for error in errors {
                if let Some(object) = error.as_object_mut() {
                    object.insert("exit_code".into(), Value::from(code));
                }
            }
        }
    }
    if json {
        println!("{}", serde_json::to_string_pretty(&env).unwrap_or_default());
        if let Some(message) = env["errors"][0]["message"].as_str() {
            eprintln!("error: {message}");
        }
    } else {
        println!("{}", render_human(&env));
        for e in env["errors"].as_array().unwrap_or(&vec![]) {
            eprintln!("error: {}: {}", e["code"].as_str().unwrap_or(""), e["message"].as_str().unwrap_or(""));
        }
    }
    std::process::exit(code);
}

fn main() {
    let started = Instant::now();
    let json = bootstrap_json();
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
            let code = match e.kind() {
                ErrorKind::UnknownArgument => "UNKNOWN_FLAG",
                ErrorKind::InvalidSubcommand => "UNKNOWN_COMMAND",
                ErrorKind::InvalidValue | ErrorKind::ValueValidation => "INVALID_INPUT",
                ErrorKind::MissingRequiredArgument => "MISSING_ARGUMENT",
                _ => "USAGE",
            };
            let rendered = e.to_string();
            let token = rendered.split('`').nth(1).or_else(|| rendered.split('\'').nth(1));
            let suggestion = if std::env::args().skip(1).any(|arg| arg == "--") { None } else { token.and_then(manifest::correction) };
            let mut problem = err(code, "invalid arguments; see --help");
            if let Some(suggestion) = suggestion {
                problem.as_object_mut().unwrap().insert("did_you_mean".into(), Value::from(suggestion.clone()));
                let command = crate::command::shell("rf", &[suggestion]);
                let env = envelope(false, vec![], meta, vec![], vec![command], vec![problem]);
                emit(env, 1, json, started);
            }
            let env = envelope(false, vec![], meta, vec![], vec![], vec![problem]);
            emit(env, 1, json, started);
        }
    };

    // Totality: no backend panic reaches the user as an unmediated crash.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| dispatch(&cli.verb)));
    match result {
        Ok((env, code)) => emit(env, code, json || cli.json, started),
        Err(_) => {
            let mut meta = Map::new();
            meta.insert("verb".into(), Value::Null);
            let env = envelope(false, vec![], meta, vec![], vec![], vec![err("INTERNAL", "internal error (panic caught)")]);
            emit(env, 6, json || cli.json, started);
        }
    }
}
