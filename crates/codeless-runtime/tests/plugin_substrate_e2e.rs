//! Plugin-substrate end-to-end coverage (PS-ACCEPT, DOCS/PLUGIN-SUBSTRATE.md
//! Acceptance §3). Drives the on-disk `plugins/notes/` plugin through the
//! same seams the host binary uses at boot:
//!
//! - PS6 (plugin manifest + registry): `PluginRegistry::load_plugin` against
//!   the workspace `plugins/notes/` directory using the `notes` crate's real
//!   `register` entry point — the same path `codeless-cli` walks.
//! - PS5 (persona / thread-kind data model): the loaded persona is
//!   upserted into the live `personas` table and bound to a new Assistant
//!   thread via `create_assistant_thread`; the resolver returns the
//!   plugin's `allowed_tools` + system prompt.
//! - PS3 (server-side capability derivation): the substrate-doc matcher
//!   in `codeless_types::allowed_tools::tool_allowed` accepts
//!   `notes.append` (and `attachments.read`) for the plugin persona, and
//!   rejects out-of-namespace ids — proving the persona's column drives
//!   what a thread may invoke, not a UI routing prop.
//! - PS7 (tool-result attachments): the plugin tool's output schema
//!   declares the `codeless://attachment` marker; uploading an
//!   attachment and asking `find_attachment_refs` +
//!   `reconcile_attachment_refs` to walk a synthesised tool output
//!   yields an `AssistantAttachmentCard` whose stored-row fields win
//!   over the tool's hints.
//! - PS8 (Assistant agent loop): the planner-side `tool_allowed` filter
//!   (in `assistant_planner::run_planner_turn`) accepts a plugin tool
//!   when the persona grants `notes.*` — the same matcher both the
//!   prompt-trailer builder and the publish-closure use.
//!
//! Items PS2 (`CommonChat` extraction) and PS4 (chat state moves
//! server-side) are halted with the analysis recorded in
//! DOCS/sessions/2026-05-17-plugin-substrate.md; the UI-side prep that
//! did land (PS2a `threadId` plumbing) has its own vitest under
//! `ui/codeless-ui/src/modules/chat/CommonChat.test.tsx`. Reflected in
//! the substrate doc's Acceptance section.
//!
//! Living in `tests/` (not in `src/`) so the harness exercises the
//! plugin layer the way an out-of-tree author would: through the public
//! API of `codeless-tools` + `codeless-runtime`, not via crate-private
//! shortcuts.

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use codeless_plugin_host_wasm::{HostPolicy, WasmPlugin, WasmRuntime};
use codeless_rpc::{
    AppendAssistantMessageArgs, CreateAssistantThreadArgs, RpcServer, UploadAssistantAttachmentArgs,
};
use codeless_runtime::InProcessRpc;
use codeless_tools::attachment::{find_attachment_refs, reconcile_attachment_refs};
use codeless_tools::plugin::{PluginRegistry, RegistrationTable};
use codeless_types::allowed_tools::tool_allowed;
use codeless_types::{AssistantAttachmentCard, AssistantMessageRole, Persona, UnixMillis};
use serde_json::json;
use tempfile::TempDir;

/// Path to the workspace-rooted `plugins/notes/` directory. Walking up
/// two levels from `CARGO_MANIFEST_DIR` (`crates/codeless-runtime`) is
/// what `codeless-plugin-notes`'s smoke test does and is the contract
/// the host CLI's `--plugins-dir` flag also lands on by default.
fn notes_plugin_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root")
        .join("plugins")
        .join("notes")
}

/// Load the on-disk notes plugin through `PluginRegistry` and apply
/// its migrations to the runtime's pool. Returns the registry plus the
/// loaded snapshot so the test can assert against the same data
/// `codeless plugin info` would print.
async fn load_notes_into(
    rpc: &InProcessRpc,
) -> (PluginRegistry, codeless_tools::plugin::LoadedPlugin) {
    let mut table = RegistrationTable::new();
    table.insert(
        codeless_plugin_notes::PLUGIN_ID,
        codeless_plugin_notes::register,
    );
    let mut registry = PluginRegistry::new();
    let loaded = registry
        .load_plugin(&notes_plugin_dir(), &table)
        .expect("load notes plugin")
        .clone();

    // The substrate doc places the SQL apply on the runtime side (see
    // `crates/codeless-tools/src/plugin/registry.rs` head comment); the
    // codeless-tools crate stops at vetting. Drive the apply here so
    // the test exercises the documented contract end-to-end -- the
    // future host-boot path will do the same against `rpc.pool()`.
    let pool = rpc.pool();
    for migration in &loaded.migrations {
        sqlx::query(&migration.sql)
            .execute(pool)
            .await
            .expect("apply plugin migration");
    }

    (registry, loaded)
}

