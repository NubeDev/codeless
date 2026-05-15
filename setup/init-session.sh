#!/usr/bin/env bash
# init-session.sh — bring up a codeless server with sane per-user paths.
#
# Layout (override root with CODELESS_HOME):
#   $CODELESS_HOME/codeless.db       single SQLite source of truth
#   $CODELESS_HOME/worktrees/        per-job git worktrees
#   $CODELESS_HOME/logs/server.log   server stdout+stderr
#   $CODELESS_HOME/server.pid        background-mode pid
#
# Secrets stay at the XDG default (~/.config/codeless/secrets.toml) so
# `codeless secrets …` keeps working unchanged.
#
# See setup/GETTING-STARTED.md for the why.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CODELESS_REPO="$(cd "$SCRIPT_DIR/.." && pwd)"

CODELESS_HOME="${CODELESS_HOME:-$HOME/.codeless}"
DB_PATH="$CODELESS_HOME/codeless.db"
WORKTREE_ROOT="$CODELESS_HOME/worktrees"
LOG_DIR="$CODELESS_HOME/logs"
LOG_FILE="$LOG_DIR/server.log"
PID_FILE="$CODELESS_HOME/server.pid"

BIND="${CODELESS_BIND:-127.0.0.1:7777}"
FS_ROOT="${CODELESS_FS_ROOT:-}"
RUNNERS="${CODELESS_RUNNERS:-claude}"   # comma list: claude,anthropic,codex,copilot
DRIVER_CONCURRENCY="${CODELESS_DRIVER_CONCURRENCY:-4}"

usage() {
  cat <<EOF
usage: $(basename "$0") <command> [args]

commands:
  start [--bg]            start the server (default: foreground)
  stop                    stop a backgrounded server
  status                  show server / port / db status
  reset                   wipe \$CODELESS_HOME (asks for confirmation)
  add-repo <name> <path>  register a local repo via RPC
  list-repos              list repos via RPC
  paths                   print resolved paths and exit

env:
  CODELESS_HOME           default: \$HOME/.codeless
  CODELESS_BIND           default: 127.0.0.1:7777
  CODELESS_FS_ROOT        default: <added repo's local_path>, or unset
  CODELESS_RUNNERS        default: claude  (comma list: claude,anthropic,codex,copilot)
  CODELESS_DRIVER_CONCURRENCY  default: 4
EOF
}

ensure_dirs() {
  mkdir -p "$CODELESS_HOME" "$WORKTREE_ROOT" "$LOG_DIR"
}

build_serve_args() {
  local args=(
    --db "$DB_PATH"
    serve
    --bind "$BIND"
    --worktree-root "$WORKTREE_ROOT"
    --driver-concurrency "$DRIVER_CONCURRENCY"
  )
  if [[ -n "$FS_ROOT" ]]; then
    args+=(--fs-root "$FS_ROOT")
  fi
  IFS=',' read -ra runners <<<"$RUNNERS"
  for r in "${runners[@]}"; do
    case "$r" in
      claude)    args+=(--enable-claude) ;;
      anthropic) args+=(--enable-anthropic) ;;
      codex)     args+=(--enable-codex) ;;
      copilot)   args+=(--enable-copilot) ;;
      mock|"")   ;;
      *) echo "unknown runner: $r (allowed: claude,anthropic,codex,copilot,mock)" >&2; exit 2 ;;
    esac
  done
  printf '%s\n' "${args[@]}"
}

server_running() {
  [[ -f "$PID_FILE" ]] && kill -0 "$(cat "$PID_FILE")" 2>/dev/null
}

cmd_paths() {
  cat <<EOF
CODELESS_HOME   $CODELESS_HOME
db              $DB_PATH
worktrees       $WORKTREE_ROOT
log             $LOG_FILE
pid             $PID_FILE
bind            $BIND
fs_root         ${FS_ROOT:-(unset; pass --fs-root or set CODELESS_FS_ROOT)}
runners         $RUNNERS
repo (cargo)    $CODELESS_REPO
EOF
}

