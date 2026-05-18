#!/usr/bin/env bash
# Cross-compile codeless-tauri-desktop for Linux and/or Windows.
# Run from anywhere — the script resolves paths relative to itself.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# Workspace root is 4 levels up: docker/ -> codeless-tauri-desktop/ -> crates/ -> codeless/ -> codeless-workspace/
WORKSPACE_ROOT="$(cd "$SCRIPT_DIR/../../../../" && pwd)"
# Build context is one level higher so the sibling `ai-ui/` repo
# (path-dep'd by codeless-ai-ui via ../../../../ai-ui) is reachable.
BUILD_CONTEXT="$(cd "$WORKSPACE_ROOT/.." && pwd)"
OUTPUT_DIR="${SCRIPT_DIR}/dist"

mkdir -p "$OUTPUT_DIR"

# BuildKit is required for per-Dockerfile dockerignore files
# (Dockerfile.linux.dockerignore next to the Dockerfile).
export DOCKER_BUILDKIT=1

usage() {
    cat <<EOF
Usage: $(basename "$0") [linux|windows|all]

Builds codeless-tauri-desktop as a standalone binary.

Targets:
  linux    Build Linux x86_64 binary
  windows  Build Windows x86_64 .exe (cross-compiled from Linux)
  all      Build both (default)

Output goes to: $OUTPUT_DIR/
EOF
    exit 0
}

build_linux() {
    echo "==> Building Linux binary..."
    docker build \
        -f "$SCRIPT_DIR/Dockerfile.linux" \
        -t codeless-desktop-linux \
        "$BUILD_CONTEXT"

    # Extract the binary from the image
    container_id=$(docker create codeless-desktop-linux)
    docker cp "$container_id:/usr/local/bin/codeless-tauri-desktop" "$OUTPUT_DIR/codeless-tauri-desktop-linux-x86_64"
    docker rm "$container_id" > /dev/null
    echo "==> Linux binary: $OUTPUT_DIR/codeless-tauri-desktop-linux-x86_64"
}

build_windows() {
    echo "==> Building Windows binary..."
    # Use --output to extract directly from the scratch stage
    docker build \
        -f "$SCRIPT_DIR/Dockerfile.windows" \
        --output "type=local,dest=$OUTPUT_DIR" \
        --target output \
        "$WORKSPACE_ROOT"

    # Rename for clarity
    if [[ -f "$OUTPUT_DIR/codeless-tauri-desktop.exe" ]]; then
        mv "$OUTPUT_DIR/codeless-tauri-desktop.exe" "$OUTPUT_DIR/codeless-tauri-desktop-windows-x86_64.exe"
    fi
    echo "==> Windows binary: $OUTPUT_DIR/codeless-tauri-desktop-windows-x86_64.exe"
}

TARGET="${1:-all}"

case "$TARGET" in
    linux)   build_linux ;;
    windows) build_windows ;;
    all)     build_linux; build_windows ;;
    -h|--help) usage ;;
    *)
        echo "Unknown target: $TARGET"
        usage
        ;;
esac

echo "==> Done. Binaries in $OUTPUT_DIR/"
ls -lh "$OUTPUT_DIR/"
