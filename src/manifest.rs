//! Parser-derived surface manifest (probeability rule X-01).
//!
//! `capabilities` is hand-kept prose: summaries, types, domains, output schemas —
//! things the parser does not know. This manifest is read straight from clap's
//! parse tree instead, so the two are *independent* sources describing the same
//! surface. The conformance verb reconciles them (X-01): a verb or flag added to
//! the CLI without updating `capabilities`, or removed from one side only, makes
//! X-01 fail rather than letting the published contract drift from the real
//! parser. Think double-entry bookkeeping — the hand ledger and the machine
//! ledger must reconcile or something is wrong.

use clap::{ArgAction, CommandFactory};
use serde_json::{Map, Value};

/// clap adds these to every (sub)command automatically; they are declared once in
/// capabilities.global_flags, not per verb, so they are not part of the diff.
fn is_auto(id: &str) -> bool {
    id == "help" || id == "version"
}

/// Build the manifest by walking `Cli`'s clap parse tree: each verb's long flags
/// (with arity) and positional argument names, sorted for determinism.
pub fn build() -> Value {
    let cmd = crate::Cli::command();
    let mut verbs = Map::new();
    for sub in cmd.get_subcommands() {
        let mut flags: Vec<Value> = Vec::new();
        let mut positionals: Vec<String> = Vec::new();
        for arg in sub.get_arguments() {
            if is_auto(arg.get_id().as_str()) {
                continue;
            }
            if arg.is_positional() {
                positionals.push(arg.get_id().as_str().to_string());
            } else if let Some(long) = arg.get_long() {
                // num_args is not populated on an un-built Command, so derive arity
                // from the action: only Set/Append consume a value.
                let arity = matches!(arg.get_action(), ArgAction::Set | ArgAction::Append) as i64;
                let mut m = Map::new();
                m.insert("name".into(), Value::from(format!("--{long}")));
                m.insert("arity".into(), Value::from(arity));
                flags.push(Value::from(m));
            }
        }
        positionals.sort();
        flags.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
        let mut vm = Map::new();
        vm.insert("flags".into(), Value::from(flags));
        vm.insert(
            "positionals".into(),
            Value::from(positionals.into_iter().map(Value::from).collect::<Vec<_>>()),
        );
        verbs.insert(sub.get_name().to_string(), Value::from(vm));
    }
    let mut root = Map::new();
    root.insert("source".into(), Value::from("clap parse tree (CommandFactory)"));
    root.insert("verbs".into(), Value::from(verbs));
    Value::from(root)
}
