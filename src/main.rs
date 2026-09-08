//! rf — a search envelope that fuses ripgrep and fd for agent use.
//!
//! This is an early name-claiming stub. The working prototype today is a
//! Python wrapper (see the project's `search.py`); this crate is the Rust
//! implementation, which begins in earnest at the "in-process cutover" —
//! the point where rf links the `ignore`, `grep`, and `walkdir` crates
//! directly instead of shelling out, so every match keeps its full stage
//! provenance natively.

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() {
    let arg = std::env::args().nth(1);
    match arg.as_deref() {
        Some("--version") | Some("-V") => println!("rf {VERSION}"),
        _ => {
            println!("rf {VERSION} — early development stub.");
            println!("Planned: an agent-friendly envelope over ripgrep + fd with");
            println!("cross-tool stage attribution for false-negative forensics.");
            println!("Not yet functional. Run `rf --version` for the version.");
        }
    }
}
