## Done

- Introduced `SecretBackend` trait in `crates/codeless-adapters-host/src/secrets.rs`; `SecretStore` is now a facade over a `Box<dyn SecretBackend>` with an in-memory `BTreeMap` view.
- Kept `TomlSecretBackend` (the XDG TOML file with atomic write + fsync + 0600) as the default; `SecretStore::open(path)` still constructs it unchanged so every existing call site compiles without edits.
- Added `KeyringSecretBackend` behind a non-default `keyring` Cargo feature, talking to Secret Service / Keychain / Credential Manager via `keyring` v3, with a `__codeless_index__` entry to enumerate stored keys. Reachable through `SecretStore::open_keyring(service)`.
- Re-exported `SecretBackend`, `TomlSecretBackend`, and (under feature) `KeyringSecretBackend` from the crate root.
- Added the crash-ordering exit test in `crates/codeless-adapters-host/tests/secrets.rs`: the parent spawns itself with `--exact crash_helper_child` and `CODELESS_SECRETS_CRASH_HELPER_PATH`; the child saves a secret and `std::process::abort()`s before any restart signal could fire; the parent reopens the file and asserts the value is durable.
- Verified `cargo build -p codeless-adapters-host` (default and `--features keyring`), `cargo test -p codeless-adapters-host --test secrets` (7/7 pass), `cargo clippy -p codeless-adapters-host --all-targets -- -D warnings` clean, and `cargo fmt`.
- Committed as `stage 4: extend the SecretStore behind a SecretBackend trait…`.

## Next

- Stage 5 picks up the next item from `DOCS/WORKSPACE-ATTACH.md` §"TODO — adapter registry" (likely the RPC surface: `list_chat_adapters`, `set_chat_adapter_enabled`, `validate_chat_adapter_secrets`, `list_runners`, `set_runner_enabled`).
- Eventually wire a CLI / settings-page choice between the TOML and keyring backends through `serve.rs` so the keyring path is reachable from end users, not just constructors.

## What you need to know

- `ai-runner` lives outside the worktree at `../ai-runner/Cargo.toml` and its `workspace = "../job-…"` line is pinned to a sibling worktree id. To build inside *this* worktree I temporarily pointed it at `../job-01KRYQJVK0G60MEZVFQ6KW3Y1F` and reverted before commit; do the same swap if you need to `cargo build` locally.
- A pre-existing compilation error in `crates/codeless-cli/src/serve.rs` (test module `use super::compose_system_prompt;` resolves to nothing) was inherited from stage 2 — `cargo clippy --workspace --all-targets` currently fails on `codeless-cli` test build for that reason. Not introduced here; flag it to whoever owns stage 2 follow-up.
- `SecretStore::path()` was dropped; callers should use `SecretStore::location()` (returns the TOML path or `keyring:<service>`). No existing caller used `path()` outside the module, so nothing else needed touching.
- The keyring backend never gets exercised in CI (no Secret Service in headless containers); it is compile-tested via `cargo build --features keyring` only. The TOML backend is the tested path.

## Open questions

- The "config flag" wording in `WORKSPACE-ATTACH.md` step 3 is satisfied by `SecretStore::open_keyring(service)` as a constructor; whether stage 1 also wants a `--secrets-backend=keyring|toml` CLI flag (or a SQLite row) is left to whichever stage rewires the boot path in `codeless-cli/src/serve.rs`.
- `KeyringSecretBackend::save` does a full overwrite (delete-then-write each key + index). If the index entry ever desyncs from real entries — e.g. a partial save crashed mid-loop — orphan secrets could linger in the OS keychain. Worth a follow-up either to make save transactional (write index last) or to add a `repair_index` admin verb; out of scope for stage 4.
