//! Per-Run supervisor task (JOB-CHAT.md (C2)).
//!
//! Lifetime
//! --------
//! One supervisor task per Run. `drive_job` spawns it after the row
//! transitions to `Running` and the `JobStarted` event is published;
//! the task exits when it observes a Run terminal event
//! (`JobCompleted` / `JobFailed` / `JobStopped`). `AwaitingReview` is
//! not terminal — the supervisor stays alive across a review gate so
//! the same conversation thread persists. A resumed Run spawns a
//! fresh supervisor; supervisors are per-Run-attempt, not per-Job.
//!
//! Crate placement (R1 of CLAUDE.md / JOB-CHAT.md "Crate placement").
//! ------------------------------------------------------------------
//! The supervisor lives inside `codeless-runtime` as a module — never
//! a separate `codeless-supervisor` crate. The module never imports
//! the stdlib or tokio process-spawn surfaces; any action the
//! supervisor gains in a future stage routes through the existing
//! RPCs on `InProcessRpc`. The `lint` module below has a `cargo test`
//! grep that fails the build if either process-spawn import name
//! re-appears in this module's source.
//!
//! Voice contract (the load-bearing C2 invariant)
//! ----------------------------------------------
//! The supervisor's only outbound channel is `Tools::post_chat_message`,
//! which mirrors `post_job_message` with `transport='supervisor'`.
//! Concretely, this module is forbidden from:
//!
//! - the stdlib print macros (user-visible stdout/stderr noise),
//! - the loud tracing macros at info / warn / error level (those
//!   surface to operator dashboards; supervisor reasoning belongs in
//!   the chat thread, not in the log stream — the debug-level macro
//!   stays permitted for engineer-only diagnostics),
//! - calling `publish` on the event bus directly (every supervisor
//!   utterance has to go through the tool surface so the per-Job
//!   chat log is the single source of truth and the asymmetric
//!   echo-suppression rule in `codeless-bot-core` still applies),
//! - importing the stdlib `process` module or tokio's `process`
//!   module (R1 of CLAUDE.md; both are spelled with the usual `::`
//!   separator and are forbidden by the lint fragment below;
//!   process-spawn confinement is enforced by grep here and by the
//!   separate crate-layout check in `codeless-adapters-host`).
//!
//! Tool surface (this stage)
//! -------------------------
//! Read tools live in `tools.rs`: `get_job_state`, `read_events`,
//! `read_handover`, `read_template`, `read_stage_log`, `read_notes`.
//! Each routes through existing `SqliteStore` / `EventBus` reads.
//! The single write tool is `post_chat_message`. The reactor below
//! pattern-matches "what stage" questions and answers from
//! `get_job_state`; richer LLM-driven dispatch is the next stage's
//! problem.

use std::sync::Arc;

use codeless_types::{ChatTransport, Event, JobId};
use futures_util::StreamExt;
use tokio::task::JoinHandle;

use crate::event_bus::{EventBus, SubscribeFilter};
use crate::store::SqliteStore;

pub mod tools;
pub use tools::{JobStateView, NoteFile, StageSummary, ToolError, Tools};

/// Spawn the per-Run supervisor task. Lifecycle-only entry: the task
/// subscribes to the Job's event stream and self-terminates on a Run
/// terminal envelope, but it does not dispatch chat replies because
/// it does not have a `SqliteStore` handle. Existing callers
/// (`drive_job`, the lifecycle integration tests) use this entry
/// point until the driver wiring upgrade lands.
pub fn spawn_supervisor(bus: Arc<EventBus>, job_id: JobId) -> JoinHandle<()> {
    tokio::spawn(run_lifecycle_only(bus, job_id))
}

/// Spawn the per-Run supervisor task with the full tool surface
/// available to its chat reactor. The reactor watches the same
/// `ChatMessageAppended` stream the lifecycle-only path watches and,
/// when it sees a non-supervisor message that asks "what stage is it
/// on?", calls `get_job_state` and replies via `post_chat_message`.
/// The tool-equipped variant is the one the e2e test
/// `supervisor_answers_what_stage_is_it_on` exercises.
pub fn spawn_supervisor_with_tools(
    bus: Arc<EventBus>,
    store: Arc<SqliteStore>,
    job_id: JobId,
) -> JoinHandle<()> {
    tokio::spawn(run_with_tools(Tools::new(bus, store), job_id))
}

