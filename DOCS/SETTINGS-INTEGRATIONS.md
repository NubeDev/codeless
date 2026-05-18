# SETTINGS-INTEGRATIONS — Scope

Status: draft
Owner: ap@nube-io.com
Created: 2026-05-18

## Summary

Today, enabling Codeless features means CLI flags and a hand-edited
secrets file: `--fs-root` for the working directory,
`--enable-telegram` plus a `telegram_bot_token` in the secrets store
for the Telegram adapter. The server has to be restarted to change
either. That's fine for the developer running ticks, impossible for a
non-CLI user.

This document specifies a **Settings → Integrations** surface that
lets the user enable, configure, and disable runtime features from
the UI, with no CLI and no restart. Two integrations land here first:
**Workspaces** (already server-complete, picking up
[`WORKSPACE-ATTACH.md`](./WORKSPACE-ATTACH.md) milestone 4) and
**Telegram** (server-side hot-toggle refactor + first UI panel).
Plugins land here later, once
[`PLUGIN-SUBSTRATE.md`](./PLUGIN-SUBSTRATE.md) items 1–6 exist.

> **Sister docs.** [`WORKSPACE-ATTACH.md`](./WORKSPACE-ATTACH.md)
> §"UX — the Workspaces surface" already specifies the Workspaces
> panel's contents — this doc only pins where it lives and how it
> hangs off the Settings shell. [`UI-ARCHITECTURE.md`](./UI-ARCHITECTURE.md)
> defines the `RpcClient` boundary every panel stays behind.
> [`SCOPE-TELEGRAM-INTEGRATION.md`](./SCOPE-TELEGRAM-INTEGRATION.md)
> defines what the Telegram adapter *does*; this doc adds the
> runtime-toggle plumbing. Where any of those disagree with this doc,
> **they win** — open an issue and update this file.

## Goals

1. The user enables Workspaces and Telegram from a Settings tab. No
   CLI flag, no restart.
2. Settings → Integrations renders a panel per integration. Each
   panel owns its own configuration UX (the Workspaces panel is the
   one already designed; the Telegram panel is small — token, chat
   id, on/off, status).
3. Integration state is persisted in SQLite (R4). Boot replays it.
4. The CLI flags (`--fs-root`, `--enable-telegram`) stay as
   **bootstrap conveniences** — they upsert into the same persisted
   state and otherwise behave identically.
5. Same UX on browser and desktop. Mobile (Phase 6) gets a read-only
   view of which integrations are on.

## Non-goals

- A unified `CapabilityDescriptor` abstraction. With two integrations
  the duplication is small; extract it later if a third (plugins)
  shows the same shape. See §"Why not a unified capability model
  yet".
- A new top-level `/integrations` route. Settings is the surface.
- Per-integration auth scopes. Single trust boundary (R5).
- Importing arbitrary OAuth / Slack-style provider catalogues. Each
  integration is a hand-written panel.

## What ships

### 1. New Settings tab: `integrations`

[`ui/codeless-ui/src/lib/shell/settings-window.ts`](../codeless/ui/codeless-ui/src/lib/shell/settings-window.ts)
adds `"integrations"` to the `SettingsTab` union;
[`SettingsApp.tsx`](../codeless/ui/codeless-ui/src/settings/SettingsApp.tsx)
adds a tab between **Models** and **Agents** (`Plug` / `Workflow`
icon from Hugeicons). The tab renders an `IntegrationsSection` that
delegates to one panel per integration:

```
Settings
├ General
├ Shortcuts
├ Models
├ Integrations   ← new
│   ├ Workspaces      (panel — see WORKSPACE-ATTACH.md M4)
│   └ Telegram        (panel — see §3 below)
├ Agents
└ About
```

Panels are listed via a static array in `IntegrationsSection.tsx`,
not discovered dynamically. Adding a third panel is one import + one
array entry. No registry abstraction.

### 2. Workspaces panel

