//! Runtime hook for scoped pause points. The template-runner calls
//! into this module at four well-defined transition points (before a
//! stage starts, after a stage's closing trio resolves, before a todo
//! handler runs, after a todo reaches a terminal status); the hook
//! consults the `scheduled_pause_points` rows persisted in stage 5,
//! finds the first matching point, and pauses the job through the
//! same `pause_job` primitive the user-driven RPC uses.
//!
//! Why a separate module rather than inline checks in the runner:
//! the template-runner already pushes 3000+ lines, and the hook needs
//! to share its match logic with the SQL-backed schedule loader so the
//! per-transition call site stays a single async function call. The
//! match table lives here; the four call sites in `template_runner`
//! map their transition shape onto `TransitionPoint` and let this
//! module decide whether anything fires.
//!
//! What this module is NOT: a new pause primitive. It writes the same
//! `Paused` row and emits the same `JobPaused` event that
//! `rpc::jobs::pause_job` does, just with `StopReason::ScopedPausePoint
//! { point_id }` instead of `StopReason::User`. The job stays
//! resumable through the existing `resume_job` RPC.

use codeless_types::pause_point::{PausePoint, PausePointPosition, PausePointTarget, TodoSelector};
use codeless_types::{Event, JobId, JobStatus, StageId, StopReason, TodoKind};
use tokio_util::sync::CancellationToken;

use crate::event_bus::EventBus;
use crate::state_machine::transition_job;
use crate::store::SqliteStore;
use crate::time::now_ms;

/// One of the four hook points exposed by the template-runner. The
/// `stage_ordinal` field on every variant is **1-based** — it matches
/// the YAML position the operator wrote in `pause_points:` and the
/// `PausePointTarget::Stage { ordinal }` value the parser produced. The
/// stage-row `ordinal` column the recorder writes is 0-based; the
/// template-runner converts at the call site.
#[derive(Debug, Clone)]
pub enum TransitionPoint {
    /// Stage `stage_ordinal` has been selected by the runner but no
    /// todo for it has started yet (StageStarted not yet published).
    BeforeStage { stage_ordinal: u32 },
    /// Stage `stage_ordinal` finished its closing trio — the runner
    /// is about to advance to `stage_ordinal + 1`. Fires before the
    /// runner consults the next iteration's selection.
    AfterStage { stage_ordinal: u32 },
    /// A todo handler for `stage_ordinal` is about to run. The hook
    /// fires once per todo per stage; the runner is expected to call
    /// this immediately before flipping the todo to `InProgress`.
    BeforeTodo {
        stage_ordinal: u32,
        todo_ordinal: u32,
        kind: TodoKind,
        title: String,
    },
    /// A todo for `stage_ordinal` has transitioned to a terminal
    /// status (`Done` or `Skipped`). The hook fires after the
    /// `TodoCompleted` event is published but before the runner
    /// selects the next todo or closes the stage.
    AfterTodo {
        stage_ordinal: u32,
        todo_ordinal: u32,
        kind: TodoKind,
        title: String,
    },
}

/// Match `point` against `transition`. Returns `true` when the
/// transition is the one the scheduled point wants to halt before /
/// after. The matcher is total per-point: every `PausePointTarget` x
/// `PausePointPosition` x `TransitionPoint` combination either matches
/// or doesn't — there is no "deferred until next tick" state to track
/// inside this function (the schedule row stays where it is; the
/// runtime simply doesn't trigger until the right transition arrives).
pub fn matches(point: &PausePoint, transition: &TransitionPoint) -> bool {
    match (&point.target, point.position, transition) {
        // Stage-level Before — the runner has selected stage N and
        // is about to publish StageStarted. The point ordinal is the
        // resolved 1-based stage index.
        (
            PausePointTarget::Stage { ordinal },
            PausePointPosition::Before,
            TransitionPoint::BeforeStage { stage_ordinal },
        ) => ordinal == stage_ordinal,
        // Stage-level After — stage N's closing trio resolved and
        // the runner is about to move to stage N+1.
        (
            PausePointTarget::Stage { ordinal },
            PausePointPosition::After,
            TransitionPoint::AfterStage { stage_ordinal },
        ) => ordinal == stage_ordinal,
        // Todo-level Before / After narrow on `stage_ordinal` first,
        // then on the selector. The trio selector matches by kind,
        // the ordinal selector matches by todo ordinal, the
        // title-substring selector matches by case-insensitive
        // `contains` on the todo title (a non-trio todo by spec —
        // §1.2.3 of SCOPED-PAUSE-POINTS.md).
        (
            PausePointTarget::StageTodo {
                stage_ordinal: target_stage,
                selector,
            },
            PausePointPosition::Before,
            TransitionPoint::BeforeTodo {
                stage_ordinal,
                todo_ordinal,
                kind,
                title,
            },
        )
        | (
            PausePointTarget::StageTodo {
                stage_ordinal: target_stage,
                selector,
            },
            PausePointPosition::After,
            TransitionPoint::AfterTodo {
                stage_ordinal,
                todo_ordinal,
                kind,
                title,
            },
        ) if matches_position(point.position, transition) => {
            target_stage == stage_ordinal && selector_matches(selector, *todo_ordinal, *kind, title)
        }
        _ => false,
    }
}

