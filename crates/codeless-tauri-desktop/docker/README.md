# Docker Cross-Compilation for codeless-tauri-desktop

Builds standalone binaries (not AppImages, not installers) for Linux and Windows.

## Prerequisites

- Docker with BuildKit enabled (`DOCKER_BUILDKIT=1`)
- ~10 GB disk space for build images

## Quick Start

```sh
cd codeless/crates/codeless-tauri-desktop/docker
./build.sh all        # both targets
./build.sh linux      # Linux x86_64 binary only
./build.sh windows    # Windows x86_64 .exe only
```

Binaries land in `docker/dist/`:

```
dist/
  codeless-tauri-desktop-linux-x86_64
  codeless-tauri-desktop-windows-x86_64.exe
```

## How It Works

### Linux

Native compilation inside `rust:1.78-bookworm` with all GTK/WebKit
system libraries installed. The output binary dynamically links to
system libs (webkit2gtk-4.1, libsoup-3.0, GTK3, dbus). The host
running the binary needs these libraries installed.

### Windows

Cross-compilation from Linux using
[cargo-xwin](https://github.com/rust-cross/cargo-xwin) targeting
`x86_64-pc-windows-msvc`. cargo-xwin automatically downloads the
Windows SDK and MSVC CRT headers. The resulting `.exe` uses WebView2
(pre-installed on Windows 10 21H2+ and Windows 11).

## Build Context

The Docker build context is the workspace root
(`codeless-workspace/`) because the Cargo workspace references
`../ai-runner` as a path dependency. The `.dockerignore` at the
workspace root excludes `target/`, `node_modules/`, and `.git/` to
keep the context transfer fast.

## Notes

- The UI is built inside the container (`pnpm install && pnpm build`)
  before the Rust compilation.
- First build downloads ~2 GB of crates + Windows SDK. Subsequent
  builds use Docker layer caching.
- The Windows binary requires WebView2 runtime on the target machine
  (ships with Windows 10/11 by default).