/// Upsert the plugin's `LoadedPersona` into the live `personas` table
/// so `create_assistant_thread` accepts it as a FK target. The
/// substrate-doc shape (item 5) is one column-per-field; the rest of
/// the Persona surface (legacy `use_for_jobs`, `allowed_subagents`,
/// `default_snippets`) stays at safe defaults because plugin personas
/// are MCP-tool-shaped, not job-runner-shaped.
async fn upsert_loaded_persona(
    rpc: &InProcessRpc,
    loaded: &codeless_tools::plugin::LoadedPersona,
) -> Persona {
    let now = UnixMillis(0);
    let persona = Persona {
        id: loaded.id.clone(),
        name: loaded.name.clone(),
        description: loaded.description.clone(),
        icon: loaded.icon.clone(),
        instructions: loaded.system_prompt.clone(),
        use_for_jobs: false,
        default_model: None,
        allowed_subagents: Vec::new(),
        default_snippets: Vec::new(),
        allowed_tools: loaded.allowed_tools.clone(),
        default_model_family: Some(loaded.default_model_family.clone()),
        default_attachments_policy: loaded.default_attachments_policy.clone(),
        built_in: false,
        created_at: now,
        updated_at: now,
    };
    rpc.store()
        .upsert_persona(&persona)
        .await
        .expect("upsert plugin persona")
}

/// One row in the parameterised flavour matrix for
/// `notes_plugin_loads_and_seeds_persona_addressable_by_thread`. The
/// substrate-doc claim under plugin-substrate-runtimes stage 5 is:
/// the **same** `codeless-plugin-notes` source compiles into either a
/// builtin Rust shim *or* a WASI-p2 component, and the resulting
/// `notes.append` manifest the host sees is byte-identical at the
/// `AdapterToolManifest` boundary. The two test functions below are
/// thin wrappers that drive the same body against each flavour; if
/// they diverge for any reason other than the build path, the
/// substrate's "one source, two flavours" promise has slipped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NotesFlavour {
    Builtin,
    Wasm,
}

