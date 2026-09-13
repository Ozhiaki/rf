# Crate coverage matrix (SUPERSEDED)

This hand-maintained prose matrix is superseded by the machine-readable
`plumbline.json` (at the repo root) and the guard that runs it (`plumb check`,
from [`plumbline`](https://github.com/rgfind/plumbline)). The config pins every
tier-P contract fact to a field in `capabilities.rc.json`; the guard asserts
agreement on every push and pull request and before packaging. The promise "the
docs agree with the binary" is now a check, not a document.

The defects this matrix enumerated (CD1–CD5) were resolved before the 0.0.5
publish:

- **CD1 / CD2 / CD3** — the README verb list, the false "no workflow guide"
  statement, and the wrong CHANGELOG version label were fixed in the README/
  CHANGELOG reconciliation (rf-guy.11).
- **CD4** — the top-level `--help` line that contradicted `capabilities` was fixed
  in code (rf-guy.13), forcing the RC re-capture that produced the shipped 0.0.5
  fixture.
- **CD5** — the crate README names a curated subset of `find` stages and defers the
  full eight-value table to `rf capabilities`; the complete prose table is the
  site's find page (rf-guy.6).

The one drift this registry caught on first run: `data[0].contract_version` is the
JSON string `"2"`, not an integer. The registry pins it as a string, and the site
`json-contract.md` was corrected to match.

Retained only as history. Do not edit; add or change registered facts in
`coverage-registry.json`.
