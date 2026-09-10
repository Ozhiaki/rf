# Crate coverage matrix

Enumerates every contract fact the crate surfaces (README.md, CHANGELOG.md) must
show, tier-classified and mapped, diffed against `capabilities.rc.json` (the
pinned rf 0.0.5 release-candidate fixture). Same tier rule as the site matrix
(`rgfind.dev:src/content/_contract/coverage-matrix.md`); the complete coverage
matrix is the merge of the two, assembled before the crate publish.

## Tier-P facts (crate surfaces)

| # | Contract fact | Fixture path | Surface / section | Claim type | Status / required action | Verification |
|---|---------------|--------------|-------------------|-----------|--------------------------|--------------|
| 1 | Verb set (7): `capabilities, conformance, content, doctor, find, robot-docs, robot-docs guide` | `data[0].verbs` | README "Verbs"; CHANGELOG released-commands line | complete set | **DEFECT CD1** - README lists 5; `robot-docs` and `robot-docs guide` absent (rf-guy.11) | diff README verb bullets vs fixture `verbs` |
| 2 | `robot-docs guide` is a released command in contract v2 | `data[0].verbs["robot-docs guide"]` (runs; emits workflow set, `version_range "2"`) | README "Contract version 2" para; CHANGELOG | presence claim | **DEFECT CD2** - both say "no workflow guide"; false (rf-guy.11) | assert prose does not deny the guide; verb present in fixture |
| 3 | `contract_version = 2` | `meta.contract_version`, `data[0].contract_version` | README "Contract version 2" heading; CHANGELOG header | single value | README OK; **DEFECT CD3** - CHANGELOG header `0.0.4 - contract version 2` labels v2 with a version already published at v1; must read `0.0.5` (rf-guy.11) | assert value 2; assert CHANGELOG version = 0.0.5 |
| 4 | Exit codes `0,1,3,5,6` (5 with `retryable`) | `data[0].exit_codes` | README "Contract version 2" para (exit 5) | reference | OK (exit 5 = snapshot conflict, retryable) | diff vs fixture exit set |
| 5 | Envelope: 7 top-level keys | top level | README "Contract version 2" para | complete set | OK (`commands,data,errors,meta,ok,tool_version,warnings`) | diff key set |
| 6 | Pagination: `--limit` default 100 range 1..=1000; `--cursor` opaque from `meta.pagination.cursor` | `data[0].verbs.{content,find}.flags` | README pagination para; CHANGELOG row-limit line | reference | OK (numbers 100/1000 fixture-backed) | diff limit default/range + cursor domain |
| 7 | `find` history coverage in `meta.history` (git-absent / non-work-tree / partial / failure distinct) | fixture `history` fields | README "Contract version 2" para; CHANGELOG | behavior | OK | assert prose matches fixture history model |
| 8 | `content.surfaced_by` layers: `default, vcs_ignore, hidden, binary, case, encoding_utf16` | `data[0].verbs.content.output_schema` | README `content` bullet | curated enum | OK (5 layers + utf-16 probe named) | diff vs fixture enum |
| 9 | `find.stage` enum (8): `found, fd_name, fd_hidden, fd_ignore, fd_filter, rg_binary, git_deleted, ast_structural` | `data[0].verbs.find.output_schema` | README `find` bullet | curated summary | **CD5 (minor)** - README names 6 of 8 (`found`, `fd_filter` absent); curated summary, defers to `capabilities`; the prose stage table is the site's job (rf-guy.6) | diff vs fixture enum |
| 10 | Install path `cargo install rf` | n/a (crates.io) | README "Install" | reference | OK | assert crate name |
| 11 | contract-source directive: read `rf capabilities --json` before automation | n/a | README, CHANGELOG | guidance | OK | present on surface |

## Tier-M facts (crate; not agent branch targets)

| Fact | Fixture path | Reason machine-only |
|------|--------------|---------------------|
| `data_hash`, `request_id`, `ts_iso` | `meta.*` | reproducibility/correlation/timestamp; not branched on |
| `parser_manifest` internals | `data[0].parser_manifest` | parser provenance; verb/flag facts already tier-P via `verbs{}` |
| per-run `meta` counts (`matched_files`, etc.) shown in README/CHANGELOG examples | not in `capabilities` | runtime meta; illustrative only, not contract-guaranteed keys |

## Known crate-surface defects (rows)

- **CD1 - README verb list incomplete.** Add `robot-docs` and `robot-docs guide`
  to the "Verbs" section. Fix in rf-guy.11.
- **CD2 - "no workflow guide" false.** README's "Contract version 2" paragraph and
  the CHANGELOG both state no workflow guide ships; `robot-docs guide` is a
  released, working command. Remove the statement. Fix in rf-guy.11.
- **CD3 - CHANGELOG version label wrong.** Header reads `0.0.4 - contract version 2`;
  0.0.4 is already published at contract version 1. The v2 release ships as
  `0.0.5`. Fix in rf-guy.11.
- **CD4 - binary `--help` contradicts capabilities (CODE, cross-surface).** The
  installed RC binary's top-level `--help` prints "Workflow guides: none are
  released in contract version 2", while `capabilities` lists `robot-docs guide`
  and the command runs. This is a code-level string defect, not a doc edit; fixing
  it is a code change that forces an RC re-capture. Tracked as a separate bead
  (discovered-from rf-guy.10); resolve or explicitly defer before publish so the
  shipped product is self-consistent.
- **CD5 - README find stages partial.** README names 6 of 8 `find.stage` values;
  acceptable as a curated summary that defers to `capabilities`. The complete
  prose stage table is the site's find page (rf-guy.6).
