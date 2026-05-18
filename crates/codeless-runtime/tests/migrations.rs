//! Applies the embedded migrator to an in-memory SQLite database and
//! checks every table/index from SCOPE.md Appendix A landed with the
//! expected column names. The point is to catch schema drift the
//! moment a migration touches the wrong column name — Phase 3 will
//! generate `sqlx::query!` calls referencing these names.

use codeless_runtime::MIGRATOR;
use sqlx::{sqlite::SqlitePoolOptions, Row, SqlitePool};

async fn fresh_db() -> SqlitePool {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("open in-memory sqlite");
    MIGRATOR.run(&pool).await.expect("apply migrations");
    pool
}

async fn columns(pool: &SqlitePool, table: &str) -> Vec<String> {
    let rows = sqlx::query(&format!("PRAGMA table_info({table})"))
        .fetch_all(pool)
        .await
        .expect("table_info");
    rows.into_iter()
        .map(|r| r.get::<String, _>("name"))
        .collect()
}

async fn index_names(pool: &SqlitePool) -> Vec<String> {
    let rows = sqlx::query(
        "SELECT name FROM sqlite_master WHERE type = 'index' AND name NOT LIKE 'sqlite_%' \
         ORDER BY name",
    )
    .fetch_all(pool)
    .await
    .expect("indexes");
    rows.into_iter()
        .map(|r| r.get::<String, _>("name"))
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn migrator_creates_all_tables_from_appendix_a() {
    let pool = fresh_db().await;
    let tables: Vec<String> = sqlx::query(
        "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' \
         AND name NOT LIKE '_sqlx_%' ORDER BY name",
    )
    .fetch_all(&pool)
    .await
    .unwrap()
    .into_iter()
    .map(|r| r.get::<String, _>("name"))
    .collect();
    assert_eq!(
        tables,
        vec![
            "assistant_attachments".to_string(),
            "assistant_messages".to_string(),
            "assistant_threads".to_string(),
            "attached_workspaces".to_string(),
            "chat_bindings".to_string(),
            "chat_messages".to_string(),
            "events".to_string(),
            "jobs".to_string(),
            "personas".to_string(),
            "pty_sessions".to_string(),
            "repos".to_string(),
            "reviews".to_string(),
            "scheduled_pause_points".to_string(),
            "stages".to_string(),
            "supervisor_goals".to_string(),
            "tasks".to_string(),
            "todos".to_string(),
        ]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn assistant_tables_match_stage_5_schema() {
    let pool = fresh_db().await;
    assert_eq!(
        columns(&pool, "assistant_threads").await,
        vec!["id", "title", "persona_id", "created_at", "updated_at"],
    );
    assert_eq!(
        columns(&pool, "assistant_messages").await,
        vec![
            "id",
            "thread_id",
            "role",
            "content",
            "meta_json",
            "created_at",
        ],
    );
    assert_eq!(
        columns(&pool, "assistant_attachments").await,
        vec![
            "id",
            "thread_id",
            "original_name",
            "stored_filename",
            "mime_type",
            "size_bytes",
            "created_at",
        ],
    );

    let idx = index_names(&pool).await;
    for required in [
        "assistant_threads_updated_idx",
        "assistant_messages_thread_idx",
        "assistant_attachments_thread_idx",
    ] {
        assert!(
            idx.contains(&required.to_string()),
            "missing assistant index {required}; got {idx:?}"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn repos_columns_match_appendix_a() {
    let pool = fresh_db().await;
    assert_eq!(
        columns(&pool, "repos").await,
        vec![
            "id",
            "name",
            "clone_url",
            "default_branch",
            "local_path",
            "git_auth",
            "concurrency_cap",
            "default_runner",
            "created_at",
            "updated_at",
        ],
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn jobs_columns_match_appendix_a() {
    let pool = fresh_db().await;
    assert_eq!(
        columns(&pool, "jobs").await,
        vec![
            "id",
            "repo_id",
            "status",
            "stop_reason",
            "template_yaml",
            "prompt",
            "runner",
            "branch",
            "worktree_path",
            "cost_cap_cents",
            "wall_clock_cap_ms",
            "cost_cents",
            "started_at",
            "ended_at",
            "created_at",
            "model",
            "permission_mode",
            "effort",
            "workspace_mode",
            "system_prompt",
            "persona_id",
            "auto_bypass_policy",
            "pending_operator_comment",
            "precheck_override_once",
        ],
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tasks_columns_include_depends_on_and_lease_fields() {
    let pool = fresh_db().await;
    let cols = columns(&pool, "tasks").await;
    for required in [
        "depends_on",
        "lease_holder",
        "lease_expires_at",
        "cost_cents",
        "input_tokens",
        "output_tokens",
    ] {
        assert!(
            cols.contains(&required.to_string()),
            "missing {required} in tasks"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn events_use_cursor_autoincrement_primary_key() {
    let pool = fresh_db().await;
    sqlx::query(
        "INSERT INTO events (job_id, type, payload, created_at) \
         VALUES (NULL, 'job-started', '{}', 1)",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO events (job_id, type, payload, created_at) \
         VALUES (NULL, 'job-completed', '{}', 2)",
    )
    .execute(&pool)
    .await
    .unwrap();

    let cursors: Vec<i64> = sqlx::query("SELECT cursor FROM events ORDER BY cursor")
        .fetch_all(&pool)
        .await
        .unwrap()
        .into_iter()
        .map(|r| r.get::<i64, _>("cursor"))
        .collect();
    assert_eq!(cursors, vec![1, 2]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn expected_indexes_are_present() {
    let pool = fresh_db().await;
    let idx = index_names(&pool).await;
    for required in [
        "events_created_at_idx",
        "events_job_cursor_idx",
        "jobs_repo_idx",
        "jobs_status_idx",
        "pty_idle_idx",
        "reviews_status_idx",
        "stages_job_idx",
        "tasks_lease_expiry_idx",
        "tasks_stage_idx",
    ] {
        assert!(
            idx.contains(&required.to_string()),
            "missing index {required}"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn attached_workspaces_columns_and_unique_canonical_index() {
    let pool = fresh_db().await;
    assert_eq!(
        columns(&pool, "attached_workspaces").await,
        vec![
            "repo_id",
            "fs_root_canonical",
            "fs_root_display",
            "attached_at",
        ],
    );
    assert!(
        index_names(&pool)
            .await
            .contains(&"idx_attached_workspaces_canonical".to_string()),
        "missing idx_attached_workspaces_canonical",
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn personas_table_matches_schema_sketch_and_seeds_built_ins() {
    let pool = fresh_db().await;
    assert_eq!(
        columns(&pool, "personas").await,
        vec![
            "id",
            "name",
            "description",
            "icon",
            "instructions",
            "use_for_jobs",
            "default_model",
            "allowed_subagents",
            "default_snippets",
            "built_in",
            "created_at",
            "updated_at",
            // PS5 (DOCS/PLUGIN-SUBSTRATE.md item 5) — the substrate-
            // doc allowed-tools list, model-family alias the runner
            // resolves at call time, and per-thread attachments
            // policy. Slotted at the tail because SQLite's ALTER
            // TABLE ADD COLUMN appends; the order is migration-
            // driven and the on-disk contract, so do not reorder
            // without a fresh table-rebuild migration.
            "allowed_tools",
            "default_model_family",
            "default_attachments_policy",
        ],
    );
    assert!(
        index_names(&pool)
            .await
            .contains(&"personas_use_for_jobs_idx".to_string()),
        "missing personas_use_for_jobs_idx",
    );

    let seeded: Vec<(String, i64)> =
        sqlx::query("SELECT id, built_in FROM personas WHERE built_in = 1 ORDER BY id")
            .fetch_all(&pool)
            .await
            .expect("query seeded personas")
            .into_iter()
            .map(|r| (r.get::<String, _>("id"), r.get::<i64, _>("built_in")))
            .collect();
    assert_eq!(
        seeded,
        vec![
            ("builtin:architect".to_string(), 1),
            ("builtin:coder".to_string(), 1),
            // PS5: substrate-doc-mandated default Assistant personas
            // (`general` and `coding`) seeded alongside the legacy
            // five job-runner personas. The two sets coexist —
            // `general` / `coding` are the Assistant defaults the
            // substrate doc names, the legacy five are referenced by
            // the existing job-side persona picker.
            ("builtin:coding".to_string(), 1),
            ("builtin:designer".to_string(), 1),
            ("builtin:general".to_string(), 1),
            ("builtin:reviewer".to_string(), 1),
            ("builtin:security".to_string(), 1),
        ],
    );

    // allowed_subagents / default_snippets are JSON arrays the
    // application-side serde parses; the migration must store valid
    // JSON so the next stage's read path does not need a fallback for
    // built-in rows.
    let coder_subagents: String =
        sqlx::query("SELECT allowed_subagents FROM personas WHERE id = 'builtin:coder'")
            .fetch_one(&pool)
            .await
            .unwrap()
            .get("allowed_subagents");
    let parsed: serde_json::Value =
        serde_json::from_str(&coder_subagents).expect("allowed_subagents parses as JSON");
    assert!(parsed.is_array(), "allowed_subagents must be a JSON array");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stages_table_includes_goal_and_acceptance() {
    let pool = fresh_db().await;
    let cols = columns(&pool, "stages").await;
    for required in ["goal", "acceptance"] {
        assert!(
            cols.contains(&required.to_string()),
            "stages table missing {required}; got {cols:?}"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stages_table_carries_persona_id_with_fk_to_personas() {
    let pool = fresh_db().await;
    let cols = columns(&pool, "stages").await;
    assert!(
        cols.contains(&"persona_id".to_string()),
        "stages table missing persona_id; got {cols:?}"
    );

    let fks: Vec<(String, String)> = sqlx::query("PRAGMA foreign_key_list(stages)")
        .fetch_all(&pool)
        .await
        .expect("fk list")
        .into_iter()
        .map(|r| (r.get::<String, _>("table"), r.get::<String, _>("from")))
        .collect();
    assert!(
        fks.iter()
            .any(|(t, f)| t == "personas" && f == "persona_id"),
        "stages.persona_id missing FK to personas; got {fks:?}",
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn assistant_threads_persona_id_carries_fk_to_personas() {
    // PS5 acceptance (DOCS/PLUGIN-SUBSTRATE.md item 5): the column is
    // NOT NULL and points at `personas(id)`. The FK is ON DELETE
    // RESTRICT so deleting a persona while threads point at it is
    // refused at the schema level -- the runner must be able to
    // reproduce the agent posture for the lifetime of the thread.
    let pool = fresh_db().await;
    let fks: Vec<(String, String, String)> =
        sqlx::query("PRAGMA foreign_key_list(assistant_threads)")
            .fetch_all(&pool)
            .await
            .expect("fk list")
            .into_iter()
            .map(|r| {
                (
                    r.get::<String, _>("table"),
                    r.get::<String, _>("from"),
                    r.get::<String, _>("on_delete"),
                )
            })
            .collect();
    assert!(
        fks.iter()
            .any(|(t, f, od)| t == "personas" && f == "persona_id" && od == "RESTRICT"),
        "assistant_threads.persona_id missing FK to personas (ON DELETE RESTRICT); got {fks:?}",
    );

    let notnull: i64 = sqlx::query(
        "SELECT \"notnull\" FROM pragma_table_info('assistant_threads') WHERE name = 'persona_id'",
    )
    .fetch_one(&pool)
    .await
    .expect("table_info row for persona_id")
    .get("notnull");
    assert_eq!(notnull, 1, "assistant_threads.persona_id must be NOT NULL");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn substrate_doc_personas_are_seeded_with_new_columns() {
    // PS5 (DOCS/PLUGIN-SUBSTRATE.md item 5): `builtin:general` and
    // `builtin:coding` are the substrate-doc-mandated defaults; the
    // migration must seed them with valid JSON `allowed_tools` and a
    // populated `default_attachments_policy` so the read path needs
    // no fallback.
    let pool = fresh_db().await;
    for id in ["builtin:general", "builtin:coding"] {
        let row = sqlx::query(
            "SELECT allowed_tools, default_model_family, default_attachments_policy \
             FROM personas WHERE id = ?",
        )
        .bind(id)
        .fetch_one(&pool)
        .await
        .unwrap_or_else(|_| panic!("seeded persona {id} present"));
        let raw: String = row.get("allowed_tools");
        let parsed: serde_json::Value =
            serde_json::from_str(&raw).expect("allowed_tools parses as JSON");
        assert!(parsed.is_array(), "{id}.allowed_tools must be a JSON array",);
        let policy: String = row.get("default_attachments_policy");
        assert!(
            !policy.is_empty(),
            "{id}.default_attachments_policy must be populated",
        );
        let _model_family: Option<String> = row.get("default_model_family");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scheduled_pause_points_table_keys_on_job_and_ordinal() {
    // SCOPED-PAUSE-POINTS §4: the row key is the YAML position
    // (`ordinal`), not the resolved stage ordinal, so a resync that
    // renumbers stages does not orphan rows. The composite primary
    // key plus the explicit job-ordered index back that contract.
    let pool = fresh_db().await;
    assert_eq!(
        columns(&pool, "scheduled_pause_points").await,
        vec![
            "job_id",
            "ordinal",
            "point_id",
            "target_json",
            "position",
            "reason",
            "created_at",
        ],
    );
    assert!(index_names(&pool)
        .await
        .contains(&"scheduled_pause_points_job_idx".to_string()));

    let fks: Vec<(String, String, String)> =
        sqlx::query("PRAGMA foreign_key_list(scheduled_pause_points)")
            .fetch_all(&pool)
            .await
            .unwrap()
            .into_iter()
            .map(|r| {
                (
                    r.get::<String, _>("table"),
                    r.get::<String, _>("from"),
                    r.get::<String, _>("on_delete"),
                )
            })
            .collect();
    assert!(
        fks.iter()
            .any(|(t, f, od)| t == "jobs" && f == "job_id" && od == "CASCADE"),
        "scheduled_pause_points.job_id must FK jobs.id ON DELETE CASCADE; got {fks:?}",
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn jobs_persona_id_was_promoted_to_personas_fk() {
    let pool = fresh_db().await;
    let fks: Vec<(String, String)> = sqlx::query("PRAGMA foreign_key_list(jobs)")
        .fetch_all(&pool)
        .await
        .expect("fk list")
        .into_iter()
        .map(|r| (r.get::<String, _>("table"), r.get::<String, _>("from")))
        .collect();
    assert!(
        fks.iter()
            .any(|(t, f)| t == "personas" && f == "persona_id"),
        "jobs.persona_id missing FK to personas; got {fks:?}",
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn chat_messages_columns_match_job_chat_doc() {
    // JOB-CHAT.md §Data model. Column order matters because it is
    // the on-disk contract; SQLite ALTER TABLE ADD COLUMN would
    // append later additions, so a rebuild migration is required to
    // reorder.
    let pool = fresh_db().await;
    assert_eq!(
        columns(&pool, "chat_messages").await,
        vec![
            "id",
            "job_id",
            "run_id",
            "transport",
            "external_id",
            "thread_key",
            "author",
            "role",
            "body",
            "metadata_json",
            "created_at",
        ],
    );

    let idx = index_names(&pool).await;
    for required in ["chat_messages_job_idx", "chat_messages_external_idx"] {
        assert!(
            idx.contains(&required.to_string()),
            "missing chat index {required}; got {idx:?}"
        );
    }

    // The (transport, external_id) UNIQUE index is partial — it
    // narrows to non-NULL external_id rows so web / cli / supervisor
    // inserts (which legitimately leave external_id NULL) do not
    // collide. Verify the partial predicate is present.
    let partial_sql: String = sqlx::query(
        "SELECT sql FROM sqlite_master WHERE type = 'index' \
         AND name = 'chat_messages_external_idx'",
    )
    .fetch_one(&pool)
    .await
    .expect("read partial-unique index DDL")
    .get("sql");
    assert!(
        partial_sql.to_ascii_uppercase().contains("WHERE"),
        "chat_messages_external_idx must be partial; got {partial_sql}",
    );
    assert!(
        partial_sql.contains("external_id"),
        "chat_messages_external_idx partial predicate must reference external_id; got {partial_sql}",
    );

    let fks: Vec<(String, String)> = sqlx::query("PRAGMA foreign_key_list(chat_messages)")
        .fetch_all(&pool)
        .await
        .expect("fk list")
        .into_iter()
        .map(|r| (r.get::<String, _>("table"), r.get::<String, _>("from")))
        .collect();
    assert!(
        fks.iter().any(|(t, f)| t == "jobs" && f == "job_id"),
        "chat_messages.job_id missing FK to jobs; got {fks:?}",
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn chat_messages_partial_unique_allows_null_external_id_duplicates() {
    // The whole point of the partial unique is that web / cli /
    // supervisor rows (external_id = NULL) do not collide with each
    // other. Two supervisor rows inserted in succession must both
    // land.
    let pool = fresh_db().await;
    seed_job(&pool, "job-A").await;
    for id in ["msg-1", "msg-2"] {
        sqlx::query(
            "INSERT INTO chat_messages \
             (id, job_id, transport, author, role, body, created_at) \
             VALUES (?, 'job-A', 'supervisor', 'supervisor', 'assistant', 'hi', 0)",
        )
        .bind(id)
        .execute(&pool)
        .await
        .expect("insert supervisor row with NULL external_id");
    }

    // And the partial unique still rejects a duplicate (telegram,
    // external_id) pair — the defence is intact for the transports
    // whose invariant requires external_id.
    sqlx::query(
        "INSERT INTO chat_messages \
         (id, job_id, transport, external_id, author, role, body, created_at) \
         VALUES ('msg-tg-1', 'job-A', 'telegram', 'tg:1', 'alice', 'user', 'hi', 1)",
    )
    .execute(&pool)
    .await
    .expect("first telegram row");
    let dup = sqlx::query(
        "INSERT INTO chat_messages \
         (id, job_id, transport, external_id, author, role, body, created_at) \
         VALUES ('msg-tg-2', 'job-A', 'telegram', 'tg:1', 'alice', 'user', 'hi', 2)",
    )
    .execute(&pool)
    .await;
    assert!(
        dup.is_err(),
        "partial unique must reject duplicate (telegram, external_id) pair",
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn chat_bindings_columns_and_thread_id_sentinel_default() {
    let pool = fresh_db().await;
    assert_eq!(
        columns(&pool, "chat_bindings").await,
        vec![
            "transport",
            "channel_id",
            "thread_id",
            "job_id",
            "bound_at",
            "bound_by",
        ],
    );

    // thread_id is NOT NULL with an empty-string sentinel default —
    // the unique-by-PK invariant depends on it (see migration
    // header). Verify both the notnull flag and the default value.
    let row = sqlx::query(
        "SELECT \"notnull\", dflt_value FROM pragma_table_info('chat_bindings') \
         WHERE name = 'thread_id'",
    )
    .fetch_one(&pool)
    .await
    .expect("thread_id pragma row");
    let notnull: i64 = row.get("notnull");
    let dflt: String = row.get("dflt_value");
    assert_eq!(notnull, 1, "chat_bindings.thread_id must be NOT NULL");
    assert_eq!(
        dflt.trim_matches('\''),
        "",
        "chat_bindings.thread_id default must be the empty-string sentinel; got {dflt}",
    );

    let fks: Vec<(String, String)> = sqlx::query("PRAGMA foreign_key_list(chat_bindings)")
        .fetch_all(&pool)
        .await
        .expect("fk list")
        .into_iter()
        .map(|r| (r.get::<String, _>("table"), r.get::<String, _>("from")))
        .collect();
    assert!(
        fks.iter().any(|(t, f)| t == "jobs" && f == "job_id"),
        "chat_bindings.job_id missing FK to jobs; got {fks:?}",
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn migrator_is_replay_idempotent_on_populated_db() {
    // sqlx's migrator hashes each migration and refuses to start if
    // the hash on disk drifts from the recorded one. The check
    // proves the new chat_* migrations record cleanly and that
    // running the migrator a second time on a populated DB is a
    // no-op rather than a re-create / collision.
    let pool = fresh_db().await;

    seed_job(&pool, "job-A").await;
    sqlx::query(
        "INSERT INTO chat_messages \
         (id, job_id, transport, author, role, body, created_at) \
         VALUES ('m-1', 'job-A', 'web', 'alice', 'user', 'hello', 1)",
    )
    .execute(&pool)
    .await
    .expect("seed chat_messages row");
    sqlx::query(
        "INSERT INTO chat_bindings \
         (transport, channel_id, thread_id, job_id, bound_at, bound_by) \
         VALUES ('telegram', 'C1', '', 'job-A', 1, 'alice')",
    )
    .execute(&pool)
    .await
    .expect("seed chat_bindings row");

    MIGRATOR
        .run(&pool)
        .await
        .expect("replay migrations on populated DB is a no-op");

    let chat_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM chat_messages")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(chat_count, 1, "replay must not drop chat_messages rows");
    let binding_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM chat_bindings")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(binding_count, 1, "replay must not drop chat_bindings rows");
}

async fn seed_job(pool: &SqlitePool, id: &str) {
    // Minimal repos + jobs pair to satisfy the chat_messages /
    // chat_bindings FK to jobs(id) (which itself FKs jobs.repo_id
    // into repos.id; both columns are NOT NULL). All other columns
    // are either nullable or have schema defaults; template_yaml /
    // prompt / runner / branch are NOT NULL with no default, so the
    // seed gives them empty placeholders.
    let repo_id = format!("repo-for-{id}");
    sqlx::query(
        "INSERT OR IGNORE INTO repos \
         (id, name, clone_url, default_branch, local_path, git_auth, \
          concurrency_cap, default_runner, created_at, updated_at) \
         VALUES (?, 'seed', '', 'main', '', '{}', 1, 'claude', 0, 0)",
    )
    .bind(&repo_id)
    .execute(pool)
    .await
    .expect("seed repos row");
    sqlx::query(
        "INSERT INTO jobs \
         (id, repo_id, status, template_yaml, prompt, runner, branch, \
          cost_cap_cents, wall_clock_cap_ms, created_at) \
         VALUES (?, ?, 'enqueued', '', '', 'claude', 'main', 0, 0, 0)",
    )
    .bind(id)
    .bind(&repo_id)
    .execute(pool)
    .await
    .expect("seed jobs row");
}