/// `matches` collapses `BeforeTodo` and `AfterTodo` into one guard arm
/// to share the selector logic; this helper preserves the position
/// discriminator the outer arm folded away.
fn matches_position(p: PausePointPosition, t: &TransitionPoint) -> bool {
    matches!(
        (p, t),
        (
            PausePointPosition::Before,
            TransitionPoint::BeforeTodo { .. }
        ) | (PausePointPosition::After, TransitionPoint::AfterTodo { .. })
    )
}

fn selector_matches(sel: &TodoSelector, ordinal: u32, kind: TodoKind, title: &str) -> bool {
    match sel {
        // The trio kinds are reserved words — `todo: docs` is always
        // the runtime-injected `Docs` row, never a runner-authored
        // todo whose title happens to be "docs". The matcher only
        // fires on a row whose `TodoKind` is non-runner.
        TodoSelector::Trio { kind: target } => kind == *target && kind != TodoKind::Runner,
        // Ordinal selector: out-of-range upper bound was deferred to
        // runtime by the parser (runner-authored todos grow), so a
        // mismatch here is the legitimate non-fire path.
        TodoSelector::Ordinal { ordinal: target } => *target == ordinal,
        // Title-substring is case-insensitive `contains`. A trio row
        // is excluded by spec (trio targeting goes through `Trio`).
        // Ambiguity (multiple matches in the same stage tick) is
        // detected in the caller via `find_match` returning the
        // first match — this stage's hook fires once per transition,
        // so a second match on the same stage will trip the next
        // transition's check instead of needing a runtime
        // `AmbiguousTitleSubstring` reject path in this function.
        TodoSelector::TitleSubstring { pattern } => {
            if kind != TodoKind::Runner {
                return false;
            }
            title
                .to_lowercase()
                .contains(pattern.to_lowercase().as_str())
        }
    }
}

/// Outcome from `check_and_pause`. The runner reads it to decide
/// whether to bail out of the current iteration. `cancel.cancel()` is
/// already fired on `Paused`, so a runner that ignores the return
/// value and loops will still observe the cancellation on its next
/// `cancel.is_cancelled()` check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookOutcome {
    /// No scheduled point matched this transition; the runner should
    /// continue advancing.
    Continue,
    /// A scheduled point matched and the job has been moved to
    /// `Paused`. The runner must not advance past this point until
    /// `resume_job` re-queues the row.
    Paused,
}

/// Consult the schedule for `job_id` and pause the job if a row
/// matches `transition`. The first matching row wins (YAML order via
/// `ORDER BY ordinal ASC` on the SQL side); a transition that matches
/// nothing is a cheap no-op. The store error path logs and returns
/// `Continue` rather than propagating — a transient SQLite hiccup must
/// not crash the runner, and the schedule re-loads from the same
/// query on the next transition anyway.
pub async fn check_and_pause(
    store: &SqliteStore,
    bus: &EventBus,
    job_id: JobId,
    transition: &TransitionPoint,
    cancel: &CancellationToken,
) -> HookOutcome {
    let schedule = match store.list_scheduled_pause_points(job_id).await {
        Ok(rows) => rows,
        Err(err) => {
            tracing::warn!(
                error = %err,
                %job_id,
                ?transition,
                "scoped_pause_hook: list_scheduled_pause_points failed; \
                 continuing without pause check"
            );
            return HookOutcome::Continue;
        }
    };
    let Some(point) = schedule.iter().find(|pp| matches(pp, transition)) else {
        return HookOutcome::Continue;
    };
    match fire_pause(store, bus, job_id, point, cancel).await {
        Ok(true) => HookOutcome::Paused,
        Ok(false) => HookOutcome::Continue,
        Err(err) => {
            tracing::warn!(
                error = %err,
                %job_id,
                point_id = %point.id,
                "scoped_pause_hook: fire_pause failed; runner continues but \
                 scheduled point will retry on the next transition"
            );
            HookOutcome::Continue
        }
    }
}

