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

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use codeless_types::{ChatTransport, Event, JobId, JobStatus};
use futures_util::stream::FuturesUnordered;
use futures_util::StreamExt;
use tokio::task::JoinHandle;

use crate::event_bus::{EventBus, SubscribeFilter};
use crate::store::supervisor_goals::{GoalCondition, SupervisorGoal};
use crate::store::{MarkOutcome, SqliteStore};
use crate::time::now_ms;

#[cfg(feature = "supervisor-claude")]
pub mod claude;
pub mod prompt;
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

/// Per-goal arm carried by the supervisor's timer set. The future
/// resolves to the goal it represents — `SupervisorGoal` is `Clone`
/// in spirit (every field is owned by value), so the arm can move the
/// goal into the future and hand it back at fire time without a
/// secondary lookup.
type GoalArm = Pin<Box<dyn Future<Output = SupervisorGoal> + Send>>;

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

    // Rehydrate pre-armed goals from the store. The persistence layer
    // is the source of truth across process restarts (JOB-CHAT.md §C3
    // — "Persisting the goal is what makes it survive a process
    // restart"); on boot we scan every `armed` row for this Run and
    // either re-arm the matching timer / event watcher or mark the
    // row `superseded` with a chat-thread explanation when the
    // condition can no longer trip. The scan is what makes the
    // load-bearing "if it runs >1h, stop it" example survive a server
    // restart: without it, a supervisor that boots after a crash
    // forgets every authorisation the user already gave.
    //
    // v0.1 re-arms `deadline-stop` only — threshold / event-notify
    // arming lands alongside their respective signal sources in a
    // later stage. A `threshold-stop` / `event-notify` row found here
    // stays `armed` and is simply not watched yet; it does not get
    // superseded, because a future supervisor version will be able to
    // honour it once the wiring lands.
    //
    // The "no longer makes sense" predicate is "the Run row is already
    // terminal" — once the Run reaches `Completed` / `Failed` /
    // `Stopped`, every `stop_job` / `post_chat_message` action against
    // it is unreachable, and a `deadline-stop` condition cannot trip
    // either (the supervisor is about to exit on the terminal envelope
    // it is going to see in the select loop below). Walking the rows
    // to `superseded` here keeps the audit trail honest — a chat reader
    // who scrolls back sees an explicit "I am dropping this goal
    // because the Run already ended" rather than a goal that silently
    // hangs forever in `armed`.
    let timers = FuturesUnordered::<GoalArm>::new();
    let mut timers = timers;
    let job_terminal = match tools.store_arc().get_job(job_id).await {
        Ok(Some(j)) => matches!(
            j.status,
            JobStatus::Completed | JobStatus::Failed | JobStatus::Stopped
        ),
        Ok(None) => {
            // No such Job — the supervisor cannot do anything useful.
            // Bail before subscribing to a stream that will never carry
            // a relevant envelope.
            tracing::debug!(%job_id, "supervisor boot: job row missing; exiting");
            return;
        }
        Err(e) => {
            tracing::debug!(%job_id, error = %e, "supervisor boot: get_job failed; continuing");
            false
        }
    };
    match tools.store_arc().list_armed_for_run(job_id).await {
        Ok(armed) => {
            tracing::debug!(%job_id, count = armed.len(), "supervisor rehydrated armed goals");
            let now = now_ms().0;
            for goal in armed {
                if job_terminal {
                    supersede_goal(
                        &tools,
                        job_id,
                        &goal,
                        "run already terminal at supervisor boot",
                    )
                    .await;
                    continue;
                }
                if let Some(arm) = arm_goal_timer(&goal, now) {
                    tracing::debug!(%job_id, goal_id = %goal.id, "armed goal timer");
                    timers.push(arm);
                } else {
                    tracing::debug!(
                        %job_id,
                        goal_id = %goal.id,
                        "goal kind not armable in v0.1; leaving armed for a future supervisor"
                    );
                }
            }
        }
        Err(e) => {
            tracing::debug!(%job_id, error = %e, "supervisor goal rehydrate failed; continuing without timers");
        }
    }

    loop {
        tokio::select! {
            // Per-goal timer fired. The select! arm uses the
            // `is_empty()` guard so the loop does not hot-spin when no
            // goals are armed — an empty `FuturesUnordered` resolves
            // to `None` immediately, which would otherwise busy-loop
            // the select.
            Some(goal) = timers.next(), if !timers.is_empty() => {
                fire_goal(&tools, job_id, goal).await;
            }
            item = stream.next() => {
                let Some(item) = item else { return; };
                let env = match item {
                    Ok(env) => env,
                    Err(_e) => {
                        tracing::debug!(%job_id, "supervisor stream error; exiting");
                        return;
                    }
                };
                match env.event {
                    Event::JobCompleted { .. } => {
                        post_terminal_summary(&tools, job_id, JobStatus::Completed).await;
                        tracing::debug!(%job_id, "supervisor observed run terminal; exiting");
                        return;
                    }
                    Event::JobFailed { .. } => {
                        post_terminal_summary(&tools, job_id, JobStatus::Failed).await;
                        tracing::debug!(%job_id, "supervisor observed run terminal; exiting");
                        return;
                    }
                    Event::JobStopped { .. } => {
                        post_terminal_summary(&tools, job_id, JobStatus::Stopped).await;
                        tracing::debug!(%job_id, "supervisor observed run terminal; exiting");
                        return;
                    }
                    Event::ChatMessageAppended { ref message, .. } => {
                        // Echo-suppression on the supervisor's own
                        // messages: the reactor would otherwise loop
                        // forever, replying to its own reply. Every
                        // non-supervisor transport is a candidate for
                        // a reply.
                        if matches!(message.transport, ChatTransport::Supervisor) {
                            continue;
                        }
                        react_to_chat(&tools, job_id, &message.body).await;
                    }
                    _ => {}
                }
            }
        }
    }
}

