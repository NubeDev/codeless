## Done

- added `EventFilter::Repo { repo_id }` and `EventFilter::Library` variants to `crates/codeless-rpc/src/subscribe.rs`; `All` retained with a doc-only legacy note for the global event-log view (no `#[deprecated]` attribute — would cascade `-D warnings` across the existing tests and `codeless tail`)
- extended `SubscribeFilter` in `crates/codeless-runtime/src/event_bus.rs` with `Repo(RepoId)` / `Library`; fan-out resolves via payload `repo_id` (six library payloads + `JobQueued`) and a `job -> repo` map snapshot of `jobs.repo_id` taken at subscribe time and folded forward live from `JobQueued`
- wired the in-process router (`crates/codeless-runtime/src/rpc/mod.rs`), the HTTP `GET /events` handler (`crates/codeless-server/src/sse.rs`), and the HTTP-client URL builder (`crates/codeless-client/src/http_client.rs`) onto the new variants — `scope=repo&repo_id=…` and `scope=library`
- regenerated the wire snapshot (`crates/codeless-rpc/tests/wire-rpc.ts.snap`) and the UI binding (`ui/codeless-ui/src/lib/rpc/generated/wire.ts`); updated the hand-mirrored UI types in `ui/codeless-ui/src/lib/rpc/methods.ts` and the `buildSubscribeUrl` mapping in `ui/codeless-ui/src/lib/rpc/http-sse-client.ts` in the same change-set so the wire ends cannot drift
- added the unit test `crates/codeless-runtime/tests/event_filter_repo_library.rs` that drives two `MockRunner`-backed jobs across two repos plus a synthetic-id assistant publish; asserts the per-repo and library subscribers partition the stream correctly, including the live `JobQueued -> job-repo map` arming the downstream runner events rely on
- `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, and the new test all green; full `cargo test --workspace --no-fail-fast` flagged one pre-existing flake (`-p codeless-mcp --test prompts::list_and_get_prompts_via_stdio` — bin-spawn `No such file or directory` from a parallel-build artifact race) that passes when run in isolation
- committed as `c9b375d` on `codeless/workspace-scoping` titled `stage 3: add Repo { repo_id } and Library variants to EventFilter`

## Next

- (none) — fresh session picks up stage 4 of 10

## What you need to know

- the `RpcServer::subscribe` trait signature is unchanged (still `EventFilter` + `Since`); only the variant set grew. Existing callers using `EventFilter::All` / `Job` keep compiling and behave identically
- the live-tail's `job_repos` map is a per-stream `HashMap<JobId, RepoId>` snapshot loaded once at subscribe and folded forward on `JobQueued`. Per-event SQL would serialise the broadcast tail, which is why the map exists. Memory cost is bounded by the `jobs` row count — the workspace is single-tenant and small
- the `ai-runner` sibling crate's `workspace = "../job-…"` pointer was stale (pointed at a deleted worktree) — patched to point at this worktree so `cargo build` succeeds. This is an out-of-repo file (`/home/user/.codeless/worktrees/ai-runner/Cargo.toml`); the patch is not in the codeless commit and will need to be re-patched in any fresh worktree until the workspace tooling repoints it automatically
- `mani` was not available in this isolated worktree so the commit went through plain `git`, not `mani run commit`. The branch is `codeless/workspace-scoping`; push is deferred to whatever path the loop driver uses

## Open questions

- assistant + unbound-chat envelopes still use a synthetic `job_id` that does not resolve through `jobs`. Stage 3 routes them to `Library` by construction (a `job_id` not in the snapshot map matches `Library`). Later stages must decide whether to thread a real `repo_id` onto `assistant_threads` (audit Family B) or keep this contract; the filter doesn't force the choice
