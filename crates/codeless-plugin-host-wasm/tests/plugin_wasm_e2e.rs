//! Stage 6 of plugin-substrate-runtimes: capability-sandbox e2e.
//!
//! Two scenarios, each backed by a small Rust-source wasm fixture
//! under `tests/fixtures/`:
//!
//! - `wasm_plugin_cannot_open_host_file` -- the `fs_probe` fixture
//!   declares the `plugin-with-fs` world (imports
//!   `codeless:fs/probe`). Loaded with `Capabilities::default()`
//!   (default-deny: `fs = []`), the host's per-plugin linker omits
//!   the interface entirely; component instantiation fails at
//!   `WasmPlugin::load`. That load-time failure is the assertion --
//!   it is what `PLUGIN-WASM.md § Capability sandbox` means by
//!   "wasi:filesystem is not linked": the sandbox enforces at the
//!   linker, not at the call boundary.
//!
//! - `wasm_plugin_attachment_round_trip` -- the `attachment_probe`
//!   fixture declares the `plugin-with-attachments` world (imports
//!   `codeless:attachments/store`). Loaded with an
//!   `InMemoryAttachmentStore` and the `attachments = ["read",
//!   "write"]` capability, the fixture mints, reads back, and
//!   returns the verification result in `tool-result.ok`. The test
//!   asserts the result string contains `"verified":true`.
//!
//! Fixtures live outside the codeless workspace
//! (`tests/fixtures/*/Cargo.toml` carry their own `[workspace]`)
//! so a host `cargo build --workspace` does not try to compile
//! `wasm32-unknown-unknown` targets. The test builds them via an
//! explicit `cargo build --manifest-path ...` on first run and
//! caches the resulting component in `target-wasm/`.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use codeless_plugin_host_wasm::{
    AdapterRequest, AttachmentStore, Capabilities, HostPolicy, InMemoryAttachmentStore,
    LoadOptions, WasmPlugin, WasmRuntime,
};
use codeless_tools::runtime_adapter::ToolCallOutcome;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root")
        .to_path_buf()
}

fn target_wasm_dir() -> PathBuf {
    workspace_root().join("target-wasm")
}

/// Build a fixture crate at `tests/fixtures/<name>/` into a wasm
/// component and return the path. Cached on the resulting
/// component file's mtime to keep test re-runs fast.
fn build_fixture(name: &str) -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let fixture_dir = manifest_dir.join("tests").join("fixtures").join(name);
    let fixture_target = target_wasm_dir().join("fixtures").join(name);
    // The fixture crate's package name is set in its Cargo.toml.
    // `cargo build` writes the .wasm under
    // `<target>/wasm32-unknown-unknown/release/<package_name>.wasm`,
    // with hyphens turned into underscores.
    let package_name = format!(
        "codeless-plugin-host-wasm-fixture-{}",
        name.replace('_', "-")
    );
    let crate_name = package_name.replace('-', "_");
    let core_module = fixture_target
        .join("wasm32-unknown-unknown")
        .join("release")
        .join(format!("{crate_name}.wasm"));
    let component_path = target_wasm_dir().join(format!("{name}.component.wasm"));

    if !core_module.exists() {
        let status = std::process::Command::new(env!("CARGO"))
            .arg("build")
            .arg("--release")
            .arg("--target")
            .arg("wasm32-unknown-unknown")
            .arg("--manifest-path")
            .arg(fixture_dir.join("Cargo.toml"))
            .env("CARGO_TARGET_DIR", &fixture_target)
            .env(
                "RUSTFLAGS",
                "-C target-cpu=mvp \
                 -C target-feature=-bulk-memory,-reference-types,-multivalue,\
                 -sign-ext,-nontrapping-fptoint,-mutable-globals",
            )
            .status()
            .expect("invoke cargo build for fixture");
        assert!(
            status.success(),
            "cargo build of fixture `{name}` failed: {status:?}"
        );
    }
    if needs_rebuild(&core_module, &component_path) {
        let core_bytes = std::fs::read(&core_module).expect("read fixture core module");
        let component_bytes = wit_component::ComponentEncoder::default()
            .validate(true)
            .module(&core_bytes)
            .expect("attach fixture module to component encoder")
            .encode()
            .expect("encode fixture component");
        std::fs::create_dir_all(component_path.parent().unwrap()).ok();
        std::fs::write(&component_path, &component_bytes).expect("write fixture component");
    }
    component_path
}

