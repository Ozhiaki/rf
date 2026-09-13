# Releasing rf

crates.io is **write-once**: once a version is published, its README, metadata,
and code can never be changed for that version. So the documentation and the
code must agree **at the exact moment of publishing** — not before, not after.

The single rule: **never run `cargo publish` unless `plumb preflight`
exits 0.** The preflight is the stop sign. It refuses to pass unless everything
that ships is in lock step.

The guard is [`plumbline`](https://github.com/rgfind/plumbline), an installed
CLI invoked as `plumb` (not an in-repo tool). Install it once, then it reads
`plumbline.json` at the repo root:

```sh
cargo install --git https://github.com/rgfind/plumbline.git plumbline --locked
```

## The one command

```sh
plumb preflight
```

It runs five gates and fails if any one does not hold:

1. **Worktree clean and committed.** `cargo publish` packages the working tree,
   so any uncommitted change could ship un-reviewed. The tree must be clean.
2. **The fixture matches the binary.** It rebuilds `rf`, runs
   `rf capabilities --json`, and confirms the committed contract fixture is
   identical (contract-equivalent) to that fresh output. If the code's contract
   changed, this fails until the fixture is recaptured.
3. **The docs match the fixture.** Every registered contract fact still equals
   its fixture field, and every generated block on a doc surface has a known id
   (an id with no renderer to keep it fresh is rejected).
4. **Packaged files stay within the allowlist.** Every file `cargo package`
   would ship is a source file, an allowlisted doc, or cargo's own metadata — so
   no guard tool, cargo config, test fixture, or CI file leaks into the archive.
5. **The README example matches the binary.** It renders the recovery example
   from a real run of the freshly built binary and confirms the committed block
   in `README.md` is byte-identical. A hand-edit that drifts from real output
   fails this gate.

Green on all five means the published version's docs will describe exactly what
its binary does.

## Release steps

1. Land all code and doc changes for the version. If the contract changed, run
   `plumb capture` to recapture the fixture, reconcile the docs against it,
   and commit.
2. Bump the version in `Cargo.toml`; update `CHANGELOG.md`. Commit.
3. Run the stop sign:

   ```sh
   plumb preflight
   ```

   Fix anything it reports. Do not continue until it prints
   `preflight: OK — every gate passed; safe to cargo publish`.
4. Publish:

   ```sh
   cargo publish
   ```

5. Tag the release and push.

## What the stop sign does not yet cover

- **Interpretive prose** (the explanatory text around the generated tables) is
  human-reviewed, not machine-checked. Read it when the contract changes. This
  includes the hand-written `--json` envelope beside the README recovery example:
  gate 5 regenerates the human render, but the JSON snippet next to it stays
  hand-kept, so re-read it when the `content` output shape changes.