async fn run_lifecycle_only(bus: Arc<EventBus>, job_id: JobId) {
    let mut stream = match bus
        .subscribe_since(SubscribeFilter::Job(job_id), None)
        .await
    {
        Ok(s) => s,
        Err(e) => {
            tracing::debug!(%job_id, error = %e, "supervisor subscribe failed; exiting");
            return;
        }
    };
    tracing::debug!(%job_id, "supervisor started");
    while let Some(item) = stream.next().await {
        let env = match item {
            Ok(env) => env,
            Err(_e) => {
                tracing::debug!(%job_id, "supervisor stream error; exiting");
                return;
            }
        };
        match env.event {
            Event::JobCompleted { .. } | Event::JobFailed { .. } | Event::JobStopped { .. } => {
                tracing::debug!(%job_id, "supervisor observed run terminal; exiting");
                return;
            }
            Event::ChatMessageAppended { .. } => {
                tracing::debug!(%job_id, "supervisor saw ChatMessageAppended");
            }
            _ => {}
        }
    }
}

async fn run_with_tools(tools: Tools, job_id: JobId) {
    let bus = tools.bus_arc();
    let mut stream = match bus
        .subscribe_since(SubscribeFilter::Job(job_id), None)
        .await
    {
        Ok(s) => s,
        Err(e) => {
            tracing::debug!(%job_id, error = %e, "supervisor subscribe failed; exiting");
            return;
        }
    };
    tracing::debug!(%job_id, "supervisor (with tools) started");
    while let Some(item) = stream.next().await {
        let env = match item {
            Ok(env) => env,
            Err(_e) => {
                tracing::debug!(%job_id, "supervisor stream error; exiting");
                return;
            }
        };
        match env.event {
            Event::JobCompleted { .. } | Event::JobFailed { .. } | Event::JobStopped { .. } => {
                tracing::debug!(%job_id, "supervisor observed run terminal; exiting");
                return;
            }
            Event::ChatMessageAppended { ref message, .. } => {
                // Echo-suppression on the supervisor's own messages:
                // the reactor would otherwise loop forever, replying
                // to its own reply. Every non-supervisor transport is
                // a candidate for a reply.
                if matches!(message.transport, ChatTransport::Supervisor) {
                    continue;
                }
                react_to_chat(&tools, job_id, &message.body).await;
            }
            _ => {}
        }
    }
}

/// Stage-10 reactor dispatch: a single hand-rolled matcher that pins
/// "what stage is it on?" against `get_job_state`. The matcher stays
/// deliberately narrow so the stage-10 contract is "the supervisor
/// answers the one specific question its tool surface can ground in
/// the DB," not "the supervisor improvises." LLM-driven dispatch
/// against the full tool set lands in a later stage.
async fn react_to_chat(tools: &Tools, job_id: JobId, body: &str) {
    let lower = body.to_ascii_lowercase();
    let asks_what_stage = lower.contains("what stage") || lower.contains("which stage");
    if !asks_what_stage {
        return;
    }
    let reply = match tools.get_job_state(job_id).await {
        Ok(view) => match view.current_stage {
            Some(stage) => format!(
                "Currently on stage {} ({}). Status: {:?}.",
                stage.ordinal, stage.name, stage.status,
            ),
            None => "No stage has started yet for this Run.".to_string(),
        },
        Err(ToolError::NotFound) => "Job not found.".to_string(),
        Err(_) => "I could not read the stage list right now.".to_string(),
    };
    // `post_chat_message` is the supervisor's only voice. A failure
    // here means the bus or store is wedged; nothing the supervisor
    // can usefully do beyond stay alive and pick up the next event.
    let _ = tools.post_chat_message(job_id, reply).await;
}

#[cfg(test)]
mod lint {
    //! Static grep that the voice contract above is honoured. The
    //! forbidden tokens are built from string fragments so this test
    //! file itself does not contain the literals that would otherwise
    //! trip the grep.

    const SUPERVISOR_SRC: &str = include_str!("mod.rs");

    fn forbidden_tokens() -> Vec<String> {
        vec![
            // eprintln! / println!
            ["epr", "intln!"].concat(),
            ["prin", "tln!"].concat(),
            // Direct event-bus writes (the only legitimate write-out
            // is via `Tools::post_chat_message`).
            ["bus.", "publish"].concat(),
            // User-surface tracing levels. `tracing::debug!` stays
            // permitted; the three loud levels do not.
            ["traci", "ng::info!"].concat(),
            ["traci", "ng::warn!"].concat(),
            ["traci", "ng::error!"].concat(),
            // Process-spawn imports forbidden by R1 of CLAUDE.md and
            // by JOB-CHAT.md "Hard rule 2". The supervisor must
            // route any future action through existing RPCs in
            // `codeless-adapters-host`, never spawn its own process.
            ["std::", "process"].concat(),
            ["tokio::", "process"].concat(),
        ]
    }

