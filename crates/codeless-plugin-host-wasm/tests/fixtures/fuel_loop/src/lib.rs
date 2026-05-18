//! Fuel-burn fixture. Built against the default `plugin` world
//! (`codeless:tool/tool` only, no host imports), so the
//! capability-sandbox path stays out of the way and the test is
//! pinning fuel exhaustion specifically.
//!
//! The body of `tool.call` is an arithmetic loop guarded by
//! `core::hint::black_box` so LLVM cannot collapse it to
//! `unreachable`. Each iteration retires several WASM
//! instructions; against a 100_000-fuel cap the `OutOfFuel` trap
//! fires inside a few milliseconds -- well under the 200 ms
//! wall-clock deadline the integration test sets.
//!
//! `describe` returns a single manifest entry so the same fixture
//! can be exercised through the manifest list as well; the test
//! only uses it to construct a `ToolCall` though.

#![allow(unsafe_code)]

wit_bindgen::generate!({
    path: "../../../../codeless-tool-wit/wit",
    world: "plugin",
});

use exports::codeless::tool::tool::{
    Guest, Tier as WitTier, ToolCall as WitToolCall, ToolManifest as WitToolManifest,
    ToolResult as WitToolResult,
};

struct Component;

impl Guest for Component {
    fn describe() -> Vec<WitToolManifest> {
        vec![WitToolManifest {
            id: "fuel.loop.spin".into(),
            description: "Spin forever; the host fuel cap is supposed to stop us.".into(),
            input_schema: "{\"type\":\"object\"}".into(),
            output_schema: "{\"type\":\"object\"}".into(),
            tier: WitTier::Read,
        }]
    }

    fn call(_req: WitToolCall) -> WitToolResult {
        // black_box keeps the accumulator live so LLVM cannot fold
        // the loop into `unreachable`; without it the wasm trap
        // would be `UnreachableCodeReached` instead of `OutOfFuel`
        // and the test could not distinguish a fuel exhaustion
        // from any other kind of guest abort.
        let mut x: u64 = 0;
        loop {
            x = core::hint::black_box(x.wrapping_add(1));
        }
    }
}

export!(Component);
