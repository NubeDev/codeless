//! Host-file-read fixture. The component declares
//! `world plugin-with-fs` -- exports `codeless:tool/tool`, imports
//! `codeless:fs/probe`. The integration test loads this under
//! `Capabilities::default()` (fs = []), the host omits
//! `codeless:fs/probe` from the linker, and the resulting
//! instantiation failure is the test's positive signal.
//!
//! `tool.call` is never reached under the default-deny load -- but
//! it still has to compile because the WIT world requires
//! `codeless:tool/tool` to be exported. The body just asks the host
//! for `/etc/passwd`; under a hypothetical broader future load
//! (with `fs = ["/etc/"]`) the test would observe the host's path
//! check rather than a load-time failure.

#![allow(unsafe_code)]

wit_bindgen::generate!({
    path: "../../../../codeless-tool-wit/wit",
    world: "plugin-with-fs",
});

use exports::codeless::tool::tool::{
    Guest, Tier as WitTier, ToolCall as WitToolCall, ToolError as WitToolError,
    ToolManifest as WitToolManifest, ToolResult as WitToolResult,
};

struct Component;

impl Guest for Component {
    fn describe() -> Vec<WitToolManifest> {
        vec![WitToolManifest {
            id: "fs.probe.open_host_file".into(),
            description: "Ask the host to read /etc/passwd.".into(),
            input_schema: "{\"type\":\"object\"}".into(),
            output_schema: "{\"type\":\"object\"}".into(),
            tier: WitTier::Read,
        }]
    }

    fn call(_req: WitToolCall) -> WitToolResult {
        match codeless::fs::probe::read_file("/etc/passwd") {
            Ok(_) => WitToolResult::Ok("{\"opened\":true}".into()),
            Err(_) => WitToolResult::Err(WitToolError {
                code: "denied".into(),
                message: "host refused /etc/passwd".into(),
                retryable: false,
            }),
        }
    }
}

export!(Component);
