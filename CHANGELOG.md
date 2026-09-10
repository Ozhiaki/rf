# Changelog

## 0.0.5 — contract version 2

This release changes the machine contract.

- The JSON envelope has seven top-level keys. Failed requests use `data: null`.
- Exit code 5 means a paged result snapshot changed. Restart with the emitted command.
- `content` and `find` return at most 100 rows by default and 1000 rows at most.
- `find` reports Git history coverage and does not hide unavailable history as an empty result.
- `rf robot-docs guide` emits ready-to-run agent workflow recipes. Capabilities lists all released commands.