/// Write `Paused` + `stop_reason = ScopedPausePoint { point_id }` on
/// the job row, publish `JobPaused`, and fire `cancel.cancel()` so the
/// runner exits at its next check. Mirrors `rpc::jobs::pause_job` so
/// the lifecycle the surface produces is indistinguishable from a
/// user-clicked pause — the only difference is the `StopReason`
/// discriminator, which lets dashboards render a planned-pause
/// divider in stage 8 without re-deriving "is this scheduled" from
/// the schedule table.
/// Returns `Ok(true)` when the row was actually moved to `Paused` and
/// the event emitted; `Ok(false)` when the row was already terminal
/// or paused and the call was a no-op. The error path is reserved for
/// SQLite / bus failures.
async fn fire_pause(
    store: &SqliteStore,
    bus: &EventBus,
    job_id: JobId,
    point: &PausePoint,
    cancel: &CancellationToken,
) -> sqlx::Result<bool> {
    let Some(mut job) = store.get_job(job_id).await? else {
        // The job was deleted between the schedule load and the
        // fire; nothing to pause.
        return Ok(false);
    };
    // Only Running and AwaitingReview map to Paused (`transition_job`
    // is the authority). The runner's own pre-checks already mean we
    // get here from Running in practice; the explicit guard exists so
    // a stray call from a different surface can't corrupt the row.
    if !matches!(job.status, JobStatus::Running | JobStatus::AwaitingReview) {
        return Ok(false);
    }
    transition_job(job.status, JobStatus::Paused)
        .map_err(|e| sqlx::Error::Decode(format!("scoped pause transition: {e}").into()))?;
    let now = now_ms();
    let reason = StopReason::ScopedPausePoint { point_id: point.id };
    job.status = JobStatus::Paused;
    job.stop_reason = Some(reason);
    job.ended_at = Some(now);
    store.update_job(&job).await?;
    bus.publish(
        Some(job_id),
        None,
        None,
        Event::JobPaused { job_id, reason },
        now,
    )
    .await
    .map_err(|e| sqlx::Error::Decode(format!("scoped pause publish: {e}").into()))?;
    cancel.cancel();
    Ok(true)
}

