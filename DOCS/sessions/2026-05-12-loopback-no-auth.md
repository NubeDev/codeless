# Build status — loopback = no auth, zero-paste first-run

> ⛔ **AGENT REMINDER — READ BEFORE TOUCHING THIS FILE**
>
> 1. JOB-LOOP spec: `DOCS/JOB-LOOP.md`. Project scope: `DOCS/SCOPE.md`.
>    Code-style rules: `CLAUDE.md`.
> 2. One logical batch per tick; verify + commit + push per stage.
> 3. ⛔ Schedule the next tick before exiting; report DONE if all `[x]`.
> 4. ⛔ Commit AND push every stage via mani; never --force, never --no-verify.
> 5. ⛔ Comments: why, not what. No emojis, no task-status, no banners.
> 6. ⛔ Cross-platform reach: UI imports `RpcClient` only.

File: DOCS/sessions/2026-05-12-loopback-no-auth.md
Goal: A user runs `codeless serve --fs-root .` + `pnpm dev`, opens the
      browser, and the demo works. No DevTools, no localStorage paste,
      no token. Non-loopback binds keep the bearer-token requirement
      (or refuse to boot).
Started: 2026-05-12
Last tick: 2026-05-12 19:00
Current stage: 5 / 5 — DONE

Repo:        codeless
Branch:      master
Scheduler:   CronCreate one-shot, ~1 min between ticks
Max ticks:   30

## Stages

- [x] 1. [M] `AppState` now carries `AuthMode::{Required, Open}`. The
       bearer middleware short-circuits on `Open`; SSE auth check
       skips the same way. CLI default: loopback bind → `Open` (no
       token); non-loopback without `--require-token` refuses to
       boot with an explanatory error. Three serve_cli tests
       updated/added covering: legacy bearer-required, loopback
       no-token unauth pass, non-loopback footgun guard.

- [x] 2. [S] UI auth header was already conditional on `token`
       being `Some` in `HttpSseClient`; the `readToken()` function
       returns null when nothing is configured, so the header simply
       isn't sent. SSE URL builder only appends `&token=` when
       set. No code change needed; behaviour now matches the
       server-side `Open` mode.

- [x] 3. [S] `readBaseUrl` defaults to `http://127.0.0.1:7777` when
       on a Vite dev port (1420 or 5173) with no explicit setting.
       Browser shell entry replaced its "default to mock on dev
       port" rule with a `/healthz` probe (1s timeout): real server
       reachable → `HttpSseClient`; not reachable → `MockRpcClient`
       on dev ports, `HttpSseClient` (with the resulting error
       visible) elsewhere.

- [x] 4. [S] DEMO-UI.md + codeless/README.md drop the localStorage
       paste from the happy path; non-loopback section documents
       `--require-token` for operators who need auth.

- [x] 5. [S] `scripts/smoke-demo.sh` now exercises both modes:
       default (no token, loopback) and `REQUIRE_TOKEN=1`. Both PASS
       against the new server.

## Notes
- R5 (Single-tenant trust boundary) explicitly allows this: same
  trust boundary as the user, loopback already restricts to
  same-host same-user processes. The token only made sense once
  the server might be reached from elsewhere.
- The bearer-token machinery is not removed — operators who bind
  to anything other than loopback still need it. The token is just
  optional on loopback now.

## Blockers
(none)

## Tick log
- Tick 1 (2026-05-12 19:00): all 5 stages. AppState.AuthMode plumbed
  through middleware and SSE; CLI gates non-loopback without
  --require-token. UI shell probes /healthz; readBaseUrl picks the
  conventional 127.0.0.1:7777 when on a Vite dev port. DEMO-UI.md
  and README updated. Smoke script covers both modes. End-to-end
  smoke verified: server-up + pnpm-dev + open browser = working UI,
  no DevTools paste.
