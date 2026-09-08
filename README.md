# rf

`rf` (**r**ipgrep + **f**d) is an agent-friendly search envelope that fuses
ripgrep and fd behind a single, structured CLI — with cross-tool **stage
attribution** so a search can explain *why* a naive query missed (gitignore,
hidden files, binary skipping, encoding, deleted-in-history, and so on).

**Status: early development — in-process port underway.** `rf` now links the
[`ignore`](https://crates.io/crates/ignore) walker and
[`grep`](https://crates.io/crates/grep) searcher (ripgrep's and fd's own crates)
directly instead of shelling out, so each match retains full stage provenance
natively rather than reconstructing it after a pipe erases it.

Every verb emits one structured envelope (`rf capabilities` prints the machine
contract). What works today:

- **`content <pattern> <path>`** — content search that peels ripgrep's default
  filters as cumulative layers (vcs-ignore, hidden, binary, case) plus a parallel
  encoding probe (utf-16), and attributes every recovered file to the exact
  filter that hid it, with a paste-ready correction. Fully in-process.
- **`find <pattern> <path> --name <ext>`** — staged cross-source discovery. The
  fd name filters (`fd_name`/`fd_hidden`/`fd_ignore`) and ripgrep's binary skip
  (`rg_binary`) run in-process; git history (`git_deleted`) and, with
  `--structural PAT --lang LANG`, ast-grep (`ast_structural`) add two more
  independent sources. Each miss is attributed to the one stage that hid it.
- **`doctor <path>`** — reports the linked engine and the active ignore mode for
  the path (the context-flip: `.gitignore` only applies inside a git work tree).
- **`capabilities`** — the machine contract (verbs, flags, exit codes, warnings).

## License

Licensed under either of MIT or Apache-2.0 at your option.
