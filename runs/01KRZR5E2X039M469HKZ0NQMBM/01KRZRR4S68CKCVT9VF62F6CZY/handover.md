## Done

- Wrote `.codeless/jobs/job-export/BUNDLE-DESIGN.md` — the frozen design lock for the `.codeless-job` bundle: file shape (gzipped tar, UTF-8, deterministic ordering), directory layout, `manifest.json` schema_version 1 field contract, per-Run `run.json` + JSONL stream rules, the secrets denylist regex set (`token`, `secret`, `api[_-]?key`, `pass(word|wd)`, `private[_-]?key`, `credential`, `bearer`, `auth[_-]?header`; column-name substring, case-insensitive), the size cap constants table (200 MiB bundle / 10 MiB entry plus per-kind sub-caps, 1024 runs/bundle, 500k events/run), open-question resolutions, refuse-to-export preconditions, README outline, and what stages 3–7 inherit.
- Extended `DOCS/sessions/2026-05-19-job-export.md` with the stage 2 record and locked-answer summary.
- Rewrote `handover.md` to brief stage 3 (the REVIEW gate) on what to present to the reviewer.
- Committed as `stage 2: design bundle layout and manifest.json schema_version 1 …` on `codeless/job-export`.

## Next

- Stage 3 — REVIEW gate. Read-only paper. Stage 3's handover should quote the three RPC signatures from `DOCS/SCOPE-JOB-EXPORT.md` §"RPC surface" and the conflict-policy enum (only `Refuse` wired in E1) alongside `BUNDLE-DESIGN.md` so the reviewer has one packet. Reviewer approves or asks for changes; stage 3 amends `BUNDLE-DESIGN.md` in place if requested.

## What you need to know

- JOB-WORKFLOW (B) is still **not merged** — re-verified this session (`crates/codeless-runtime/migrations/` has no `runs` migration). Stage 1 remains `[!]`. Stages 2 and 3 are paper and proceeded; stages 4–7 stay blocked on (B). The unblock checklist in the prior handover (now in git history; see commit `8086d97`) is unchanged.
- Locked open-question answers: size caps per the §5 table; `output_path` jailed under `attached_workspaces.fs_root_canonical`; `events.cursor` monotonic via SQLite `AUTOINCREMENT`; imported handover lands on `jobs.handover_md`; payload-content scanning out for E1; `scheduled_pause_points` out for E1.
- The denylist applies to **column names only**, not values. Currently zero schema columns match — the list is prospective.
- Branch is `codeless/job-export`; the worktree pushes via mani per repo `CLAUDE.md` once code starts landing. Stage 2 was committed via raw `git` because it is doc-only and no mani push is needed mid-stage; the next code-bearing stage should switch to `./bin/mani run commit/push --projects codeless` from the workspace root.

## Open questions

- Stage 3 reviewer may push back on cap defaults, denylist breadth (esp. whether to add a `key` catch-all or scan `events.payload` content), or the OQ-E call to leave `scheduled_pause_points` out. All three are explicitly flagged for review in `BUNDLE-DESIGN.md` §§5–7.
- Tar/gzip crate choice (`tar` + `flate2` vs. alternatives) is deliberately not locked; that's a stage-4 call.