The content is the **Workspaces page** already specified in
[`WORKSPACE-ATTACH.md`](./WORKSPACE-ATTACH.md) §"UX — the Workspaces
surface". This doc only pins:

- It renders inside the Settings → Integrations → Workspaces panel
  for milestone 1.
- The `/workspaces` route and sidebar group (M4–M5 of
  WORKSPACE-ATTACH) ship later, sharing the same components. The
  panel and the route are **the same React tree** mounted in
  different locations — no duplication.
- The `+ Attach` button, attach modal, validator, detach modal, and
  health badges are all already specced; this doc adds no new UX for
  Workspaces.

Picking this up unblocks WORKSPACE-ATTACH M3 (`RpcClient` pickup)
into M4 (Workspaces page). M3's remaining work is enumerated in
[`WORKSPACE-ATTACH.md`](./WORKSPACE-ATTACH.md) §"Milestones" item 3
and is a prerequisite for this panel.

### 3. Telegram panel

The smallest possible panel. One screen, four fields, two RPCs.

```
┌─ Telegram ─────────────────────────────────────────────┐
│ Status: connected as @codeless_demo_bot                │
│                                                         │
│ Bot token:   ••••••••••••••••••••••••••••  [edit]      │
│ Chat ID:     -1001234567890                            │
│ Outbound:    [x] post failure cards                    │
│                                                         │
│ Enabled:     ( ●─── ) on                               │
│                                                         │
│         [Test connection]    [Save]                    │
└────────────────────────────────────────────────────────┘
```

States rendered inline:
- `disabled` — toggle is off; fields editable; no live connection.
- `enabled, healthy` — green status line, last polled timestamp.
- `enabled, error` — red banner with the error from `getMe` /
  long-poll (e.g. `401 Unauthorized`), fields stay editable so the
  user can fix and retry without disabling first.

#### RPC

```rust
#[derive(Serialize, Deserialize, specta::Type)]
pub struct TelegramStatus {
    pub enabled: bool,
    pub chat_id: Option<String>,
    pub outbound_failure_cards: bool,
    pub bot_username: Option<String>,
    pub last_error: Option<String>,
    pub last_polled_at: Option<UnixMillis>,
}

#[derive(Serialize, Deserialize, specta::Type)]
pub struct ConfigureTelegramArgs {
    pub enabled: bool,
    pub bot_token: Option<String>,        // `None` keeps stored token
    pub chat_id: Option<String>,
    pub outbound_failure_cards: bool,
}

// routes: /rpc/get_telegram_status, /rpc/configure_telegram
```

`configure_telegram` is the single mutation verb — enable/disable,
edit fields, all one call. It writes the new values to the existing
`SecretStore` (so the on-disk format stays identical to the current
`--enable-telegram` boot path), then start/stops the supervisor (§4).

`get_telegram_status` never leaks the bot token. The UI receives a
masked indicator and edits via the *same* `configure_telegram` call
that takes an opaque new token.

### 4. Server-side: Telegram supervisor