/// Trio-kind convenience entry point. The trio_emitter call sites
/// (`verify_runner` for `Checks`, the claude `Docs` writer, the
/// `commit_stage_changes` `Git` step) know the `StageId` and the
/// `TodoKind` they're about to flip; this helper resolves the stage's
/// 1-based ordinal off the row and forwards to `check_and_pause`. The
/// trio rows themselves carry top-of-`u32` ordinals (see
/// `publish_trio`), so the substring/ordinal selectors don't fire on
/// a trio call here — only the `Trio` selector does, which is the
/// only one that targets these kinds anyway per §1.2.3.
pub async fn check_trio(
    store: &SqliteStore,
    bus: &EventBus,
    job_id: JobId,
    stage_id: StageId,
    kind: TodoKind,
    position: PausePointPosition,
    cancel: &CancellationToken,
) -> HookOutcome {
    let stage_ordinal = match store.get_stage(stage_id).await {
        Ok(Some(stage)) => stage.ordinal + 1,
        _ => return HookOutcome::Continue,
    };
    // Trio rows are runtime-injected with synthetic ordinals; the
    // selector matchers only care about `stage_ordinal` and `kind`
    // for the `Trio` selector. A placeholder ordinal/title keeps the
    // shared `matches` machinery in one place without inventing a
    // separate trio-only matcher.
    let transition = match position {
        PausePointPosition::Before => TransitionPoint::BeforeTodo {
            stage_ordinal,
            todo_ordinal: 0,
            kind,
            title: String::new(),
        },
        PausePointPosition::After => TransitionPoint::AfterTodo {
            stage_ordinal,
            todo_ordinal: 0,
            kind,
            title: String::new(),
        },
    };
    check_and_pause(store, bus, job_id, &transition, cancel).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use codeless_types::pause_point::{
        PausePoint, PausePointId, PausePointPosition, PausePointTarget, TodoSelector,
    };
    use codeless_types::TodoKind;

    fn pp_stage(ordinal: u32, position: PausePointPosition) -> PausePoint {
        PausePoint {
            id: PausePointId::new(),
            target: PausePointTarget::Stage { ordinal },
            position,
            reason: None,
        }
    }

    fn pp_trio(stage: u32, kind: TodoKind, position: PausePointPosition) -> PausePoint {
        PausePoint {
            id: PausePointId::new(),
            target: PausePointTarget::StageTodo {
                stage_ordinal: stage,
                selector: TodoSelector::Trio { kind },
            },
            position,
            reason: None,
        }
    }

    fn pp_substring(stage: u32, pattern: &str, position: PausePointPosition) -> PausePoint {
        PausePoint {
            id: PausePointId::new(),
            target: PausePointTarget::StageTodo {
                stage_ordinal: stage,
                selector: TodoSelector::TitleSubstring {
                    pattern: pattern.into(),
                },
            },
            position,
            reason: None,
        }
    }

    #[test]
    fn before_stage_matches_only_its_ordinal_and_position() {
        let p = pp_stage(3, PausePointPosition::Before);
        assert!(matches(
            &p,
            &TransitionPoint::BeforeStage { stage_ordinal: 3 }
        ));
        assert!(!matches(
            &p,
            &TransitionPoint::BeforeStage { stage_ordinal: 4 }
        ));
        assert!(!matches(
            &p,
            &TransitionPoint::AfterStage { stage_ordinal: 3 }
        ));
    }

    #[test]
    fn after_stage_matches_only_its_ordinal_and_position() {
        let p = pp_stage(2, PausePointPosition::After);
        assert!(matches(
            &p,
            &TransitionPoint::AfterStage { stage_ordinal: 2 }
        ));
        assert!(!matches(
            &p,
            &TransitionPoint::BeforeStage { stage_ordinal: 2 }
        ));
    }

    #[test]
    fn trio_selector_matches_only_its_kind_and_narrows_on_stage() {
        let p = pp_trio(3, TodoKind::Docs, PausePointPosition::After);
        assert!(matches(
            &p,
            &TransitionPoint::AfterTodo {
                stage_ordinal: 3,
                todo_ordinal: 11,
                kind: TodoKind::Docs,
                title: "writes handover".into(),
            }
        ));
        // wrong stage
        assert!(!matches(
            &p,
            &TransitionPoint::AfterTodo {
                stage_ordinal: 2,
                todo_ordinal: 11,
                kind: TodoKind::Docs,
                title: "writes handover".into(),
            }
        ));
        // wrong kind
        assert!(!matches(
            &p,
            &TransitionPoint::AfterTodo {
                stage_ordinal: 3,
                todo_ordinal: 11,
                kind: TodoKind::Git,
                title: "writes handover".into(),
            }
        ));
        // wrong position
        assert!(!matches(
            &p,
            &TransitionPoint::BeforeTodo {
                stage_ordinal: 3,
                todo_ordinal: 11,
                kind: TodoKind::Docs,
                title: "writes handover".into(),
            }
        ));
    }

    #[test]
    fn trio_selector_refuses_runner_kind() {
        // SCOPED-PAUSE-POINTS.md §1.2.3: a runner-authored todo whose
        // title is "docs" must not match `todo: docs`. The kind guard
        // is what enforces that.
        let p = pp_trio(3, TodoKind::Docs, PausePointPosition::Before);
        assert!(!matches(
            &p,
            &TransitionPoint::BeforeTodo {
                stage_ordinal: 3,
                todo_ordinal: 1,
                kind: TodoKind::Runner,
                title: "docs".into(),
            }
        ));
    }

    #[test]
    fn substring_selector_matches_runner_only_and_is_case_insensitive() {
        let p = pp_substring(5, "Migrate", PausePointPosition::Before);
        assert!(matches(
            &p,
            &TransitionPoint::BeforeTodo {
                stage_ordinal: 5,
                todo_ordinal: 2,
                kind: TodoKind::Runner,
                title: "Run schema migrate".into(),
            }
        ));
        // trio rows never match a substring selector (§1.2.3)
        assert!(!matches(
            &p,
            &TransitionPoint::BeforeTodo {
                stage_ordinal: 5,
                todo_ordinal: 2,
                kind: TodoKind::Docs,
                title: "migrate".into(),
            }
        ));
        // wrong stage
        assert!(!matches(
            &p,
            &TransitionPoint::BeforeTodo {
                stage_ordinal: 4,
                todo_ordinal: 2,
                kind: TodoKind::Runner,
                title: "migrate".into(),
            }
        ));
    }

    #[test]
    fn ordinal_selector_matches_exact_todo_ordinal() {
        let p = PausePoint {
            id: PausePointId::new(),
            target: PausePointTarget::StageTodo {
                stage_ordinal: 2,
                selector: TodoSelector::Ordinal { ordinal: 4 },
            },
            position: PausePointPosition::After,
            reason: None,
        };
        assert!(matches(
            &p,
            &TransitionPoint::AfterTodo {
                stage_ordinal: 2,
                todo_ordinal: 4,
                kind: TodoKind::Runner,
                title: "anything".into(),
            }
        ));
        assert!(!matches(
            &p,
            &TransitionPoint::AfterTodo {
                stage_ordinal: 2,
                todo_ordinal: 3,
                kind: TodoKind::Runner,
                title: "anything".into(),
            }
        ));
    }
}