/// Build the per-goal timer arm. Deadlines are persisted as absolute
/// wall-clock milliseconds (so a restart re-anchors against the same
/// real instant, not against boot time), but `tokio::time::sleep`
/// runs on the tokio time-driver — which is what the e2e suite
/// pauses + advances. Computing the sleep duration as `deadline -
/// now_ms` at arm time keeps both clocks consistent: the production
/// path waits on real wall time, the test path drives mock tokio
/// time and observes the same arm fire.
///
/// A past deadline yields a zero-duration sleep so the reactor
/// catches a missed deadline (e.g. process restart after the
/// deadline) on the next select-tick rather than waiting forever for
/// a deadline that is already behind us.
fn arm_goal_timer(goal: &SupervisorGoal, now_ms_val: i64) -> Option<GoalArm> {
    match goal.condition {
        GoalCondition::DeadlineStop { deadline_ms } => {
            let delta_ms = deadline_ms.saturating_sub(now_ms_val).max(0) as u64;
            let dur = Duration::from_millis(delta_ms);
            let goal = goal.clone();
            Some(Box::pin(async move {
                tokio::time::sleep(dur).await;
                goal
            }))
        }
        // Threshold / event-notify goals are out of scope for the
        // deadline-stop loop — they need a different signal source
        // (metric sampling for threshold, an `event_kind` predicate
        // for event-notify). They land in their own stage so a fresh
        // failure on one of them does not break the deadline path.
        GoalCondition::ThresholdStop { .. } | GoalCondition::EventNotify { .. } => None,
    }
}

/// Walk one armed goal to `superseded` and record the reason in the
/// chat thread. JOB-CHAT.md §C3 hands the supervisor a single voice
/// (`post_chat_message`), so the "with the reason" half of the rehydration
/// contract lives in the chat thread rather than a new SQL column —
/// a reader scrolling the per-Job thread sees the supersede note paired
/// with the original "if X then Y" authorisation by the `replies_to`
/// metadata edge, mirroring the same shape `fire_pre_armed_goal` uses
/// for the successful-fire path.
async fn supersede_goal(tools: &Tools, job_id: JobId, goal: &SupervisorGoal, reason: &str) {
    match tools.store_arc().mark_superseded(goal.id).await {
        Ok(MarkOutcome::Transitioned) => {}
        Ok(MarkOutcome::NoChange) => {
            // Concurrent transition (a user cancel, a fire from another
            // process) already moved the row out of `armed`. The audit
            // trail is fine without our note.
            return;
        }
        Err(_e) => {
            tracing::debug!(%job_id, goal_id = %goal.id, "supersede mark_superseded failed; skipping audit note");
            return;
        }
    }
    let body = format!(
        "Superseding the goal you armed earlier (goal {}): {reason}.",
        goal.id,
    );
    let _ = tools.post_chat_message(job_id, body).await;
}

/// Single goal-fire path. JOB-CHAT.md Hard rule 4 (second regime)
/// pins "no preview, no nag" for pre-armed actions — the user already
/// authorised the if-X-then-Y, so the action invokes immediately and
/// the audit trail is the post-action summary that references the
/// authorising `chat_messages.id`. The `mark_fired` guard is what
/// makes the race against a concurrent user cancellation safe:
/// whichever transition lands first wins, the loser sees `NoChange`
/// and skips the action.
async fn fire_goal(tools: &Tools, job_id: JobId, goal: SupervisorGoal) {
    match tools.mark_goal_fired(goal.id).await {
        Ok(MarkOutcome::Transitioned) => {}
        Ok(MarkOutcome::NoChange) => {
            tracing::debug!(%job_id, goal_id = %goal.id, "goal already terminal at fire-time; skipping");
            return;
        }
        Err(_e) => {
            tracing::debug!(%job_id, goal_id = %goal.id, "goal mark_fired failed; skipping fire");
            return;
        }
    }
    if let Err(_e) = tools.fire_pre_armed_goal(&goal).await {
        tracing::debug!(%job_id, goal_id = %goal.id, "goal action invocation failed");
    }
}

/// Compose and post the one-paragraph end-of-Run summary the user
/// reads in the chat thread. Pulls the stage list off the store so
/// the message cites real stage names and the `failure_detail` column
/// the recorder stamped on a `Failed` row. The text format lives in
/// `prompt::format_terminal_summary` so a doc reviewer can read it
/// without compiling.
async fn post_terminal_summary(tools: &Tools, job_id: JobId, status: JobStatus) {
    let store = tools.store_arc();
    let stages = match store.list_stages_for_job(job_id).await {
        Ok(s) => s,
        Err(_e) => {
            // Best-effort: a stale store handle is not something the
            // supervisor can recover from at end-of-Run; the chat
            // thread will simply lack a summary for this Run.
            tracing::debug!(%job_id, "supervisor could not list stages for terminal summary");
            return;
        }
    };
    let infos: Vec<prompt::TerminalStageInfo<'_>> = stages
        .iter()
        .map(|s| prompt::TerminalStageInfo {
            ordinal: s.stage.ordinal,
            name: s.stage.name.as_str(),
            status: s.stage.status,
            failure_detail: s.stage.failure_detail.as_deref(),
        })
        .collect();
    let body = prompt::format_terminal_summary(status, &infos);
    let _ = tools.post_chat_message(job_id, body).await;
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
