## Done

- New `plugins` module in `codeless-server` with `PluginCatalog` + projection types, bearer-gated `GET /plugins` listing, per-plugin `GET /plugins/<id>/ui/*` ServeDir mounts (no auth, matching host-UI-bundle posture). Opt-in via `AppState::with_plugins`.
- `tower-http` `fs` feature added; 5 integration tests in `crates/codeless-server/tests/plugins_routes.rs` (404-without-catalog, 401-without-bearer, JSON projection + ui_dir-not-leaked, ServeDir round-trip, per-id scope). Existing 14 routes.rs tests still pass.
- Committed as `41fd2fe` with the required stage-11 title prefix.

## Next

- Stage 12: per the workflow this is the final UI-side stage before the M-UI REVIEW gate (notes plugin's AssistantPanel remote, end-to-end against the host shell). Fresh session will pick it up.

## What you need to know

- `AppState.plugins` is `Option<Arc<PluginCatalog>>`. The CLI `serve` path leaves it `None` today — host wiring that builds a catalog from the substrate's `PluginRegistry` is not part of stage 11 (the docs only specify the server surface). When the CLI wires it later, each entry needs `ui_dir = Some(<plugin_dir>/ui)` and `contributes_ui = ui_dir.join("mf-manifest.json").exists()`.
- `slots: Vec<String>` is populated by the host (the manifest `[contributes.ui]` block lands in stage 13). Today every entry just ships an empty vec; the JSON shape and route layout already accept it.
- ServeDir mounts are wired with `nest_service` per concrete id at router build time. Hot-reload is intentionally out of scope (consistent with the rest of the substrate — static-link at startup).
- Pre-existing infra gotcha (not committed): `/home/user/.codeless/worktrees/ai-runner/Cargo.toml` has its `workspace = "..."` pointer pinned to another worktree (`job-01KRXH0RYTT6EYGF435WPQS70Q`); I temporarily repointed it to this job's worktree to build/test, then restored it. Any future stage that needs `cargo build` from inside this worktree will hit the same wall and needs the same one-line nudge.

## Open questions

- (none)
