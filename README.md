# rf

A naive `rg pattern` silently skips gitignored files, hidden files, and binary
files. On a working tree it can miss most of the matches and still exit 0, so a
caller thinks the search succeeded. `rf` (**r**ipgrep + **f**d) finds those
matches and tells you which filter hid each one.

**Status: early development.** The command surface and the JSON envelope below
work today; expect breaking changes before 1.0.

## Install

```sh
cargo install rf
```

## Example

A repo with the search term in three places — a tracked file, a hidden file,
and a gitignored file:

```
config.py          timeout = 30
.env.local         timeout = 5      # hidden
cache/build.py     timeout = 999    # gitignored
```

Plain ripgrep finds one of the three. `rf` finds all three and names the filter
that hid each:

```
$ rf content timeout .
content 'timeout' in .: 3 file(s), 1 by default, 2 hidden by filters
  hidden       .env.local
  vcs_ignore   cache/build.py
  default      config.py
  ! IGNORE_VCS: 1 match(es) hidden by default; add -u (ignore .gitignore/.ignore rules)
  ! HIDDEN_SKIPPED: 1 match(es) hidden by default; add -uu (also search hidden/dotfiles)
  $ rg -u 'timeout' .
  $ rg -uu 'timeout' .
```

Each miss carries a paste-ready correction. Run at a terminal, `rf` prints the
summary above; piped or with `--json`, it prints one structured envelope:

```sh
rf content timeout . --json
```

```json
{
  "ok": true,
  "data": [
    { "file": ".env.local",     "surfaced_by": "hidden" },
    { "file": "cache/build.py", "surfaced_by": "vcs_ignore" },
    { "file": "config.py",      "surfaced_by": "default" }
  ],
  "meta": { "matched_files": 3, "default_matched_files": 1, "hidden_by_filters": 2 }
}
```

The `warnings` array (elided above) carries one entry per miss, each with a
warning code and the paste-ready `rg` command that surfaces it.

## Verbs

Every verb emits the same envelope. Run `rf capabilities` for the full machine
contract (verbs, flags, exit codes, warning codes).

- **`rf content <pattern> <path>`** — content search that peels ripgrep's
  default filters as layers (vcs-ignore, hidden, binary, case) plus an encoding
  probe (utf-16), and attributes every recovered file to the filter that hid it.
- **`rf find <pattern> <path> --name <ext>`** — staged discovery across four
  independent sources. The fd name filters (`fd_name`, `fd_hidden`, `fd_ignore`)
  and ripgrep's binary skip (`rg_binary`) run in-process; git history
  (`git_deleted`) recovers matches scrubbed from the tree; with
  `--structural PAT --lang LANG`, ast-grep (`ast_structural`) finds matches that
  have no fixed literal form. Each miss is attributed to the one stage that hid
  it.
- **`rf doctor <path>`** — reports the linked engine and whether `.gitignore` is
  active for the path. (`.gitignore` applies only inside a git work tree, so the
  same search can answer differently in a scratch dir and a real repo.)
- **`rf capabilities`** — the machine contract as JSON.

## How it works

`rf` links the [`ignore`](https://crates.io/crates/ignore) walker and the
[`grep`](https://crates.io/crates/grep) searcher — ripgrep's and fd's own crates
— directly, rather than shelling out to the `rg` and `fd` binaries. Each file's
membership in a filter is read natively as the walk runs, so a match keeps its
stage provenance instead of losing it when a shell pipe erases fd's exit code
and conflates ripgrep's "no files" with "no match." git history and ast-grep are
external oracles with no Rust binding, so `rf` runs them as subprocesses; each
contributes nothing (rather than failing) when its tool is absent.

## License

Licensed under the Apache License, Version 2.0. See [LICENSE](LICENSE).
