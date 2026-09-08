# rf

`rf` (**r**ipgrep + **f**d) is an agent-friendly search envelope that fuses
ripgrep and fd behind a single, structured CLI — with cross-tool **stage
attribution** so a search can explain *why* a naive query missed (gitignore,
hidden files, binary skipping, encoding, deleted-in-history, and so on).

**Status: early development.** This crate is currently a name-claiming stub.
The working prototype is a Python wrapper; the Rust implementation begins at the
"in-process cutover" — where `rf` links the [`ignore`](https://crates.io/crates/ignore),
[`grep`](https://crates.io/crates/grep), and [`walkdir`](https://crates.io/crates/walkdir)
crates directly instead of shelling out, so each match retains full stage
provenance natively rather than reconstructing it after a pipe erases it.

## License

Licensed under either of MIT or Apache-2.0 at your option.
