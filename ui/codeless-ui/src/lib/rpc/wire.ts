// Core wire types are re-exported from `./generated/wire`, which is
// produced by `cargo run -p codeless-rpc --example wire_ts` from the
// Rust source of truth in `codeless-types` (core domain) and
// `codeless-rpc` (subscribe filter, method args). Do not hand-edit
// those shapes here — change the Rust types and regenerate.
//
// The fs/shell/secrets forward declarations below sit alongside the
// generated module because their Rust counterparts do not exist yet.
// As each surface lands on the Rust side, its types move into
// `codeless-types` and disappear from this file.

export * from "./generated/wire";

import type { UnixMillis } from "./generated/wire";

// Forward-declared filesystem wire types. The runtime never streams
// raw file contents through the RPC channel — anything past the inline
// size budget returns `kind: "toolarge"` and the client fetches via a
// follow-up streaming channel.

export type FsKind = "file" | "dir" | "symlink";

export interface FsEntry {
  name: string;
  kind: FsKind;
  size: number | null;
  mtime: UnixMillis | null;
}

export type FsReadResult =
  | { kind: "text"; content: string; encoding: "utf-8" }
  | { kind: "binary"; bytes_base64: string }
  | { kind: "toolarge"; size: number; limit: number };

export interface FsGrepHit {
  path: string;
  line: number;
  column: number;
  preview: string;
}

export interface FsGlobHit {
  path: string;
  kind: FsKind;
}

// Forward-declared shell wire types. The PTY *streaming* channel does
// not pass through these — it uses the dedicated WebSocket reserved
// for PTY sessions. These shapes cover the one-shot run, the
// foreground "session" (sequential commands with preserved cwd), and
// the background-process surface.

export interface ShellCommandOutput {
  stdout: string;
  stderr: string;
  exit_code: number | null;
  timed_out: boolean;
  truncated: boolean;
}

export interface ShellSessionRunOutput {
  stdout: string;
  stderr: string;
  exit_code: number | null;
  timed_out: boolean;
  truncated: boolean;
  cwd_after: string;
}

export interface ShellBgLogChunk {
  bytes: string;
  next_offset: number;
  dropped: number;
  exited: boolean;
  exit_code: number | null;
}

export interface ShellBgEntry {
  handle: number;
  command: string;
  cwd: string | null;
  started_at_ms: number;
  exited: boolean;
  exit_code: number | null;
}

// Forward-declared plugin substrate wire types. The Rust side
// (`codeless-rpc` / `codeless-types`) does not yet emit these shapes;
// they live here so the UI host shell can call `rpc.call("list_plugins", {})`
// at boot and degrade to the empty list when the server returns a
// "method not found" error. When the Rust counterpart lands (stage 13
// — `[[runtimes]]` + `[contributes.ui]` manifest parsing), these
// move into `codeless-types` and disappear from this file.

/** Mirror of one row in `[[contributes.ui.exposes]]` from a plugin's
 *  `plugin.toml`. The slot id is the fully-resolved string the host
 *  passes to `<PluginSlot id=... />` — the SDK parses it. */
export interface PluginUiExposeEntry {
  name: string;
  module: string;
  slot: string;
}

/** Mirror of one plugin's `[contributes.ui]` block, plus enough
 *  ambient context for the host to register an MF remote: the entry
 *  url is server-resolved so the UI never assembles the path itself
 *  unless `contributes_ui` is true. */
export interface PluginUiContribution {
  /** Fully-qualified URL of the plugin's `mf-manifest.json`, served
   *  by `codeless-server` at `/plugins/<id>/ui/mf-manifest.json`.
   *  Null when the plugin ships no UI (`contributes_ui = false`). */
  mf_manifest_url: string | null;
  exposes: PluginUiExposeEntry[];
}

/** One enabled plugin row as the host shell needs it. The plugin
 *  registry on the server owns the truth; this is the projection the
 *  UI consumes from `rpc.call("list_plugins", {})`. */
export interface PluginListEntry {
  id: string;
  version: string;
  /** MF remote name. Conventionally `id`; carried explicitly so the
   *  host shell never has to re-derive it. */
  remote_name: string;
  contributes_ui: boolean;
  ui: PluginUiContribution | null;
}

export interface ListPluginsResult {
  plugins: PluginListEntry[];
}
