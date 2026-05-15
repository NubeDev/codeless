//! Seeded probes for the SESSION-MUTABLE-SCOPE Step 3 predicate
//! runner. Each submodule owns exactly one rule from `CLAUDE.md` (R1
//! crate-dependency direction, R2 comment hygiene) or the workspace
//! `DOCS/SCOPE.md` (the R2 boundary as it applies to the UI tree).
//!
//! Probes are pure functions over `&[ChangedFile]` so the integration
//! seam stays trivial: the runner reads files once, hands every probe
//! the same slice. A probe that wants to ignore a path skips it; the
//! probe never reaches out to the filesystem on its own.
//!
//! New probes belong here. Wire a new probe in `lib::run_all` so
//! `cargo test` exercises it and the binary picks it up automatically.

pub mod direct_fetch;
pub mod no_emojis;
pub mod no_task_status;
pub mod process_spawn;
pub mod tauri_imports;