fn needs_rebuild(src: &std::path::Path, dst: &std::path::Path) -> bool {
    let Ok(dst_meta) = std::fs::metadata(dst) else {
        return true;
    };
    let Ok(src_meta) = std::fs::metadata(src) else {
        return true;
    };
    match (src_meta.modified(), dst_meta.modified()) {
        (Ok(s), Ok(d)) => s > d,
        _ => true,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wasm_plugin_cannot_open_host_file() {
    // The fixture imports `codeless:fs/probe`. The host's per-plugin
    // linker is built from `Capabilities::default()` -- `fs = []`,
    // so the interface is *not* added to the linker. Component
    // instantiation must therefore fail at `WasmPlugin::load` with
    // an `InvalidComponent` carrying a "missing import" message;
    // that is the load-bearing assertion exercising
    // `PLUGIN-WASM.md § Capability sandbox`.
    let component = build_fixture("fs_probe");
    let runtime = Arc::new(WasmRuntime::new().expect("engine builds"));
    let err = match WasmPlugin::load(
        runtime,
        &component,
        HostPolicy::defaults(),
        LoadOptions::default(),
    )
    .await
    {
        Ok(_) => panic!("load must fail when fs probe is not linked"),
        Err(e) => e,
    };
    // The host loader wraps wasmtime's instantiate-time error in
    // `HostError::InvalidComponent`; the wasmtime message names the
    // missing import so we can assert against the interface name
    // without coupling to the exact wording.
    let msg = format!("{err}");
    assert!(
        msg.contains("codeless:fs/probe") || msg.contains("import"),
        "expected load failure to mention the missing fs probe import, got: {msg}",
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wasm_plugin_attachment_round_trip() {
    // The fixture imports `codeless:attachments/store`. The host
    // wires an `InMemoryAttachmentStore` and the
    // `Capabilities { attachments_read, attachments_write, ... }`
    // set that authorises both. The fixture's `tool.call` mints a
    // known-bytes blob, reads it back, and returns
    // `{ "verified": true, "attachment_id": ... }` on success.
    let component = build_fixture("attachment_probe");
    let runtime = Arc::new(WasmRuntime::new().expect("engine builds"));
    let store: Arc<dyn AttachmentStore> = Arc::new(InMemoryAttachmentStore::new());
    let caps = Capabilities {
        attachments_read: true,
        attachments_write: true,
        ..Default::default()
    };
    let plugin = WasmPlugin::load(
        runtime,
        &component,
        HostPolicy::defaults(),
        LoadOptions {
            capabilities: caps,
            attachments: Some(Arc::clone(&store)),
        },
    )
    .await
    .expect("load fixture with attachments capability granted");
    let manifests = plugin.manifests();
    assert_eq!(manifests.len(), 1);
    assert_eq!(manifests[0].id, "attach.probe.roundtrip");

    let outcome = plugin
        .call(codeless_plugin_host_wasm::AdapterRequest {
            tool_id: "attach.probe.roundtrip",
            args_json: "{}",
            thread_id: "thread-rt-1",
        })
        .await;
    match outcome {
        ToolCallOutcome::Ok(s) => {
            assert!(
                s.contains("\"verified\":true"),
                "round-trip succeeded but body unexpected: {s}",
            );
            assert!(
                s.contains("attachment_id"),
                "round-trip should surface the minted id: {s}",
            );
        }
        ToolCallOutcome::Err(e) => panic!("round-trip failed: {e:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wasm_plugin_respects_fuel_cap() {
    // The `fuel_loop` fixture spins in a `black_box`-guarded
    // arithmetic loop. Under a 100_000-fuel cap the wasmtime
    // `OutOfFuel` trap fires after a handful of milliseconds; the
    // 200 ms wall-clock deadline below is the backstop that
    // `PLUGIN-WASM.md § Limits` requires regardless of fuel state.
    //
    // The assertion is twofold: (1) the call surfaces a
    // `ToolCallOutcome::Err` with code `"limit-exceeded"` and the
    // message names `"fuel"` -- i.e. fuel exhaustion is the
    // reported reason, not the deadline -- and (2) the entire test
    // future finishes inside the 200 ms deadline, observed by an
    // outer `tokio::time::timeout`. Together those pin both halves
    // of OQ-WASM-5: the cap actually fires, and it fires quickly
    // enough to keep the agent loop responsive.
    let component = build_fixture("fuel_loop");
    let runtime = Arc::new(WasmRuntime::new().expect("engine builds"));
    let policy = HostPolicy {
        fuel: 100_000,
        memory_max_bytes: 64 * 1024 * 1024,
        deadline: Duration::from_millis(200),
    };
    let plugin = WasmPlugin::load(runtime, &component, policy, LoadOptions::default())
        .await
        .expect("load fuel-loop fixture under low-fuel policy");

    // Outer guard is intentionally slack -- the 200 ms assertion
    // below is the load-bearing budget, this one only exists so a
    // bug that disables both fuel *and* deadline cannot deadlock
    // the test runner.
    let started = std::time::Instant::now();
    let outcome = tokio::time::timeout(
        Duration::from_secs(5),
        plugin.call(AdapterRequest {
            tool_id: "fuel.loop.spin",
            args_json: "{}",
            thread_id: "thread-fuel-1",
        }),
    )
    .await
    .expect("call must return; both fuel and deadline are off");
    let elapsed = started.elapsed();

    match outcome {
        ToolCallOutcome::Err(e) => {
            assert_eq!(e.code, "limit-exceeded", "wrong error code: {e:?}");
            assert!(
                e.message.contains("fuel"),
                "expected fuel-reason message, got: {e:?}",
            );
        }
        ToolCallOutcome::Ok(s) => panic!("infinite loop must not return Ok: {s}"),
    }
    assert!(
        elapsed < Duration::from_millis(200),
        "fuel trap took {elapsed:?}, exceeds the 200 ms budget",
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wasm_plugin_with_attachments_denied_when_capability_missing() {
    // Defence-in-depth pin: the same fixture, the same store, but
    // no `attachments` capability. Without it the per-plugin linker
    // does not add `codeless:attachments/store`, instantiation
    // fails. The first test owns the high-level "default-deny"
    // signal; this one pins that the gate is the capability set
    // specifically, not some other accident of the load path.
    let component = build_fixture("attachment_probe");
    let runtime = Arc::new(WasmRuntime::new().expect("engine builds"));
    let store: Arc<dyn AttachmentStore> = Arc::new(InMemoryAttachmentStore::new());
    let err = match WasmPlugin::load(
        runtime,
        &component,
        HostPolicy::defaults(),
        LoadOptions {
            capabilities: Capabilities::default(),
            attachments: Some(Arc::clone(&store)),
        },
    )
    .await
    {
        Ok(_) => panic!("load must fail without attachments capability"),
        Err(e) => e,
    };
    let msg = format!("{err}");
    assert!(
        msg.contains("codeless:attachments/store") || msg.contains("import"),
        "expected load failure to mention attachments import, got: {msg}",
    );
}