cmd_start() {
  ensure_dirs
  local bg=0
  [[ "${1:-}" == "--bg" ]] && bg=1

  if server_running; then
    echo "server already running (pid $(cat "$PID_FILE"))" >&2
    exit 1
  fi

  mapfile -t serve_args < <(build_serve_args)

  echo "→ starting codeless server"
  echo "  db:        $DB_PATH"
  echo "  worktrees: $WORKTREE_ROOT"
  echo "  bind:      $BIND"
  echo "  runners:   $RUNNERS"
  [[ -n "$FS_ROOT" ]] && echo "  fs-root:   $FS_ROOT"

  cd "$CODELESS_REPO"
  if [[ "$bg" == 1 ]]; then
    nohup cargo run --quiet -p codeless-cli --bin codeless -- "${serve_args[@]}" \
      >"$LOG_FILE" 2>&1 &
    echo $! >"$PID_FILE"
    echo "→ pid $(cat "$PID_FILE"), tail with: tail -f $LOG_FILE"
  else
    exec cargo run -p codeless-cli --bin codeless -- "${serve_args[@]}"
  fi
}

cmd_stop() {
  if ! server_running; then
    echo "no running server (no pid file or process gone)"
    rm -f "$PID_FILE"
    return 0
  fi
  local pid
  pid="$(cat "$PID_FILE")"
  echo "→ stopping pid $pid"
  kill "$pid" 2>/dev/null || true
  for _ in 1 2 3 4 5; do
    kill -0 "$pid" 2>/dev/null || break
    sleep 0.5
  done
  kill -0 "$pid" 2>/dev/null && kill -9 "$pid" || true
  rm -f "$PID_FILE"
  echo "→ stopped"
}

cmd_status() {
  cmd_paths
  echo
  if server_running; then
    echo "server: RUNNING (pid $(cat "$PID_FILE"))"
  else
    echo "server: not running"
  fi
  if command -v ss >/dev/null 2>&1; then
    ss -tlnp 2>/dev/null | grep -E ":${BIND##*:}\b" || true
  fi
}

cmd_reset() {
  if [[ ! -e "$CODELESS_HOME" ]]; then
    echo "$CODELESS_HOME does not exist; nothing to reset"
    return 0
  fi
  read -rp "wipe $CODELESS_HOME ? [y/N] " ans
  [[ "$ans" =~ ^[Yy]$ ]] || { echo "aborted"; exit 1; }
  if server_running; then cmd_stop; fi
  rm -rf "$CODELESS_HOME"
  echo "→ removed $CODELESS_HOME"
}

cmd_add_repo() {
  local name="${1:?name required}" path="${2:?absolute repo path required}"
  path="$(cd "$path" && pwd)"
  curl --fail-with-body -sS -X POST "http://${BIND}/rpc/add_repo" \
    -H 'content-type: application/json' \
    -d "$(printf '{"name":"%s","clone_url":"","default_branch":"master","local_path":"%s","git_auth":{"kind":"token","env_var":"GITHUB_TOKEN"},"concurrency_cap":null,"default_runner":"claude"}' "$name" "$path")" \
    | python3 -m json.tool
}

cmd_list_repos() {
  curl --fail-with-body -sS -X POST "http://${BIND}/rpc/list_repos" \
    -H 'content-type: application/json' -d '{}' | python3 -m json.tool
}

cmd="${1:-}"; shift || true
case "$cmd" in
  start)      cmd_start "$@" ;;
  stop)       cmd_stop ;;
  status)     cmd_status ;;
  reset)      cmd_reset ;;
  add-repo)   cmd_add_repo "$@" ;;
  list-repos) cmd_list_repos ;;
  paths)      cmd_paths ;;
  ""|-h|--help) usage ;;
  *) usage; exit 2 ;;
esac
