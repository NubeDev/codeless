//! End-to-end test of the MCP prompts surface against the
//! codeless-mcp binary backed by a real (temp) SQLite database.
//!
//! Seeds two personas — one with `use_for_jobs = 1`, one without —
//! starts the binary with `CODELESS_DB_PATH` pointing at the file,
//! drives a `prompts/list` and a `prompts/get`. The DB path round-
//! trip is the only way to prove that the bin actually opens the
//! store the runtime writes to.

use codeless_mcp::personas::open_sqlite_persona_source;
use codeless_runtime::SqliteStore;
use codeless_types::{Persona, UnixMillis};
use rmcp::model::GetPromptRequestParams;
use rmcp::service::ServiceExt;
use rmcp::transport::{ConfigureCommandExt, TokioChildProcess};
use tokio::process::Command;

fn server_binary() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_codeless-mcp"))
}

fn persona(id: &str, name: &str, instructions: &str, use_for_jobs: bool) -> Persona {
    Persona {
        id: id.into(),
        name: name.into(),
        description: format!("desc for {id}"),
        icon: "coder".into(),
        instructions: instructions.into(),
        use_for_jobs,
        default_model: None,
        allowed_subagents: Vec::new(),
        default_snippets: Vec::new(),
        allowed_tools: Vec::new(),
        default_model_family: None,
        default_attachments_policy: "inline-thread-scoped".into(),
        built_in: false,
        created_at: UnixMillis::from(0),
        updated_at: UnixMillis::from(0),
    }
}

async fn seed_db(path: &std::path::Path) {
    let source = open_sqlite_persona_source(path)
        .await
        .expect("open db for seeding");
    // The helper hides the store; seed by opening a parallel pool.
    let opts = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(false);
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await
        .expect("seed pool");
    let store = SqliteStore::new(pool);
    store
        .upsert_persona(&persona(
            "user:coder",
            "User Coder",
            "INSTRUCTIONS-FOR-CODER",
            true,
        ))
        .await
        .expect("upsert exposed");
    store
        .upsert_persona(&persona(
            "user:chat-only",
            "Chat Only",
            "secret-chat-only-instructions",
            false,
        ))
        .await
        .expect("upsert hidden");
    drop(source);
}

#[tokio::test]
async fn list_and_get_prompts_via_stdio() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("codeless.db");
    seed_db(&db_path).await;

    let bin = server_binary();
    let db_path_str = db_path.to_string_lossy().to_string();
    let client = ()
        .serve(
            TokioChildProcess::new(Command::new(&bin).configure(|cmd| {
                cmd.env(
                    "CODELESS_WORKTREE_ROOT",
                    std::env::temp_dir().to_string_lossy().as_ref(),
                );
                cmd.env("CODELESS_DB_PATH", &db_path_str);
            }))
            .expect("spawn codeless-mcp"),
        )
        .await
        .expect("mcp init handshake");

    let prompts = client
        .list_prompts(Default::default())
        .await
        .expect("list_prompts");
    let names: Vec<String> = prompts.prompts.iter().map(|p| p.name.clone()).collect();
    assert!(
        names.iter().any(|n| n == "user:coder"),
        "expected user:coder in {names:?}"
    );
    assert!(
        names.iter().all(|n| n != "user:chat-only"),
        "chat-only persona must not be exposed: {names:?}"
    );
    // Built-in seeded personas all ship with use_for_jobs = 0, so the
    // surface should hold only user-promoted personas right now.
    assert!(
        names.iter().all(|n| !n.starts_with("builtin:")),
        "no seeded builtin should be exposed by default: {names:?}"
    );

    let got = client
        .get_prompt(GetPromptRequestParams::new("user:coder"))
        .await
        .expect("get_prompt");
    assert_eq!(got.messages.len(), 1);
    let text = match &got.messages[0].content {
        rmcp::model::PromptMessageContent::Text { text } => text.clone(),
        other => panic!("expected text, got {other:?}"),
    };
    assert_eq!(text, "INSTRUCTIONS-FOR-CODER");

    // A chat-only persona must surface as a protocol-level error,
    // not as a silent empty body.
    let err = client
        .get_prompt(GetPromptRequestParams::new("user:chat-only"))
        .await
        .expect_err("chat-only must not be gettable");
    let msg = err.to_string();
    assert!(
        msg.contains("not exposed") || msg.contains("use_for_jobs") || msg.contains("unknown"),
        "got err={msg}"
    );

    client.cancel().await.expect("clean shutdown");
}
