# Release-candidate contract fixture

`capabilities.rc.json` is the byte-for-byte output of `rf capabilities --json`
from the packaged-and-installed release candidate. It is captured, never
hand-edited. It is the source of truth the crate docs (README, CHANGELOG) are
reconciled against — distinct from the site's published-artifact fixture.

## Capture procedure (shared packaged-artifact procedure)

1. Commit the version bump so the worktree is clean (packaging forbids a
   dirty-worktree override).
2. `cargo package --package rf` — build the archive for the crate member.
3. Extract the archive and install from the extracted manifest into a temporary
   cargo root: `cargo install --path <extracted>/rf-0.0.5 --root <tmproot> --locked`.
4. Run `<tmproot>/bin/rf capabilities --json` and `<tmproot>/bin/rf conformance`
   from the installed binary — never a source-tree `cargo run`.

## Pinned release candidate

- Next package version: `rf 0.0.5` (contract version 2)
- Source commit: `aa6b9b74c14b014eab5b970626878013f6dc3f07` (cargo `.cargo_vcs_info.json`)
- Capture content hash (sha256): `261dbd7d9ee8eff28c6224244c8f537b69c58bdc7a5a0be0bc3d1afc8b45c919`
- Package archive (sha256): `f2c30df87354800512f2d4990c47eb0f69a559ef89ba4f4ba0013a80eefb89c8`
  (`target/package/rf-0.0.5.crate`, 20 files) — the verification package built at
  source commit `aa6b9b7`.
- Installed binary: `<tmproot>/bin/rf`, installed from the extracted `rf-0.0.5`
  package manifest (not the working tree)
- Conformance at capture: profile `release-self-check`, 28 pass / 5 not-applicable
  / 0 fail (green)

Committing this re-capture moves HEAD past `aa6b9b7`, so `cargo publish`
re-packages at the final release commit. Every *included* file is byte-identical
to the verification package (this fixture and its README are excluded from the
archive); only the auto-generated `.cargo_vcs_info.json` records a different
commit sha, so the uploaded archive's hash differs from the one above by that one
field. Re-run the packaged-artifact procedure at the exact publish commit if a
matching archive hash is required.

## Validity

Prose edits (README, CHANGELOG) keep this fixture valid, because they do not
change what the binary emits. A code or contract change after this capture
invalidates the fixture and forces a re-capture and re-verify before publish.

## Re-capture history

- Captured at `aa10addf` (rf-guy.9) — first RC capture.
- Re-captured at `aa6b9b7` (rf-guy.12, final pre-publish) — re-anchored provenance
  after the pre-publish code and packaging changes:
  - `75d34bb` (rf-guy.13) — fixed the top-level `--help` workflow-guide line (CLI
    help text only).
  - `1af9de1` + `aa6b9b7` — restricted the published archive to a root-anchored
    `include` allowlist (drops `.beads/`, `bench/`, internal contract-QA files).
  Verified the machine contract is unchanged across the re-capture: normalized
  `capabilities --json` (dropping per-run `request_id`, `ts_iso`, `data_hash`,
  `elapsed_ms`) is byte-identical to the prior fixture. Conformance green
  (28/0/5). None of these changes touch the compiled binary's contract; the
  re-capture re-anchors source commit, capture hash, and archive hash only.

This fixture is current for the staged publish commit. Any further code or
contract change forces another re-capture and re-verify before publish.
