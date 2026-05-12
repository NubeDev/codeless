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