    #[test]
    fn supervisor_module_source_has_no_forbidden_outbound_calls() {
        let src = SUPERVISOR_SRC;
        // Strip the `forbidden_tokens` definition from the haystack
        // so the constructor's fragments do not trip the check on
        // themselves. The split sentinel is the function's name; if
        // a future refactor renames it the test will start matching
        // its own body — a desirable failure mode (the assert message
        // says exactly what to do).
        let cut = src.find("fn forbidden_tokens").unwrap_or(src.len());
        let haystack = &src[..cut];
        for tok in forbidden_tokens() {
            assert!(
                !haystack.contains(&tok),
                "supervisor module contains forbidden token `{tok}`; \
                 the supervisor's only outbound voice is post_chat_message \
                 with transport='supervisor' and the module must never \
                 import the stdlib or tokio process modules (see module doc-comment)",
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rpc::InProcessRpc;
    use crate::time::now_ms;
    use codeless_rpc::{AddRepoArgs, RpcServer, SubmitJobArgs};
    use codeless_types::{GitAuth, StopReason};
    use std::time::Duration;

    async fn fresh_rpc_with_job() -> (Arc<InProcessRpc>, JobId) {
        let rpc = InProcessRpc::new().await.unwrap();
        let repo = rpc
            .add_repo(AddRepoArgs {
                name: "r".into(),
                clone_url: "u".into(),
                default_branch: "main".into(),
                local_path: "/tmp".into(),
                git_auth: GitAuth::Token {
                    env_var: "X".into(),
                },
                concurrency_cap: None,
                default_runner: None,
            })
            .await
            .unwrap();
        let job = rpc
            .submit_job(SubmitJobArgs {
                repo_id: repo.id,
                prompt: Some("p".into()),
                template_yaml: None,
                runner: "mock".into(),
                branch: "b".into(),
                workspace_mode: None,
                cost_cap_cents: 0,
                wall_clock_cap_ms: 0,
                model: None,
                permission_mode: None,
                effort: None,
                system_prompt: None,
                persona_id: None,
                auto_bypass_policy: None,
                start_immediately: false,
            })
            .await
            .unwrap();
        (Arc::new(rpc), job.id)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn exits_on_job_completed() {
        let (rpc, job_id) = fresh_rpc_with_job().await;
        let handle = spawn_supervisor(rpc.bus().clone(), job_id);
        tokio::time::sleep(Duration::from_millis(20)).await;
        rpc.bus()
            .publish(
                Some(job_id),
                None,
                None,
                Event::JobCompleted { job_id },
                now_ms(),
            )
            .await
            .unwrap();
        let res = tokio::time::timeout(Duration::from_secs(2), handle).await;
        assert!(res.is_ok(), "supervisor must exit after JobCompleted");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn exits_on_job_failed() {
        let (rpc, job_id) = fresh_rpc_with_job().await;
        let handle = spawn_supervisor(rpc.bus().clone(), job_id);
        tokio::time::sleep(Duration::from_millis(20)).await;
        rpc.bus()
            .publish(
                Some(job_id),
                None,
                None,
                Event::JobFailed { job_id },
                now_ms(),
            )
            .await
            .unwrap();
        let res = tokio::time::timeout(Duration::from_secs(2), handle).await;
        assert!(res.is_ok(), "supervisor must exit after JobFailed");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn exits_on_job_stopped() {
        let (rpc, job_id) = fresh_rpc_with_job().await;
        let handle = spawn_supervisor(rpc.bus().clone(), job_id);
        tokio::time::sleep(Duration::from_millis(20)).await;
        rpc.bus()
            .publish(
                Some(job_id),
                None,
                None,
                Event::JobStopped {
                    job_id,
                    reason: StopReason::User,
                },
                now_ms(),
            )
            .await
            .unwrap();
        let res = tokio::time::timeout(Duration::from_secs(2), handle).await;
        assert!(res.is_ok(), "supervisor must exit after JobStopped");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn stays_alive_through_non_terminal_events() {
        let (rpc, job_id) = fresh_rpc_with_job().await;
        let handle = spawn_supervisor(rpc.bus().clone(), job_id);
        tokio::time::sleep(Duration::from_millis(20)).await;
        rpc.bus()
            .publish(
                Some(job_id),
                None,
                None,
                Event::JobStarted { job_id },
                now_ms(),
            )
            .await
            .unwrap();
        rpc.bus()
            .publish(
                Some(job_id),
                None,
                None,
                Event::JobPaused {
                    job_id,
                    reason: StopReason::User,
                },
                now_ms(),
            )
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(80)).await;
        assert!(
            !handle.is_finished(),
            "supervisor must not exit on JobStarted or JobPaused",
        );
        handle.abort();
        let _ = handle.await;
    }
}