/// Body shared by both flavour rows of
/// `notes_plugin_loads_and_seeds_persona_addressable_by_thread`. The
/// builtin row dispatches `notes.append` through the host
/// `ToolRegistry`; the wasm row dispatches the same id through a
/// `WasmPlugin::load`ed component. Persona-seed and thread-create are
/// shared because both come from the on-disk `plugin.toml` -- the
/// runtime flavour does not change the manifest.
async fn run_notes_loads_and_seeds_persona_addressable_by_thread(flavour: NotesFlavour) {
    let rpc = InProcessRpc::new().await.unwrap();
    let (_registry, loaded) = load_notes_into(&rpc).await;
    let plugin_persona = &loaded.personas[0];
    assert_eq!(plugin_persona.id, "notes:notes");
    upsert_loaded_persona(&rpc, plugin_persona).await;

    let thread = rpc
        .create_assistant_thread(CreateAssistantThreadArgs {
            title: Some("notes".into()),
            persona_id: plugin_persona.id.clone(),
        })
        .await
        .expect("thread bound to plugin persona");
    assert_eq!(thread.persona_id, "notes:notes");

    // The flavour-specific seam: builtin reads the tool manifest off
    // the `ToolRegistry` (the host's `Arc<dyn Tool>`), wasm reads it
    // off a `WasmPlugin::load`ed component's `describe()` export. Both
    // must surface the same id + tier and an output schema whose PS7
    // attachment marker survives the manifest round-trip.
    match flavour {
        NotesFlavour::Builtin => {
            let tool = _registry
                .tool_registry()
                .get("notes.append")
                .expect("builtin shim registers notes.append");
            assert_eq!(tool.name(), "notes.append");
            let out = tool.output_schema();
            assert_eq!(
                out.pointer("/properties/attachment/$ref")
                    .and_then(|v| v.as_str()),
                Some("codeless://attachment"),
                "builtin output schema declares the PS7 marker",
            );
        }
        NotesFlavour::Wasm => {
            let plugin = wasm_notes_plugin().await;
            let manifests = plugin.manifests();
            assert_eq!(manifests.len(), 1, "notes plugin contributes a single tool",);
            let m = &manifests[0];
            assert_eq!(m.id, "notes.append");
            assert_eq!(m.tier, "write");
            let out: serde_json::Value =
                serde_json::from_str(&m.output_schema).expect("output schema is JSON");
            assert_eq!(
                out.pointer("/properties/attachment/$ref")
                    .and_then(|v| v.as_str()),
                Some("codeless://attachment"),
                "wasm describe() output schema declares the PS7 marker",
            );
        }
    }

    // Append a free-form user message; without `with_agent_chat` the
    // planner branch routes to the NOOP fallback, so the round-trip
    // exercises the persona-bound thread without depending on a model
    // dispatch. The substrate-doc test of "the runner loads tools and
    // system prompt from that persona at agent-call time" is exercised
    // by `planner_keeps_tool_calls_inside_persona_allow_list` in the
    // planner module's unit tests; here we pin the create + append
    // round-trip on a plugin-seeded persona id.
    let appended = rpc
        .append_assistant_message(AppendAssistantMessageArgs {
            thread_id: thread.id,
            content: "remember to buy milk".into(),
        })
        .await
        .expect("append on plugin-persona thread");
    assert!(matches!(
        appended.user_message.role,
        AssistantMessageRole::User
    ));
    assert!(matches!(
        appended.assistant_message.role,
        AssistantMessageRole::Assistant
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn notes_plugin_loads_and_seeds_persona_addressable_by_thread_builtin() {
    // Substrate items 5 + 6 jointly: a plugin's manifest produces a
    // persona row the Assistant surface can bind a thread to. A
    // regression in either the namespacing rule (`<plugin>:<slug>`) or
    // the `create_assistant_thread` FK validation fails here, before
    // anything more elaborate runs.
    run_notes_loads_and_seeds_persona_addressable_by_thread(NotesFlavour::Builtin).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn notes_plugin_loads_and_seeds_persona_addressable_by_thread_wasm() {
    // Plugin-substrate-runtimes stage 5: the same `codeless-plugin-
    // notes` source compiles to a WASI-p2 component, whose
    // `describe()` export surfaces the same `notes.append` manifest
    // the builtin shim does. Loading the component through
    // `WasmPlugin::load` is what `codeless-plugin-host-wasm`'s
    // adapter table will do once stage 13 wires `[[runtimes]] kind =
    // "wasm"` into the manifest parser; doing it directly here keeps
    // the parameterisation independent of that later piece.
    run_notes_loads_and_seeds_persona_addressable_by_thread(NotesFlavour::Wasm).await;
}

/// Build (if needed) and load the `notes.wasm` artefact through the
/// host wasm runtime. Cached behind a `OnceLock` so the parameterised
/// flavour runs once across the test binary even when the `Wasm` row
/// is exercised by several tests.
///
/// Building inside the test rather than committing the `.wasm`
/// fixture keeps the artefact in lockstep with the same `src/lib.rs`
/// the builtin shim compiles against -- a divergence between the
/// flavours becomes a build failure on this row, not a stale-fixture
/// pass.
async fn wasm_notes_plugin() -> Arc<WasmPlugin> {
    static CELL: OnceLock<Arc<WasmPlugin>> = OnceLock::new();
    if let Some(p) = CELL.get() {
        return Arc::clone(p);
    }
    let path = ensure_notes_wasm_built();
    let runtime = Arc::new(WasmRuntime::new().expect("wasm runtime"));
    let plugin = WasmPlugin::load(
        runtime,
        &path,
        HostPolicy::defaults(),
        codeless_plugin_host_wasm::LoadOptions::default(),
    )
    .await
    .expect("load notes.wasm component");
    let arc = Arc::new(plugin);
    // Race-tolerant init: if a concurrent test populated the cell
    // first, fall through to the populated value and let the local
    // build's `Arc` drop on return.
    let _ = CELL.set(Arc::clone(&arc));
    Arc::clone(CELL.get().expect("set or already populated"))
}

/// Build (if needed) the notes plugin as a `wasm32-unknown-unknown`
/// core module, then encode it as a WASI-p2 component via
/// `wit-component`. Returns the path to the encoded component.
///
/// Why not `wasm32-wasip2` directly: rustc's bundled wasi-preview1-
/// to-preview2 adapter emits a component-model encoding newer than
/// `wasmtime` 23's parser accepts. `wit-component` pinned to the
/// matching minor produces a component the host can load -- and the
/// notes plugin's `world plugin { export tool; }` has no WASI
/// imports anyway, so the adapter would be load for an empty cargo.
fn ensure_notes_wasm_built() -> PathBuf {
    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root")
        .to_path_buf();
    if let Ok(p) = std::env::var("CODELESS_NOTES_WASM") {
        let pb = PathBuf::from(p);
        assert!(pb.exists(), "CODELESS_NOTES_WASM points at missing file");
        return pb;
    }
    let target_dir = workspace_root.join("target-wasm");
    let core_module = target_dir
        .join("wasm32-unknown-unknown")
        .join("release")
        .join("codeless_plugin_notes.wasm");
    let component_path = target_dir.join("codeless_plugin_notes.component.wasm");
    if !core_module.exists() {
        // Disable newer wasm features rustc enables by default but
        // `wasmtime` 23 (wasmparser 0.212) does not yet parse. The
        // notes plugin's `world plugin { export tool; }` has no need
        // for bulk-memory / reference-types either way; stripping
        // them produces an MVP-compatible core module that the host
        // loads cleanly. Bumping wasmtime is the proper fix, gated by
        // an OQ-WASM-* review.
        let status = std::process::Command::new(env!("CARGO"))
            .arg("build")
            .arg("--release")
            .arg("--target")
            .arg("wasm32-unknown-unknown")
            .arg("-p")
            .arg("codeless-plugin-notes")
            .arg("--no-default-features")
            .arg("--features")
            .arg("wasm")
            .env("CARGO_TARGET_DIR", &target_dir)
            .env(
                "RUSTFLAGS",
                "-C target-cpu=mvp \
                 -C target-feature=-bulk-memory,-reference-types,-multivalue,\
                 -sign-ext,-nontrapping-fptoint,-mutable-globals",
            )
            .current_dir(&workspace_root)
            .status()
            .expect("invoke cargo build for wasm32-unknown-unknown");
        assert!(
            status.success(),
            "cargo build of codeless-plugin-notes (wasm32-unknown-unknown) failed: {status:?}",
        );
    }
    if needs_rebuild(&core_module, &component_path) {
        let core_bytes = std::fs::read(&core_module).expect("read core module");
        let component_bytes = wit_component::ComponentEncoder::default()
            .validate(true)
            .module(&core_bytes)
            .expect("attach core module to component encoder")
            .encode()
            .expect("encode component");
        std::fs::write(&component_path, &component_bytes).expect("write component artefact");
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
async fn persona_allowed_tools_admit_plugin_namespace_only() {
    // Substrate item 3 (server-side capability derivation) + item 5
    // (persona's allowed_tools column). The plugin's persona grants
    // `notes.*` and `attachments.read`; the matcher must accept
    // `notes.append` (prefix match) and `attachments.read` (literal)
    // and reject anything else -- specifically the built-in
    // `assistant.*` namespace the planner exposes to default personas.
    // A regression that widened the matcher (or that accidentally
    // grandfathered built-ins through) trips here.
    let rpc = InProcessRpc::new().await.unwrap();
    let (_registry, loaded) = load_notes_into(&rpc).await;
    let persona = &loaded.personas[0];

    assert!(tool_allowed(&persona.allowed_tools, "notes.append"));
    assert!(tool_allowed(&persona.allowed_tools, "notes.list"));
    assert!(tool_allowed(&persona.allowed_tools, "attachments.read"));
    assert!(!tool_allowed(&persona.allowed_tools, "attachments.write"));
    assert!(!tool_allowed(&persona.allowed_tools, "assistant.list_jobs",));
    assert!(!tool_allowed(&persona.allowed_tools, "fs.read"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn plugin_tool_output_schema_round_trips_through_attachment_reconciler() {
    // Substrate item 7 (tool-result attachments) plus the marker
    // contract: a plugin tool's declared output schema, fed to
    // `find_attachment_refs` together with a synthesised tool output,
    // produces an `AssistantAttachmentCard` whose items reflect the
    // stored row (filename, mime, size) rather than the tool's hints.
    // Driving this through an actual upload + the public reconciler
    // (rather than the runtime's `rpc::attachment` wrapper) means the
    // test fails for the same reason the live agent loop would: a
    // wrong marker in the plugin schema, a cross-thread id, or a
    // dangling id.
    let dir = TempDir::new().unwrap();
    let rpc = InProcessRpc::new()
        .await
        .unwrap()
        .with_assistant_data_dir(dir.path().to_path_buf());
    let (registry, loaded) = load_notes_into(&rpc).await;
    upsert_loaded_persona(&rpc, &loaded.personas[0]).await;
    let thread = rpc
        .create_assistant_thread(CreateAssistantThreadArgs {
            title: None,
            persona_id: loaded.personas[0].id.clone(),
        })
        .await
        .unwrap();

    let upload = rpc
        .upload_assistant_attachment(UploadAssistantAttachmentArgs {
            thread_id: thread.id,
            filename: "note.md".into(),
            mime_type: Some("text/markdown".into()),
            // STANDARD base64 of `# hi\n`.
            content_b64: "IyBoaQo=".into(),
        })
        .await
        .expect("upload attachment");

    // The plugin tool advertises its output shape on the registry --
    // PS7 acceptance is that the runtime's renderer walks the schema
    // for the `codeless://attachment` `$ref`. Reading the schema from
    // the registry rather than re-declaring it here means a future
    // change to the plugin's output shape automatically retests the
    // marker contract.
    let tool = registry
        .tool_registry()
        .get("notes.append")
        .expect("notes.append registered");
    let output_schema = tool.output_schema();
    let stored = upload.attachment;
    // The tool emits a "lies" hint to prove the stored row wins.
    let output_value = json!({
        "attachment": {
            "attachment_id": stored.id.to_string(),
            "mime": "text/plain",
            "filename": "lies.txt",
        },
        "summary": "noted",
    });

    let refs = find_attachment_refs(&output_schema, &output_value)
        .expect("walk schema for attachment refs");
    assert_eq!(refs.len(), 1, "exactly one attachment declared");
    assert_eq!(refs[0].attachment_id, stored.id);

    // Materialise the rows the reconciler needs to do its job. In the
    // runtime this is the `rpc::attachment::build_attachment_card`
    // wrapper; here we drive the underlying contract directly so the
    // test fails for substrate-doc reasons rather than wrapper reasons.
    let store_row = rpc
        .store()
        .get_assistant_attachment(stored.id)
        .await
        .expect("store lookup")
        .expect("row exists");
    let card = reconcile_attachment_refs(&refs, thread.id, |id| {
        if id == stored.id {
            Some(store_row.clone())
        } else {
            None
        }
    })
    .expect("reconcile");

    assert_eq!(card.kind, AssistantAttachmentCard::META_KIND);
    assert_eq!(card.items.len(), 1);
    let item = &card.items[0];
    assert_eq!(item.attachment_id, stored.id);
    // PS7 contract: the stored row's authoritative values win over the
    // tool's hints. A regression that surfaced the tool's lies would
    // flip both assertions.
    assert_eq!(item.filename, "note.md");
    assert_eq!(item.mime.as_deref(), Some("text/markdown"));
    assert_eq!(item.size_bytes, stored.size_bytes);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn plugin_tool_call_executes_through_registry() {
    // Substrate item 1 (tools layer is real and registrable): a plugin
    // tool the host has wired into `PluginRegistry` is reachable as an
    // `Arc<dyn Tool>` through `ToolRegistry::get`, with a JSON-schema
    // round-trip and a dispatchable `call`. The notes tool's runtime
    // wiring (a `notes_entries` writer + attachment minting) is
    // deferred per the PS-NOTES comment in
    // `crates/codeless-plugin-notes/src/lib.rs`, so calling it today
    // returns a structured `Failed` -- which is itself the substrate
    // contract: the tool layer is the dispatcher, and a tool that
    // returns `Failed` is the documented signal that the host has not
    // wired the per-tool ctx extension yet.
    let rpc = InProcessRpc::new().await.unwrap();
    let (registry, _loaded) = load_notes_into(&rpc).await;
    let tool = registry
        .tool_registry()
        .get("notes.append")
        .expect("notes.append registered");
    assert_eq!(tool.name(), "notes.append");

    let ctx = codeless_tools::ToolCtx::new(
        std::env::temp_dir(),
        codeless_tools::policy::NetworkMode::default(),
        codeless_tools::policy::AllowlistFile::default(),
        tokio_util::sync::CancellationToken::new(),
        tracing::Span::current(),
    );

    // Argument-shape enforcement landed with the plugin (PS-NOTES). An
    // empty body is invalid args, not a `Failed` execution -- the tool
    // is responsible for its own preconditions.
    let bad = tool.call(&ctx, json!({ "body": "   " })).await.unwrap_err();
    assert!(
        matches!(bad, codeless_tools::ToolError::InvalidArgs(_)),
        "empty body must be InvalidArgs, got {bad:?}",
    );

    // A well-formed body reaches the runtime-wiring branch and surfaces
    // `Failed` with the documented "PS-ACCEPT" message. Pinning the
    // error variant (not the message) means landing the writer in a
    // later tick will trip exactly this assertion -- the signal the
    // test was waiting for.
    let pending = tool
        .call(&ctx, json!({ "body": "remember to buy milk" }))
        .await
        .unwrap_err();
    assert!(
        matches!(pending, codeless_tools::ToolError::Failed(_)),
        "pre-wire body must be Failed, got {pending:?}",
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn planner_allow_filter_admits_plugin_tool_under_persona_namespace() {
    // Substrate item 8 (Assistant agent loop) acceptance via the
    // narrowest seam the planner exposes from outside the crate: a
    // bare `tool_allowed` lookup against the persona's `allowed_tools`
    // column. The planner-side filter in
    // `assistant_planner::run_planner_turn` is the same call, on the
    // same data; planner unit tests pin the surrounding event-bus +
    // parse-tool-call wiring, while this assertion pins the load-time
    // -> agent-time data flow: the plugin's manifest grant survives
    // through the registry, through the persona upsert, through the
    // `assistant_threads.persona_id` FK, and ends up matching the
    // tool id the agent loop will emit. A regression anywhere along
    // that chain trips here.
    let rpc = InProcessRpc::new().await.unwrap();
    let (_registry, loaded) = load_notes_into(&rpc).await;
    let persona = upsert_loaded_persona(&rpc, &loaded.personas[0]).await;
    let thread = rpc
        .create_assistant_thread(CreateAssistantThreadArgs {
            title: None,
            persona_id: persona.id.clone(),
        })
        .await
        .unwrap();
    // Re-resolve via the same accessor the planner uses at agent-call
    // time -- the persona round-tripped through SQLite must still hold
    // the plugin's grant verbatim.
    let resolved = rpc
        .store()
        .get_assistant_thread(thread.id)
        .await
        .unwrap()
        .unwrap();
    let from_db = rpc
        .store()
        .get_persona(&resolved.persona_id)
        .await
        .unwrap()
        .expect("persona row exists");
    assert!(tool_allowed(&from_db.allowed_tools, "notes.append"));
    assert!(!tool_allowed(&from_db.allowed_tools, "assistant.start_job"));
}

/// Acceptance §1 -- the substrate-doc shape rule: a new plugin is one
/// crate + plugin.toml + domains/. Asserts the on-disk layout the
/// `notes` plugin actually ships, so a future drive-by that scatters
/// plugin assets across the codeless-runtime crate (or worse, edits a
/// codeless-owned crate to register a plugin) fails the substrate
/// contract here.
#[test]
fn notes_plugin_directory_shape_matches_substrate_contract() {
    let dir = notes_plugin_dir();
    assert!(dir.join("plugin.toml").exists(), "plugin.toml present");
    assert!(
        dir.join("prompts").join("system.md").exists(),
        "prompt_file shipped under the plugin dir",
    );
    assert!(
        dir.join("migrations").join("0001_init.sql").exists(),
        "initial migration shipped under the plugin dir",
    );
    assert!(
        dir.join("domains").is_dir(),
        "domains/ data dir present (substrate-doc item 6 [data] block)",
    );

    // The plugin crate lives at the workspace's `crates/` root, not
    // under `codeless-runtime` -- substrate-doc Acceptance §1 is "no
    // change to codeless-runtime, codeless-rpc, codeless-tools (other
    // than auto-registration), or the UI". Probing for the crate
    // directly keeps that promise enforceable as long as the test
    // tree stays canonical.
    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("codeless-plugin-notes");
    assert!(
        crate_dir.join("Cargo.toml").exists(),
        "plugin crate ships next to its sibling crates",
    );
}
