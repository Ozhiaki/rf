//! The `capabilities` verb — the machine contract, emitted as data. Layer-1
//! introspection: an agent reads this once and knows every verb, flag, exit
//! code, and warning code without trial and error.

use crate::envelope::{envelope, CONTRACT_VERSION, TOOL_VERSION};
use serde_json::{json, Map, Value};

pub fn build() -> Value {
    json!({
        "contract_version": CONTRACT_VERSION,
        "tool_version": TOOL_VERSION,
        "engine": "in-process (ignore + grep crates); no subprocess",
        "verbs": {
            "capabilities": {"summary": "emit this machine contract", "flags": ["--json"]},
            "content": {
                "summary": "content search; attributes each match to the filter that would hide it",
                "args": [
                    {"name": "pattern", "arity": 1, "type": "string"},
                    {"name": "path", "arity": 1, "type": "path", "default": "."}
                ],
                "flags": [{"name": "--json", "arity": 0, "type": "bool"}],
                "output_schema": {"data[]": {"file": "string", "surfaced_by": "enum[default,vcs_ignore,hidden,binary,case,encoding_utf16]"}}
            },
            "find": {
                "summary": "staged fd|rg pipe (in-process) + git history + ast-grep structural; attributes each miss to fd_name/fd_hidden/fd_ignore/rg_binary/git_deleted/ast_structural",
                "args": [
                    {"name": "pattern", "arity": 1, "type": "string"},
                    {"name": "path", "arity": 1, "type": "path", "default": "."}
                ],
                "flags": [
                    {"name": "--name", "arity": 1, "type": "string", "required": true, "domain": "file extension, no dot"},
                    {"name": "--structural", "arity": 1, "type": "string", "domain": "ast-grep pattern; a construct with no fixed literal form"},
                    {"name": "--lang", "arity": 1, "type": "string", "domain": "ast-grep language id; required with --structural"},
                    {"name": "--json", "arity": 0, "type": "bool"}
                ],
                "output_schema": {"data[]": {"file": "string", "stage": "enum[found,fd_name,fd_hidden,fd_ignore,fd_filter,rg_binary,git_deleted,ast_structural]", "fix": "string|null"}}
            },
            "doctor": {
                "summary": "environment DIAGNOSE: engine build, regex features, and the active ignore mode for a path",
                "args": [{"name": "path", "arity": 1, "type": "path", "default": "."}],
                "flags": [{"name": "--json", "arity": 0, "type": "bool"}]
            }
        },
        "exit_codes": {
            "0": {"meaning": "success (includes empty results: data:[])", "retryable": false},
            "1": {"meaning": "user-input-error (bad flags / missing args)", "retryable": false},
            "3": {"meaning": "tool-environment-error / internal", "retryable": false}
        },
        "warning_codes": [
            "IGNORE_VCS", "HIDDEN_SKIPPED", "BINARY_SKIPPED", "CASE_SENSITIVE",
            "ENCODING_MISS", "FD_NAME", "FD_HIDDEN", "FD_IGNORE", "RG_BINARY",
            "GIT_DELETED", "AST_STRUCTURAL", "STRUCTURAL_UNAVAILABLE", "IGNORE_MODE"
        ],
        "env_vars": ["SOURCE_DATE_EPOCH", "NO_COLOR"]
    })
}

pub fn run() -> (Value, i32) {
    let mut meta = Map::new();
    meta.insert("verb".into(), Value::from("capabilities"));
    (envelope(true, vec![build()], meta, vec![], vec![], vec![]), 0)
}
