//! Attachment round-trip fixture. Exercises the
//! `codeless:attachments/store` host import end-to-end by minting a
//! known-bytes blob and reading it back, then returning the
//! verification result as a JSON string the host test asserts on.
//!
//! Built against the `plugin-with-attachments` world: the component
//! declares the import; the host's per-plugin linker (gated on the
//! manifest's `[runtimes.capabilities] attachments` set) provides
//! the implementation. With the capability denied this component
//! fails to instantiate -- which is the test's negative case in
//! the host crate's `plugin_wasm_e2e` integration test.

#![allow(unsafe_code)]

wit_bindgen::generate!({
    path: "../../../../codeless-tool-wit/wit",
    world: "plugin-with-attachments",
});

use exports::codeless::tool::tool::{
    Guest, Tier as WitTier, ToolCall as WitToolCall, ToolError as WitToolError,
    ToolManifest as WitToolManifest, ToolResult as WitToolResult,
};

struct Component;

impl Guest for Component {
    fn describe() -> Vec<WitToolManifest> {
        vec![WitToolManifest {
            id: "attach.probe.roundtrip".into(),
            description: "Mint an attachment, read it back, verify bytes match.".into(),
            // The host treats these as opaque strings; the e2e test
            // does not validate args against the schema before
            // dispatch, so empty objects are enough.
            input_schema: "{\"type\":\"object\"}".into(),
            output_schema: "{\"type\":\"object\"}".into(),
            tier: WitTier::Write,
        }]
    }

    fn call(_req: WitToolCall) -> WitToolResult {
        let bytes = b"hello from wasm";
        let id = match codeless::attachments::store::mint("probe.txt", bytes) {
            Ok(id) => id,
            Err(_) => {
                return WitToolResult::Err(WitToolError {
                    code: "mint-failed".into(),
                    message: "host refused mint".into(),
                    retryable: false,
                });
            }
        };
        let read_back = match codeless::attachments::store::read(&id) {
            Ok(b) => b,
            Err(_) => {
                return WitToolResult::Err(WitToolError {
                    code: "read-failed".into(),
                    message: "host refused read".into(),
                    retryable: false,
                });
            }
        };
        if read_back == bytes {
            WitToolResult::Ok(format!("{{\"attachment_id\":\"{id}\",\"verified\":true}}"))
        } else {
            WitToolResult::Err(WitToolError {
                code: "mismatch".into(),
                message: "round-trip bytes differ".into(),
                retryable: false,
            })
        }
    }
}

export!(Component);
