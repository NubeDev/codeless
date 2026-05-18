//! Per-Run supervisor task (JOB-CHAT.md (C2) scaffold).
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
//! `std::process` / `tokio::process`; any action it gains in a future
//! stage routes through the existing RPCs on `InProcessRpc`. The
//! crate-layout grep in `crates/codeless-adapters-host` already
//! enforces process-spawn confinement; this module is mentioned here
//! so a future reader does not propose lifting it into its own crate.
//!
//! Voice contract (the load-bearing C2 invariant)
//! ----------------------------------------------
//! The supervisor's only outbound channel is `post_job_message` with
//! `transport='supervisor'`. Concretely, this module is forbidden
//! from:
//!
//! - the stdlib print macros (user-visible stdout/stderr noise),
//! - the loud tracing macros at info / warn / error level (those
//!   surface to operator dashboards; supervisor reasoning belongs in
//!   the chat thread, not in the log stream — the debug-level macro
//!   stays permitted for engineer-only diagnostics),
//! - calling `publish` on the event bus directly (every supervisor
//!   utterance has to go through `post_job_message` so the per-Job
//!   chat log is the single source of truth and the asymmetric
//!   echo-suppression rule in `codeless-bot-core` still applies).
//!
//! The `lint` module below contains a `cargo test`-time grep that
//! fails the build if any of those substrings re-appear in this
//! module. Keep that check honest — adding the forbidden token in a
//! comment defeats the linter the same way it would defeat the
//! intent.

use std::sync::Arc;

use codeless_types::{Event, JobId};
use futures_util::StreamExt;
use tokio::task::JoinHandle;

use crate::event_bus::{EventBus, SubscribeFilter};

/// Spawn the per-Run supervisor task. Returns a `JoinHandle` so the
/// caller (`drive_job` today, the hosted server's job-driver loop in
/// future stages) can `.abort()` it on shutdown; the task otherwise
/// self-terminates when it sees a Run terminal event on the bus.
///
/// The bus subscription is opened **inside** the spawned task rather
/// than synchronously here. Doing it inside means the `JobStarted`
/// event the driver publishes immediately *before* this call is
/// guaranteed to be in the persisted events table by the time the
/// supervisor's subscription replays — there is no live-vs-replay
/// race between the driver's `JobStarted` publication and this
/// subscription. The trade-off is that subscription failures land in
/// `tracing::debug!` instead of being returned to the caller; the
/// supervisor is best-effort and the driver never blocks on it.
pub fn spawn_supervisor(bus: Arc<EventBus>, job_id: JobId) -> JoinHandle<()> {
    tokio::spawn(run(bus, job_id))
}

async fn run(bus: Arc<EventBus>, job_id: JobId) {
    let mut stream = match bus
        .subscribe_since(SubscribeFilter::Job(job_id), None)
        .await
    {
        Ok(s) => s,
        Err(e) => {
            // Subscription failure means the bus is wedged; nothing
            // the supervisor can do. `debug!` keeps the diagnostic
            // out of operator dashboards per the voice contract.
            tracing::debug!(%job_id, error = %e, "supervisor subscribe failed; exiting");
            return;
        }
    };
    tracing::debug!(%job_id, "supervisor started");
    while let Some(item) = stream.next().await {
        let env = match item {
            Ok(env) => env,
            Err(_e) => {
                // Event lag on the broadcast channel. The next
                // subscribe + replay would catch us back up, but the
                // supervisor scaffold has no per-Job state worth
                // resuming yet, so bail and let `drive_job` spawn a
                // fresh task on the next Run.
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
                // C2 scaffold: no reply logic yet. The next stage
                // wires the assistant runner here. Acknowledging the
                // append on `debug!` proves the subscription is
                // delivering chat traffic at the right grain.
                tracing::debug!(%job_id, "supervisor saw ChatMessageAppended");
            }
            _ => {}
        }
    }
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
            // bus.publish (the only legitimate write-out is via
            // post_job_message, which the lint pins by exclusion —
            // a future contributor adding `rpc.post_job_message`
            // is fine, adding `bus.publish` is not).
            ["bus.", "publish"].concat(),
            // user-surface tracing levels. tracing::debug! stays
            // permitted; the three loud levels do not.
            ["traci", "ng::info!"].concat(),
            ["traci", "ng::warn!"].concat(),
            ["traci", "ng::error!"].concat(),
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
                 the supervisor's only outbound voice is post_job_message \
                 with transport='supervisor' (see module doc-comment)",
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
        // Yield so the subscribe runs before we publish.
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
        // Give the loop time to drain the two non-terminal events
        // and not exit.
        tokio::time::sleep(Duration::from_millis(80)).await;
        assert!(
            !handle.is_finished(),
            "supervisor must not exit on JobStarted or JobPaused",
        );
        handle.abort();
        let _ = handle.await;
    }
}
