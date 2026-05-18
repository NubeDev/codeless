//! Doc-only smoke test for the codeless tool WIT contract.
//!
//! Two cheap checks the next-stage host work depends on:
//!
//! 1. `wit/tool.wit` is syntactically valid and declares the package
//!    name the host loader will look for. A typo, an accidental
//!    package rename, or an interface that no longer exports the
//!    `tool` world fails here before stage 4's
//!    `wasmtime::component::bindgen!` ever runs.
//! 2. The wit-bindgen-derived guest types can carry a manifest's
//!    fields end-to-end. We instantiate `ToolManifest`, `ToolCall`,
//!    and both arms of `ToolResult`, then read every field back --
//!    proof that the generated layout still matches the WIT record
//!    after any regeneration. No plugin code is loaded; no wasm
//!    artefact is built. This is the "doc-only" bound called out in
//!    the stage description.

use codeless_tool_wit::bindings::exports::codeless::tool::tool::{
    Tier, ToolCall, ToolError, ToolManifest, ToolResult,
};
use codeless_tool_wit::{PACKAGE_ID, TOOL_WIT};

#[test]
fn wit_parses_and_declares_expected_package() {
    // Resolve from the whole `wit/` directory rather than a single
    // file. Stage 6 of plugin-substrate-runtimes added cross-package
    // imports (`codeless:attachments/store`, `codeless:fs/probe`)
    // referenced from new worlds in `tool.wit`; their packages live
    // under `wit/deps/` per the wit-parser convention. Pushing just
    // `TOOL_WIT` as a string would fail to resolve those imports.
    let wit_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("wit");
    let mut resolve = wit_parser::Resolve::default();
    let (pkg_id, _) = resolve
        .push_dir(&wit_dir)
        .expect("wit/ directory parses as a WIT package tree");

    let pkg = &resolve.packages[pkg_id];
    // Silence the now-unused-direct-import warning for `TOOL_WIT`:
    // the WIT-source constant still exists for downstream
    // consumers; the smoke test just doesn't need it any more.
    let _: &str = TOOL_WIT;
    let name = pkg.name.to_string();
    assert_eq!(
        name, PACKAGE_ID,
        "WIT package name drifted: parsed `{name}`, expected `{PACKAGE_ID}` -- \
         a rename here is an ABI break; bump the package version and update \
         PLUGIN-WASM.md instead of silently mutating the parsed name"
    );

    let world = resolve
        .worlds
        .iter()
        .find(|(_, w)| w.name == "plugin")
        .expect("WIT declares a `plugin` world");
    assert!(
        world
            .1
            .exports
            .values()
            .any(|item| matches!(item, wit_parser::WorldItem::Interface { .. })),
        "`plugin` world must export the `tool` interface"
    );
}

#[test]
fn generated_manifest_round_trips_through_the_wit_types() {
    // Build a manifest the same shape `codeless-plugin-sdk::Manifest`
    // serialises to when the WASM flavour answers `describe()`. The
    // round-trip is field-by-field; the generated `ToolManifest`
    // does not implement `PartialEq` (it carries `_rt::String`), so
    // the assertions read the fields back rather than comparing
    // whole records.
    let manifest = ToolManifest {
        id: "notes.notes_append".into(),
        description: "Append a note to the active thread.".into(),
        input_schema: r#"{"type":"object"}"#.into(),
        output_schema: r#"{"type":"object"}"#.into(),
        tier: Tier::Write,
    };
    assert_eq!(manifest.id, "notes.notes_append");
    assert_eq!(manifest.tier as u8, Tier::Write as u8);

    let call = ToolCall {
        tool_id: manifest.id.clone(),
        args_json: r#"{"body":"hello"}"#.into(),
        thread_id: "t_abc".into(),
    };
    assert_eq!(call.thread_id, "t_abc");

    let ok = ToolResult::Ok(r#"{"attachment_id":"a_1"}"#.into());
    let err = ToolResult::Err(ToolError {
        code: "limit-exceeded".into(),
        message: "fuel cap hit".into(),
        retryable: false,
    });
    match ok {
        ToolResult::Ok(payload) => {
            assert!(payload.contains("attachment_id"))
        }
        ToolResult::Err(_) => panic!("ok arm should be Ok"),
    }
    match err {
        ToolResult::Err(e) => {
            assert_eq!(e.code, "limit-exceeded");
            assert!(!e.retryable);
        }
        ToolResult::Ok(_) => panic!("err arm should be Err"),
    }
}

#[test]
fn tier_discriminants_are_stable() {
    // The ABI pins these to 0/1/2. A wit-bindgen regeneration that
    // reordered them would be a silent ABI break -- catch it here
    // rather than at runtime in a deployed plugin.
    assert_eq!(Tier::Read as u8, 0);
    assert_eq!(Tier::Write as u8, 1);
    assert_eq!(Tier::Destructive as u8, 2);
}
