## Done

- Marked BROWSER-LAUNCHER.md M5 (EventFilter sum-type + subscribe scoping) and M6 (list_jobs + fs.* threading) as `[x]`, with a one-line note on each pointing at what actually landed (`fs_root_for_repo` helper, reject path for unknown/detached `repo_id`).
- Marked M4 (tab-local storage contract) and M10 (UI shell-detection + deep-link router) as `[~]` partial, naming the deviation: M4 moved the tab-level state (assistant focus thread, open job tabs) per DOCS/STORAGE-AUDIT.md but the ESLint/CI grep is still open; M10 landed the `?workspace=<repo_id>` deep-link router and `history.replaceState` flow, the shell-detection flip and fragment-token bootstrap defer to M7 + M9.
- Added a "Workspace scoping on the wire" section to DOCS/UI-ARCHITECTURE.md describing the per-tab `activeRepoId`, the `EventFilter ::= Repo { repo_id } | Library | All` (deprecated) wire shape, parallel Library subscription for the picker, and file-explorer rehydration on `activeRepoId` change.
- Verified the three rules hold on this branch's diff: R1 — `git diff master..HEAD` contains no new `tokio::process` or `std::process` usage outside `codeless-adapters-host`; R2 — no new `@tauri-apps/*` imports outside `src/shells/<shell>/` (the only hits in `src/` are pre-existing — `lib/rpc/tauri-ipc-client.ts` and terminal `pty-bridge.ts` / `useTerminalSession.ts`, both already documented as blocked on Rust); R3 — diff touches existing files in their existing concept.
- Committed as `9b93219 stage 10: final cleanup …` on `codeless/workspace-scoping`.

## Next

- (none) — this is the final stage. The branch is ready for the dogfood gate described in BROWSER-LAUNCHER.md §"Dogfood gate (A complete)".

## What you need to know

- The job goal text reads "milestones 5, 6, and 9", but per the current BROWSER-LAUNCHER.md numbering M9 is "Launcher mode (Linux)" (Tauri shell + xdg-open + tray icon), which this job did not touch. The work actually completed maps to M5, M6, and M10 (UI deep-link router); plus M4 partially. The deviation is recorded in the milestone bullets themselves, not just here, so a fresh reader picks it up.
- M5 and M6 are now closed; the dogfood gate (Project A complete) is open and waits on the team running two browser tabs against two attached workspaces for a week. The follow-on Project B/C work (M7 security hardening, M8 data-dir revert, M9 launcher, the rest of M10) is unstarted.
- DOCS/STORAGE-AUDIT.md is the durable artifact for the storage-contract work; the ESLint/CI grep that M4 calls for is still open.

## Open questions

- Should the deprecated `EventFilter::All` variant be removed before M11's exit test, or kept indefinitely for the global event log view? The branch keeps it; BROWSER-LAUNCHER.md §"RPC additions" should decide.
- The M10 fragment-token bootstrap is sequenced behind M7 (security hardening); confirm that order still holds before the launcher work starts, since the launcher milestone (M9) also wants to deliver the token over the URL fragment.
