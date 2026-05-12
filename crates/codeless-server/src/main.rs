//! Binary entry point. The `codeless serve` verb on the main
//! `codeless` CLI is the supported way to run this server; this
//! stand-alone binary exists so `cargo run -p codeless-server` still
//! produces a usable executable for direct integration tests.
//!
//! Wiring (DB open, port bind, secrets path resolution) lives behind
//! the CLI verb. Until that lands this prints a hint and exits 1 so
//! a stray `cargo run` does not silently appear to succeed.

fn main() {
    eprintln!(
        "codeless-server: use the `codeless serve` CLI verb to start the server. \
         This binary is a placeholder so the crate builds standalone."
    );
    std::process::exit(1);
}
