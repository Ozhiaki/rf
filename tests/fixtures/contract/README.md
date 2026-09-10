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
- Source commit: `aa10addf49ab4c9706acbb38fdc03fe0cedda2eb` (cargo `.cargo_vcs_info.json`)
- Capture content hash (sha256): `569cf76cc0f63f9376586d0620da621efcc40e5d5194302e98dc0a4987e0647a`
- Installed binary: `<tmproot>/bin/rf`, installed from the extracted `rf-0.0.5`
  package manifest (not the working tree)
- Conformance at capture: profile `release-self-check`, 28 pass / 5 not-applicable
  / 0 fail (green)

No final archive hash is recorded here; the shippable package is built and hashed
at publish time (see the final-package verification step).

## Validity

Prose edits (README, CHANGELOG) keep this fixture valid, because they do not
change what the binary emits. A code or contract change after this capture
invalidates the fixture and forces a re-capture and re-verify before publish.

## Invalidation log (pending re-capture)

- `75d34bb` (rf-guy.13) — fixed the top-level `--help` workflow-guide line. CLI
  help text only; the `capabilities` envelope is unchanged (verified: normalized
  `capabilities --json` is byte-identical to this fixture apart from per-run
  volatile fields — `request_id`, `ts_iso`, `data_hash`, `elapsed_ms`). This
  fixture's pinned source commit (`aa10addf`) now predates HEAD, so provenance is
  stale even though contract content matches.

The re-capture runs once at the final pre-publish capture step (rf-guy.12). It
re-anchors provenance for `75d34bb` only; the guardrail bead (rf-guy.7) was
re-sequenced to land after publish, so no further pre-publish code change is
pending. Until the re-capture, this fixture stays contract-valid but
provenance-stale; the publish gate (rf-guy.12) forbids shipping without a fresh
capture.
