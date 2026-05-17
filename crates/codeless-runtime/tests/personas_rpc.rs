//! Persona RPC surface (agent-personas stage 7). Exercises the
//! `list_personas` / `get_persona` / `upsert_persona` /
//! `delete_persona` round-trip against an in-memory `InProcessRpc` so
//! the UI's KV-as-cache mirror has a contract to lean on. Built-in
//! invariants (seeded by migration 0011) and the delete-of-built-in
//! refusal are the load-bearing assertions; the rest is plain CRUD.

use codeless_rpc::{
    DeletePersonaArgs, GetPersonaArgs, ListPersonasArgs, RpcError, RpcServer, UpsertPersonaArgs,
};
use codeless_runtime::InProcessRpc;

async fn fresh_rpc() -> InProcessRpc {
    InProcessRpc::new().await.expect("open runtime")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn list_personas_returns_seeded_built_ins() {
    let rpc = fresh_rpc().await;
    let listed = rpc
        .list_personas(ListPersonasArgs {})
        .await
        .expect("list_personas");
    let ids: Vec<&str> = listed.personas.iter().map(|p| p.id.as_str()).collect();
    // Migration 0011 seeds the five legacy job-runner personas;
    // PS5's migration 0017 adds `builtin:general` and
    // `builtin:coding` (the substrate-doc Assistant defaults).
    // Built-ins come first per the ORDER BY in `list_personas`.
    assert_eq!(
        ids,
        vec![
            "builtin:architect",
            "builtin:coder",
            "builtin:coding",
            "builtin:designer",
            "builtin:general",
            "builtin:reviewer",
            "builtin:security",
        ]
    );
    assert!(listed.personas.iter().all(|p| p.built_in));
    let coder = listed
        .personas
        .iter()
        .find(|p| p.id == "builtin:coder")
        .unwrap();
    assert_eq!(
        coder.allowed_subagents,
        vec!["explore", "code-review", "security", "general"]
    );
    assert!(coder.default_snippets.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn get_persona_round_trips_and_404s() {
    let rpc = fresh_rpc().await;
    let coder = rpc
        .get_persona(GetPersonaArgs {
            id: "builtin:coder".into(),
        })
        .await
        .expect("get builtin:coder");
    assert_eq!(coder.id, "builtin:coder");
    assert!(coder.built_in);

    let err = rpc
        .get_persona(GetPersonaArgs { id: "nope".into() })
        .await
        .unwrap_err();
    assert!(matches!(err, RpcError::NotFound(_)));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn upsert_persona_creates_and_updates_preserving_built_in_and_created_at() {
    let rpc = fresh_rpc().await;
    let created = rpc
        .upsert_persona(UpsertPersonaArgs {
            id: "user:rust".into(),
            name: "Rust Tutor".into(),
            description: "Idiomatic Rust mentor.".into(),
            icon: "coder".into(),
            instructions: "Help the user write idiomatic Rust.".into(),
            use_for_jobs: true,
            default_model: Some("claude-opus-4-7".into()),
            allowed_subagents: vec!["explore".into()],
            default_snippets: vec![],
        })
        .await
        .expect("upsert new persona");
    assert!(!created.built_in, "user-minted row must not be built-in");
    assert!(created.use_for_jobs);
    assert_eq!(created.default_model.as_deref(), Some("claude-opus-4-7"));

    let updated = rpc
        .upsert_persona(UpsertPersonaArgs {
            id: "user:rust".into(),
            name: "Rust Tutor (v2)".into(),
            description: "Idiomatic Rust mentor, refined.".into(),
            icon: "coder".into(),
            instructions: "Help the user write idiomatic Rust. Prefer iterators.".into(),
            use_for_jobs: false,
            default_model: None,
            allowed_subagents: vec![],
            default_snippets: vec!["snippet:rust-conventions".into()],
        })
        .await
        .expect("upsert existing persona");
    assert_eq!(updated.name, "Rust Tutor (v2)");
    assert!(!updated.use_for_jobs);
    assert!(updated.allowed_subagents.is_empty());
    assert_eq!(updated.default_snippets, vec!["snippet:rust-conventions"]);
    assert!(!updated.built_in);
    assert_eq!(
        updated.created_at, created.created_at,
        "created_at must be preserved across upsert"
    );

    // Editing a built-in keeps the built_in flag set so the row
    // cannot be deleted, but accepts body edits.
    let edited = rpc
        .upsert_persona(UpsertPersonaArgs {
            id: "builtin:coder".into(),
            name: "Coder (custom)".into(),
            description: "Adjusted by user.".into(),
            icon: "coder".into(),
            instructions: "Custom prompt for the built-in Coder.".into(),
            use_for_jobs: true,
            default_model: None,
            allowed_subagents: vec!["explore".into()],
            default_snippets: vec![],
        })
        .await
        .expect("upsert edits built-in");
    assert!(edited.built_in);
    assert_eq!(edited.name, "Coder (custom)");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn upsert_persona_rejects_empty_required_fields() {
    let rpc = fresh_rpc().await;
    let bad_id = rpc
        .upsert_persona(UpsertPersonaArgs {
            id: "".into(),
            name: "x".into(),
            description: "".into(),
            icon: "".into(),
            instructions: "y".into(),
            use_for_jobs: false,
            default_model: None,
            allowed_subagents: vec![],
            default_snippets: vec![],
        })
        .await
        .unwrap_err();
    assert!(matches!(bad_id, RpcError::InvalidArgument(_)));

    let bad_instructions = rpc
        .upsert_persona(UpsertPersonaArgs {
            id: "user:x".into(),
            name: "x".into(),
            description: "".into(),
            icon: "".into(),
            instructions: "   ".into(),
            use_for_jobs: false,
            default_model: None,
            allowed_subagents: vec![],
            default_snippets: vec![],
        })
        .await
        .unwrap_err();
    assert!(matches!(bad_instructions, RpcError::InvalidArgument(_)));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn delete_persona_refuses_built_ins_and_removes_user_rows() {
    let rpc = fresh_rpc().await;
    let err = rpc
        .delete_persona(DeletePersonaArgs {
            id: "builtin:coder".into(),
        })
        .await
        .unwrap_err();
    assert!(matches!(err, RpcError::Conflict(_)));

    let not_found = rpc
        .delete_persona(DeletePersonaArgs {
            id: "user:ghost".into(),
        })
        .await
        .unwrap_err();
    assert!(matches!(not_found, RpcError::NotFound(_)));

    rpc.upsert_persona(UpsertPersonaArgs {
        id: "user:doomed".into(),
        name: "Doomed".into(),
        description: "".into(),
        icon: "spark".into(),
        instructions: "transient".into(),
        use_for_jobs: false,
        default_model: None,
        allowed_subagents: vec![],
        default_snippets: vec![],
    })
    .await
    .unwrap();
    rpc.delete_persona(DeletePersonaArgs {
        id: "user:doomed".into(),
    })
    .await
    .expect("delete user persona");
    let after = rpc
        .get_persona(GetPersonaArgs {
            id: "user:doomed".into(),
        })
        .await
        .unwrap_err();
    assert!(matches!(after, RpcError::NotFound(_)));
}
