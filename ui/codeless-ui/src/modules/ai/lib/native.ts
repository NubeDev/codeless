// The legacy `native` surface — kept as a free-function module
// because its consumers are Zustand stores, registered AI tools, and
// transport setup that live outside React's component tree. Rather
// than plumb `RpcClient` through every one of them, the shell entry
// registers the single client at boot via `configureNative(rpc)`.
// Inside the trust boundary there is only one client (R5), so a
// module-level binding is safe; outside React context it is the only
// shape that doesn't require touching every caller.

import type { RpcClient } from "@/lib/rpc/client";
import type {
  ShellBgEntry,
  ShellBgLogChunk,
  ShellCommandOutput,
  ShellSessionRunOutput,
} from "@/lib/rpc/wire";

export type ReadResult =
  | { kind: "text"; content: string; size: number }
  | { kind: "binary"; size: number }
  | { kind: "toolarge"; size: number; limit: number };

export type DirEntry = {
  name: string;
  kind: "file" | "dir" | "symlink";
  size: number;
  mtime: number;
};

export type CommandOutput = ShellCommandOutput;

export type GrepHit = {
  path: string;
  rel: string;
  line: number;
  text: string;
};

export type GrepResponse = {
  hits: GrepHit[];
  truncated: boolean;
  files_scanned: number;
};

export type GlobHit = { path: string; rel: string };
export type GlobResponse = { hits: GlobHit[]; truncated: boolean };

let bound: RpcClient | null = null;

export function configureNative(rpc: RpcClient): void {
  bound = rpc;
}

function rpc(): RpcClient {
  if (!bound) {
    throw new Error(
      "native: RpcClient not configured. The shell entry must call configureNative(rpc) at boot.",
    );
  }
  return bound;
}

function relTo(root: string, path: string): string {
  const prefix = root.endsWith("/") ? root : `${root}/`;
  return path.startsWith(prefix) ? path.slice(prefix.length) : path;
}

export const native = {
  readFile: async (path: string): Promise<ReadResult> => {
    const r = await rpc().call("fs_read_file", { path, byte_limit: null });
    if (r.kind === "text") {
      return {
        kind: "text",
        content: r.content,
        size: new TextEncoder().encode(r.content).length,
      };
    }
    if (r.kind === "binary") {
      return {
        kind: "binary",
        size: Math.ceil((r.bytes_base64.length * 3) / 4),
      };
    }
    return { kind: "toolarge", size: r.size, limit: r.limit };
  },

  writeFile: async (path: string, content: string): Promise<void> => {
    await rpc().call("fs_write_file", {
      path,
      content,
      create_parents: false,
    });
  },

  createFile: async (path: string): Promise<void> => {
    await rpc().call("fs_create_file", {
      path,
      content: null,
      overwrite: false,
    });
  },

  createDir: async (path: string): Promise<void> => {
    await rpc().call("fs_create_dir", { path, recursive: true });
  },

  readDir: async (path: string): Promise<DirEntry[]> => {
    const { entries } = await rpc().call("fs_read_dir", { path });
    return entries.map((e) => ({
      name: e.name,
      kind: e.kind,
      size: e.size ?? 0,
      mtime: e.mtime ?? 0,
    }));
  },

  grep: async (params: {
    pattern: string;
    root: string;
    glob?: string[];
    caseInsensitive?: boolean;
    maxResults?: number;
  }): Promise<GrepResponse> => {
    // Wire `fs_search` takes a single glob filter; collapse a glob
    // array down to its first element for the mirror. The real Rust
    // adapter accepts the array directly; this matches the existing
    // API shape until the codegen step replaces native.ts entirely.
    const r = await rpc().call("fs_search", {
      root: params.root,
      query: params.pattern,
      case_sensitive: !(params.caseInsensitive ?? false),
      max_results: params.maxResults ?? null,
      glob: params.glob && params.glob.length ? params.glob[0] : null,
    });
    return {
      hits: r.hits.map((h) => ({
        path: h.path,
        rel: relTo(params.root, h.path),
        line: h.line,
        text: h.preview,
      })),
      truncated: r.truncated,
      files_scanned: 0,
    };
  },

  glob: async (params: {
    pattern: string;
    root: string;
    maxResults?: number;
  }): Promise<GlobResponse> => {
    const r = await rpc().call("fs_glob", {
      root: params.root,
      pattern: params.pattern,
      max_results: params.maxResults ?? null,
    });
    return {
      hits: r.hits.map((h) => ({
        path: h.path,
        rel: relTo(params.root, h.path),
      })),
      truncated: r.truncated,
    };
  },

  runCommand: (
    command: string,
    cwd?: string | null,
    timeoutSecs?: number,
  ): Promise<CommandOutput> =>
    rpc().call("shell_run", {
      command,
      cwd: cwd ?? null,
      timeout_secs: timeoutSecs ?? null,
    }),

  shellSessionOpen: (cwd?: string | null): Promise<number> =>
    rpc().call("shell_session_open", { cwd: cwd ?? null }),

  shellSessionRun: (
    id: number,
    command: string,
    cwd?: string | null,
    timeoutSecs?: number,
  ): Promise<ShellSessionRunOutput> =>
    rpc().call("shell_session_run", {
      id,
      command,
      cwd: cwd ?? null,
      timeout_secs: timeoutSecs ?? null,
    }),

  shellSessionClose: (id: number): Promise<null> =>
    rpc().call("shell_session_close", { id }),

  shellBgSpawn: (command: string, cwd?: string | null): Promise<number> =>
    rpc().call("shell_bg_spawn", { command, cwd: cwd ?? null }),

  shellBgLogs: (handle: number, sinceOffset?: number): Promise<ShellBgLogChunk> =>
    rpc().call("shell_bg_logs", {
      handle,
      since_offset: sinceOffset ?? null,
    }),

  shellBgKill: (handle: number): Promise<null> =>
    rpc().call("shell_bg_kill", { handle }),

  shellBgList: async (): Promise<ShellBgEntry[]> => {
    const { entries } = await rpc().call("shell_bg_list", {});
    return entries;
  },
};