Today the bot is constructed once at boot in
[`crates/codeless-cli/src/serve.rs:506-529`](../codeless/crates/codeless-cli/src/serve.rs#L506-L529)
and its handle is dropped (the `let _telegram = …` line). We need it
to live in `ServerState` and respond to runtime toggle calls.

```rust
// crates/codeless-server/src/state.rs
pub struct TelegramSupervisor {
    bot: Mutex<Option<TelegramBot>>,
    store: Arc<SecretStore>,
    rpc:   Arc<dyn RpcServer>,
}

impl TelegramSupervisor {
    pub async fn enable(&self) -> Result<(), TelegramError>;
    pub async fn disable(&self) -> Result<(), TelegramError>;
    pub async fn reconfigure(&self) -> Result<(), TelegramError>;
    pub async fn status(&self) -> TelegramStatus;
}
```

`enable` reads the current `SecretStore`, calls `TelegramBot::spawn`,
stashes the handle. `disable` takes the handle out and awaits
`bot.shutdown()` — which already drains both inbound and outbound
tasks ([`crates/codeless-telegram/src/lib.rs:129`](../codeless/crates/codeless-telegram/src/lib.rs#L129)).
`reconfigure` is `disable` then `enable` so token / chat-id edits
take effect cleanly. Boot path: if the persisted `enabled` row is
`true`, call `enable()` once startup finishes, equivalent to the
current `--enable-telegram` behaviour.

The `--enable-telegram` CLI flag becomes a **boot-time upsert** of
`enabled = true` into the same persisted state, exactly like
`--fs-root` upserts into `attached_workspaces`. The flag stays for
demo / scripts; the UI is the source of truth.

### 5. Persistence

New table:

```sql
CREATE TABLE integration_state (
    integration TEXT PRIMARY KEY,   -- 'telegram' for now; 'plugin:<id>' later
    enabled     INTEGER NOT NULL,
    config_json TEXT NOT NULL,       -- integration-specific blob; opaque to schema
    updated_at  INTEGER NOT NULL     -- UnixMillis
);
```

The `config_json` blob is shape-checked by the integration, not by
the schema. Workspaces does **not** use this table — it already has
`attached_workspaces` (which is the strictly-better shape for that
domain). The Workspaces panel reads `list_workspaces` directly.

`integration_state` is the place plugin-on/off rows will land later
when [`PLUGIN-SUBSTRATE.md`](./PLUGIN-SUBSTRATE.md) item 6 (manifest
reader) exists.

## Cross-cutting rules (must hold)

- **R1**: `TelegramSupervisor` lives in `codeless-server`, which is
  host-only; nothing the supervisor calls touches a mobile-safe
  crate's dependency graph. The bot itself already spawns processes
  only via the existing adapter path.
- **R2**: panels import `RpcClient` only — no `@tauri-apps/*`, no
  direct `fetch`. The Workspaces panel reuses its already-typed
  methods; the Telegram panel adds two.
- **R3**: one component tree. The Workspaces panel and the
  `/workspaces` route are the same React subtree, mounted differently.
- **R4**: state in SQLite (`attached_workspaces`, `integration_state`).
  UI subscribes to the existing event bus for live updates; it does
  not cache authoritative state.
- **R5**: bearer token authorises `get_telegram_status` and
  `configure_telegram` identically to every other RPC.

## Why not a unified capability model yet

The previous proposal sketched a `CapabilityDescriptor` +
`list_capabilities` / `configure_capability` pair. Three reasons it's
deferred:

1. **Two integrations don't justify it.** Workspaces and Telegram
   have almost nothing in common at the data layer — workspaces is
   an N-row attached set with a per-row picker UX; Telegram is a
   single global on/off with four scalar fields. Forcing both behind
   a generic `config_schema` shape would either lose the
   per-workspace richness or carry dead weight for Telegram.
2. **Plugins isn't ready.** The third integration that *would* share
   structure is plugins, but
   [`PLUGIN-SUBSTRATE.md`](./PLUGIN-SUBSTRATE.md) items 1–6 aren't
   landed. Designing the abstraction now means guessing the third
   case. Once plugins lands and we have three concrete panels, the
   shared shape (if any) will be obvious.
3. **R4 says: state in SQLite.** A generic `configure_capability`
   RPC pushes the integration-specific shape into a JSON blob and
   relies on each integration's runtime to validate it. That's
   strictly worse for Workspaces, which already has a typed schema.

If the duplication across the third panel actually justifies it,
extract `CapabilityDescriptor` as a refactor at that point, not now.

## Migration / backwards compat

- `--enable-telegram` becomes "set `integration_state.telegram.enabled
  = true` on boot if not already set, then call
  `TelegramSupervisor::enable`". The flag stays. No script changes.
- `--fs-root` already documented as a bootstrap convenience in
  [`WORKSPACE-ATTACH.md`](./WORKSPACE-ATTACH.md) §"Migration /
  backwards compat". Unchanged.
- Existing `telegram_bot_token` / `telegram_chat_id` secrets file
  keys keep working. The UI writes to them through `SecretStore`;
  the format is unchanged.

## Edge cases — explicit decisions

- **User disables Telegram while jobs are mid-flight that emitted
  outbound cards.** Outbound publisher is part of the bot; `shutdown`
  drains it ([`crates/codeless-telegram/src/lib.rs:135`](../codeless/crates/codeless-telegram/src/lib.rs#L135)).
  Cards already queued send; new events after disable are dropped on
  the floor with a one-line warn-log. No buffering across toggles.
- **`configure_telegram` with `enabled=true` but no token.**
  Validation error returned before any `SecretStore` write; the
  toggle does not flip.
- **Token rotation while enabled.** `configure_telegram` writes the
  new token, then calls `reconfigure()` which is disable+enable. A
  brief window (~1s) of dropped updates is acceptable; Telegram
  long-poll resumes from the last update id, no duplication.
- **Workspaces panel viewed before WORKSPACE-ATTACH M3 finishes.**
  Renders a "Workspaces not available — RPC client missing methods"
  placeholder. The Settings tab still works; only that one panel
  is empty. This is the natural sequencing fence between the two
  scopes, not a special case.

## Milestones

Status legend: `[x]` done, `[~]` partial, `[ ]` not started.

1. `[ ]` **Settings shell wiring.** Add `"integrations"` to
   `SettingsTab` and an empty `IntegrationsSection.tsx`. Tab visible,
   no panels yet. Round-trips through deep-link (`?tab=integrations`).
   _Size: S._

2. `[ ]` **WORKSPACE-ATTACH M3 finish.** Land the four `RpcClient`
   methods + the `PathPicker` shell capability + both shell
   injectors. Picks up WORKSPACE-ATTACH directly; this milestone
   exists here only as a dependency marker.
   _Size: M._

3. `[ ]` **Workspaces panel.** Mount the WORKSPACE-ATTACH M4 component
   tree inside the Integrations tab. Empty state, attach modal,
   detach modal, list. RTL happy-path test (attach → list → detach).
   _Size: M._

4. `[ ]` **Telegram supervisor.** Add `TelegramSupervisor` to
   `ServerState`. Replace the boot `let _telegram = …` with a call
   into the supervisor. CLI flag becomes a boot-time upsert. Unit
   test: enable → disable → enable cycles cleanly without leaking
   tasks.
   _Size: M._

5. `[ ]` **Telegram RPC + panel.** Add `get_telegram_status` /
   `configure_telegram` to `RpcServer`, route them in the server,
   generate TS wire types, ship the panel. Test-connection button
   calls `getMe` via the supervisor. RTL happy-path test
   (configure → enable → status reports connected).
   _Size: M._

6. `[ ]` **Integration events.** Existing `workspace_*` events stay
   as-is. Add `telegram_status_changed` ride-along on the same
   event bus so the panel updates live without polling.
   _Size: S._

Total: 1 S, 4 M, 1 S — sized to land in ~6 ticks at the JOB-LOOP
rhythm.

## Open questions

1. **Should the Telegram panel show recent inbound messages / a
   one-shot test-send button?** Bias: no. The panel proves
   *configuration*; observing what the bot is doing belongs in a
   future Telegram-specific surface (or just the Telegram client
   itself).
2. **Where does the panel registry live when plugins lands — still a
   static array, or read from plugin manifests?** Bias: static array
   for built-ins (Workspaces, Telegram), dynamic append for
   `integration_state` rows whose `integration` key starts with
   `plugin:`. Resolve when PLUGIN-SUBSTRATE item 6 (manifest reader)
   has a concrete shape.
3. **Does mobile render the Integrations tab at all in Phase 6, or
   hide it?** Bias: render read-only (status pills, no edit). Defer
   the decision to Phase 6 kickoff.
