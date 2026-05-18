use codeless_types::pause_point::{PausePoint, PausePointId, PausePointPosition, PausePointTarget};
use codeless_types::{JobId, UnixMillis};
use sqlx::Row;

use super::codec::{parse_id, serde_err};
use super::SqliteStore;

/// Wire label for the `position` column. Kebab-case matches the
/// `PausePointPosition` serde rename and the SCOPED-PAUSE-POINTS doc;
/// the explicit pattern match (rather than `Display`-via-serde) keeps
/// the wire spelling in one place.
fn position_label(p: PausePointPosition) -> &'static str {
    match p {
        PausePointPosition::Before => "before",
        PausePointPosition::After => "after",
    }
}

fn parse_position(s: &str) -> sqlx::Result<PausePointPosition> {
    match s {
        "before" => Ok(PausePointPosition::Before),
        "after" => Ok(PausePointPosition::After),
        other => Err(sqlx::Error::Decode(
            format!("unknown pause-point position: {other}").into(),
        )),
    }
}

impl SqliteStore {
    /// Replace the scheduled pause points for `job_id` with `points`,
    /// preserving the YAML order via the 1-based `(job_id, ordinal)`
    /// composite key. Idempotent: calling twice with the same input
    /// converges on the same row set, and an empty `points` slice
    /// drops every row for the job.
    ///
    /// The resync path (`resync_template_from_disk`) calls this after
    /// `JobTemplate::resolve_pause_points` succeeds — the parser is
    /// the source of truth for what the schedule means, the store is
    /// the source of truth for what's persisted. Wrapped in a
    /// transaction so a partial write can't leave a half-rebuilt
    /// schedule visible to the runtime hook.
    pub async fn replace_scheduled_pause_points(
        &self,
        job_id: JobId,
        points: &[PausePoint],
        now: UnixMillis,
    ) -> sqlx::Result<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM scheduled_pause_points WHERE job_id = ?")
            .bind(job_id.to_string())
            .execute(&mut *tx)
            .await?;
        for (idx, point) in points.iter().enumerate() {
            let target_json = serde_json::to_string(&point.target).map_err(serde_err)?;
            // 1-based ordinal matches the YAML position the operator
            // sees in the editor and the spelling the parser uses for
            // duplicate errors.
            let ordinal = (idx + 1) as i64;
            sqlx::query(
                "INSERT INTO scheduled_pause_points \
                 (job_id, ordinal, point_id, target_json, position, reason, created_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(job_id.to_string())
            .bind(ordinal)
            .bind(point.id.to_string())
            .bind(target_json)
            .bind(position_label(point.position))
            .bind(point.reason.as_deref())
            .bind(now.0)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Load the schedule for one job in YAML order. Empty when the
    /// template carried no `pause_points:` block, when every entry
    /// has been removed via resync, or when the job predates the
    /// feature.
    pub async fn list_scheduled_pause_points(
        &self,
        job_id: JobId,
    ) -> sqlx::Result<Vec<PausePoint>> {
        let rows = sqlx::query(
            "SELECT point_id, target_json, position, reason \
             FROM scheduled_pause_points \
             WHERE job_id = ? \
             ORDER BY ordinal ASC",
        )
        .bind(job_id.to_string())
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                let point_id: String = row.try_get("point_id")?;
                let target_json: String = row.try_get("target_json")?;
                let position: String = row.try_get("position")?;
                let reason: Option<String> = row.try_get("reason")?;
                let target: PausePointTarget =
                    serde_json::from_str(&target_json).map_err(serde_err)?;
                Ok(PausePoint {
                    id: parse_id::<PausePointId>(&point_id)?,
                    target,
                    position: parse_position(&position)?,
                    reason,
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migrations::MIGRATOR;
    use codeless_types::pause_point::TodoSelector;
    use codeless_types::{
        CostCents, GitAuth, Job, JobId, JobStatus, Repo, RepoId, TodoKind, WorkspaceMode,
    };
    use sqlx::sqlite::SqlitePoolOptions;

    async fn fresh_store_with_job() -> (SqliteStore, JobId) {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        MIGRATOR.run(&pool).await.unwrap();
        let store = SqliteStore::new(pool);

        let repo = Repo {
            id: RepoId::new(),
            name: "r".into(),
            clone_url: "u".into(),
            default_branch: "main".into(),
            local_path: "/tmp".into(),
            git_auth: GitAuth::Ssh {
                key_path: "/tmp/k".into(),
            },
            concurrency_cap: None,
            default_runner: None,
            created_at: UnixMillis(0),
            updated_at: UnixMillis(0),
        };
        store.insert_repo(&repo).await.unwrap();

        let job = Job {
            id: JobId::new(),
            repo_id: repo.id,
            status: JobStatus::Queued,
            stop_reason: None,
            template_yaml: None,
            prompt: None,
            runner: "mock".into(),
            branch: "b".into(),
            workspace_mode: WorkspaceMode::Worktree,
            worktree_path: None,
            cost_cap_cents: CostCents(0),
            wall_clock_cap_ms: 0,
            cost_cents: CostCents(0),
            model: None,
            permission_mode: None,
            effort: None,
            system_prompt: None,
            persona_id: None,
            auto_bypass_policy: None,
            pending_operator_comment: None,
            precheck_override_once: false,
            started_at: None,
            ended_at: None,
            created_at: UnixMillis(0),
        };
        store.insert_job(&job).await.unwrap();
        (store, job.id)
    }

    fn stage_point(stage_ordinal: u32, position: PausePointPosition) -> PausePoint {
        PausePoint {
            id: PausePointId::new(),
            target: PausePointTarget::Stage {
                ordinal: stage_ordinal,
            },
            position,
            reason: None,
        }
    }

    fn trio_point(stage_ordinal: u32, kind: TodoKind, reason: &str) -> PausePoint {
        PausePoint {
            id: PausePointId::new(),
            target: PausePointTarget::StageTodo {
                stage_ordinal,
                selector: TodoSelector::Trio { kind },
            },
            position: PausePointPosition::After,
            reason: Some(reason.into()),
        }
    }

    #[tokio::test]
    async fn empty_input_clears_the_schedule() {
        let (store, job_id) = fresh_store_with_job().await;
        store
            .replace_scheduled_pause_points(
                job_id,
                &[stage_point(1, PausePointPosition::Before)],
                UnixMillis(0),
            )
            .await
            .unwrap();
        store
            .replace_scheduled_pause_points(job_id, &[], UnixMillis(1))
            .await
            .unwrap();
        assert!(store
            .list_scheduled_pause_points(job_id)
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn roundtrip_preserves_targets_position_and_reason() {
        let (store, job_id) = fresh_store_with_job().await;
        let points = vec![
            stage_point(2, PausePointPosition::Before),
            trio_point(3, TodoKind::Docs, "after stage 3 docs"),
            PausePoint {
                id: PausePointId::new(),
                target: PausePointTarget::StageTodo {
                    stage_ordinal: 4,
                    selector: TodoSelector::TitleSubstring {
                        pattern: "migrate".into(),
                    },
                },
                position: PausePointPosition::Before,
                reason: None,
            },
            PausePoint {
                id: PausePointId::new(),
                target: PausePointTarget::StageTodo {
                    stage_ordinal: 5,
                    selector: TodoSelector::Ordinal { ordinal: 7 },
                },
                position: PausePointPosition::After,
                reason: Some("between todos".into()),
            },
        ];
        store
            .replace_scheduled_pause_points(job_id, &points, UnixMillis(42))
            .await
            .unwrap();
        let loaded = store.list_scheduled_pause_points(job_id).await.unwrap();
        assert_eq!(loaded, points);
    }

    #[tokio::test]
    async fn rebuild_is_idempotent_on_repeated_input() {
        let (store, job_id) = fresh_store_with_job().await;
        let points = vec![
            stage_point(1, PausePointPosition::Before),
            trio_point(2, TodoKind::Git, "after git"),
        ];
        store
            .replace_scheduled_pause_points(job_id, &points, UnixMillis(1))
            .await
            .unwrap();
        store
            .replace_scheduled_pause_points(job_id, &points, UnixMillis(2))
            .await
            .unwrap();
        let loaded = store.list_scheduled_pause_points(job_id).await.unwrap();
        assert_eq!(loaded, points);
    }

    #[tokio::test]
    async fn rebuild_drops_rows_that_left_the_schedule() {
        // The resync contract: a chat-driven template edit that
        // removes a `pause_points:` entry must remove its row on the
        // next resync, not leave an orphan that the runtime later
        // trips on. The parser hands back the *new* resolved set; the
        // store replays the whole thing.
        let (store, job_id) = fresh_store_with_job().await;
        let before = vec![
            stage_point(1, PausePointPosition::Before),
            stage_point(2, PausePointPosition::Before),
            stage_point(3, PausePointPosition::Before),
        ];
        store
            .replace_scheduled_pause_points(job_id, &before, UnixMillis(0))
            .await
            .unwrap();
        let after = vec![stage_point(2, PausePointPosition::Before)];
        store
            .replace_scheduled_pause_points(job_id, &after, UnixMillis(0))
            .await
            .unwrap();
        let loaded = store.list_scheduled_pause_points(job_id).await.unwrap();
        assert_eq!(loaded.len(), 1);
        assert!(matches!(
            loaded[0].target,
            PausePointTarget::Stage { ordinal: 2 }
        ));
    }

    #[tokio::test]
    async fn rebuild_renumbers_ordinals_for_the_new_yaml_order() {
        // YAML index, not point id, is the row key — moving a point's
        // position in `pause_points:` rewrites the row layout. The
        // identity (`PausePointId`) is fresh per resolve, so we can
        // observe the renumbering by reading the rows back in order.
        let (store, job_id) = fresh_store_with_job().await;
        let first = vec![
            stage_point(1, PausePointPosition::Before),
            stage_point(2, PausePointPosition::Before),
        ];
        store
            .replace_scheduled_pause_points(job_id, &first, UnixMillis(0))
            .await
            .unwrap();
        let swapped = vec![
            stage_point(2, PausePointPosition::Before),
            stage_point(1, PausePointPosition::Before),
        ];
        store
            .replace_scheduled_pause_points(job_id, &swapped, UnixMillis(0))
            .await
            .unwrap();
        let loaded = store.list_scheduled_pause_points(job_id).await.unwrap();
        assert!(matches!(
            loaded[0].target,
            PausePointTarget::Stage { ordinal: 2 }
        ));
        assert!(matches!(
            loaded[1].target,
            PausePointTarget::Stage { ordinal: 1 }
        ));
    }

    #[tokio::test]
    async fn job_isolation_keeps_schedules_separate() {
        let (store, job_a) = fresh_store_with_job().await;
        // A second job sharing the same store; the schedule for one
        // must not leak across when the other rebuilds.
        let repo = Repo {
            id: RepoId::new(),
            name: "r2".into(),
            clone_url: "u".into(),
            default_branch: "main".into(),
            local_path: "/tmp".into(),
            git_auth: GitAuth::Ssh {
                key_path: "/tmp/k".into(),
            },
            concurrency_cap: None,
            default_runner: None,
            created_at: UnixMillis(0),
            updated_at: UnixMillis(0),
        };
        store.insert_repo(&repo).await.unwrap();
        let job_b = Job {
            id: JobId::new(),
            repo_id: repo.id,
            status: JobStatus::Queued,
            stop_reason: None,
            template_yaml: None,
            prompt: None,
            runner: "mock".into(),
            branch: "b".into(),
            workspace_mode: WorkspaceMode::Worktree,
            worktree_path: None,
            cost_cap_cents: CostCents(0),
            wall_clock_cap_ms: 0,
            cost_cents: CostCents(0),
            model: None,
            permission_mode: None,
            effort: None,
            system_prompt: None,
            persona_id: None,
            auto_bypass_policy: None,
            pending_operator_comment: None,
            precheck_override_once: false,
            started_at: None,
            ended_at: None,
            created_at: UnixMillis(0),
        };
        store.insert_job(&job_b).await.unwrap();

        store
            .replace_scheduled_pause_points(
                job_a,
                &[stage_point(1, PausePointPosition::Before)],
                UnixMillis(0),
            )
            .await
            .unwrap();
        store
            .replace_scheduled_pause_points(
                job_b.id,
                &[stage_point(2, PausePointPosition::After)],
                UnixMillis(0),
            )
            .await
            .unwrap();
        store
            .replace_scheduled_pause_points(job_a, &[], UnixMillis(0))
            .await
            .unwrap();
        assert!(store
            .list_scheduled_pause_points(job_a)
            .await
            .unwrap()
            .is_empty());
        let b = store.list_scheduled_pause_points(job_b.id).await.unwrap();
        assert_eq!(b.len(), 1);
        assert_eq!(b[0].position, PausePointPosition::After);
    }

    #[tokio::test]
    async fn cascade_on_job_delete_removes_rows() {
        let (store, job_id) = fresh_store_with_job().await;
        store
            .replace_scheduled_pause_points(
                job_id,
                &[stage_point(1, PausePointPosition::Before)],
                UnixMillis(0),
            )
            .await
            .unwrap();
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(store.pool())
            .await
            .unwrap();
        sqlx::query("DELETE FROM jobs WHERE id = ?")
            .bind(job_id.to_string())
            .execute(store.pool())
            .await
            .unwrap();
        assert!(store
            .list_scheduled_pause_points(job_id)
            .await
            .unwrap()
            .is_empty());
    }
}
