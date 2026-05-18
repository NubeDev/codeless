//! End-to-end smoke test for the stage-2 authoring surface.
//!
//! Pins three properties together so a regression in any one trips
//! exactly one test:
//!
//! 1. `#[derive(Tool)]` reads `id` / `tier` / `description` off the
//!    `#[tool(...)]` attribute and surfaces them as `ToolMeta`
//!    constants.
//! 2. `Manifest::for_behavior::<T>()` returns a manifest whose ids
//!    and schemas agree with the trait impls -- in particular, the
//!    `schemars`-derived input/output schemas reach the manifest
//!    intact.
//! 3. `register!(T)` compiles for a valid `ToolBehavior` impl. The
//!    macro is a stub today (per `register.rs`), but the
//!    type-checking expansion must succeed so plugin source written
//!    against this SDK survives the later builtin/wasm wiring with
//!    no diff.

use async_trait::async_trait;
use codeless_plugin_sdk::{
    register, Manifest, Tier, Tool, ToolBehavior, ToolCtx, ToolError, ToolMeta,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Tool)]
#[tool(
    id = "smoke.append",
    tier = "write",
    description = "Smoke-test tool used by the SDK's own tests."
)]
struct SmokeAppend;

#[derive(Deserialize, JsonSchema)]
struct SmokeArgs {
    #[allow(dead_code)]
    body: String,
}

#[derive(Serialize, JsonSchema)]
struct SmokeOutput {
    #[allow(dead_code)]
    bytes: u32,
}

#[async_trait]
impl ToolBehavior for SmokeAppend {
    type Args = SmokeArgs;
    type Output = SmokeOutput;

    async fn call(&self, _ctx: &ToolCtx, args: Self::Args) -> Result<Self::Output, ToolError> {
        Ok(SmokeOutput {
            bytes: args.body.len() as u32,
        })
    }
}

register!(SmokeAppend);

#[test]
fn derive_emits_tool_meta_constants() {
    assert_eq!(<SmokeAppend as ToolMeta>::ID, "smoke.append");
    assert_eq!(<SmokeAppend as ToolMeta>::TIER, Tier::Write);
    assert_eq!(
        <SmokeAppend as ToolMeta>::DESCRIPTION,
        "Smoke-test tool used by the SDK's own tests."
    );
}

#[test]
fn manifest_for_behavior_round_trips_schemas() {
    let m: Manifest = Manifest::for_behavior::<SmokeAppend>();
    assert_eq!(m.id, "smoke.append");
    assert_eq!(m.tier, Tier::Write);

    // Schemars emits an object schema at the root for a struct;
    // poking the `properties.body` path proves schemars actually ran
    // and the manifest carried its output through unchanged.
    let body = m
        .input_schema
        .pointer("/properties/body/type")
        .and_then(|v| v.as_str());
    assert_eq!(body, Some("string"));

    let bytes = m
        .output_schema
        .pointer("/properties/bytes/type")
        .and_then(|v| v.as_str());
    // schemars represents `u32` as `"integer"`.
    assert_eq!(bytes, Some("integer"));
}

#[tokio::test]
async fn call_dispatches_through_behavior() {
    let tool = SmokeAppend;
    let ctx = ToolCtx::__from_host_seal();
    let out = tool
        .call(
            &ctx,
            SmokeArgs {
                body: "hello".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(out.bytes, 5);
}

#[test]
fn tier_round_trips_canonical_strings() {
    assert_eq!(Tier::Read.as_str(), "read");
    assert_eq!(Tier::Write.as_str(), "write");
    assert_eq!(Tier::Destructive.as_str(), "destructive");
}
