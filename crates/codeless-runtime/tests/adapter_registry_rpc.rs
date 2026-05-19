//! End-to-end exercise for stage 6 of the adapter-registry job: the
//! six chat-adapter / runner RPCs round-trip through `InProcessRpc`
//! and the SQLite tables `chat_adapters` + `runner_config`. Three
//! contracts are pinned here:
//!
//! 1. `list_chat_adapters` → `validate_chat_adapter_secrets` →
//!    `set_chat_adapter_enabled(true)` flips the row to enabled and
//!    "arms" it for the next restart (the row is visible to a fresh
//!    `list_chat_adapters` call).
//! 2. `set_chat_adapter_enabled(true)` without a prior successful
//!    validate refuses with `AdapterError::MissingSecrets` carrying
//!    the kind's canonical secret-key names. The validate-before-
//!    enable coupling is the only correct refusal path per
//!    `DOCS/WORKSPACE-ATTACH.md` §"TODO — adapter registry".
//! 3. The validate path enforces a 5/s per-`(kind, instance_id)`
//!    rate limit and a 5s hard timeout. A 6th call inside the same
//!    second is refused with `Conflict`; a probe that never resolves
//!    surfaces as `ChatAdapterSecretProblem::Timeout` (not as an RPC
//!    error — the timeout is an observation the UI renders).
//!
//! The runner-side RPCs (`list_runners`, `set_runner_enabled`) have
//! no validate step, so the test asserts the simpler upsert path.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use codeless_rpc::{
    AdapterError, ChatAdapterKind, ChatAdapterSecretProblem, RpcError, RpcServer,
    SetChatAdapterEnabledArgs, SetRunnerEnabledArgs, ValidateChatAdapterSecretsArgs,
    ValidateChatAdapterSecretsResult,
};
use codeless_runtime::{InProcessRpc, StaticValidationProbe, ValidationProbe};

fn ok_result() -> ValidateChatAdapterSecretsResult {
    ValidateChatAdapterSecretsResult {
        ok: true,
        problems: vec![],
    }
}

fn fail_result(reason: &str) -> ValidateChatAdapterSecretsResult {
    ValidateChatAdapterSecretsResult {
        ok: false,
        problems: vec![ChatAdapterSecretProblem::Unauthorized {
            reason: reason.to_owned(),
        }],
    }
}

async fn rpc_with_probe(probe: Arc<dyn ValidationProbe>) -> InProcessRpc {
    InProcessRpc::new()
        .await
        .expect("fresh in-memory runtime")
        .with_validation_probe(probe)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn list_validate_set_enabled_round_trip_arms_restart() {
    let probe = Arc::new(StaticValidationProbe {
        result: ok_result(),
    });
    let rpc = rpc_with_probe(probe).await;

    // Fresh runtime: no rows yet.
    let initial = rpc.list_chat_adapters().await.expect("list ok");
    assert!(
        initial.adapters.is_empty(),
        "expected empty table on a fresh runtime"
    );

    // Validate first — this is what unlocks the subsequent enable.
    let validated = rpc
        .validate_chat_adapter_secrets(ValidateChatAdapterSecretsArgs {
            kind: ChatAdapterKind::Slack,
            instance_id: "default".into(),
        })
        .await
        .expect("validate ok");
    assert!(validated.ok, "ok probe must return ok=true");
    assert!(validated.problems.is_empty());

    // Now enable. The validated cache satisfies the gate.
    rpc.set_chat_adapter_enabled(SetChatAdapterEnabledArgs {
        kind: ChatAdapterKind::Slack,
        instance_id: "default".into(),
        enabled: true,
    })
    .await
    .expect("set_chat_adapter_enabled true ok after validate");

    // The row is now armed: a fresh list sees the enabled row, which
    // is what `--enable-*`-replacement boot would read next time.
    let listed = rpc
        .list_chat_adapters()
        .await
        .expect("list after enable ok");
    assert_eq!(listed.adapters.len(), 1);
    let row = &listed.adapters[0];
    assert_eq!(row.kind, ChatAdapterKind::Slack);
    assert_eq!(row.instance_id, "default");
    assert!(row.enabled, "the row must be armed enabled");

    // Disabling is always allowed even after the cache is cleared —
    // the user must be able to switch off a broken adapter.
    rpc.set_chat_adapter_enabled(SetChatAdapterEnabledArgs {
        kind: ChatAdapterKind::Slack,
        instance_id: "default".into(),
        enabled: false,
    })
    .await
    .expect("disable always ok");

    // Re-enabling now must re-validate first: the previous successful
    // validate is invalidated by the disable.
    let refused = rpc
        .set_chat_adapter_enabled(SetChatAdapterEnabledArgs {
            kind: ChatAdapterKind::Slack,
            instance_id: "default".into(),
            enabled: true,
        })
        .await
        .expect_err("re-enable without re-validate must refuse");
    match refused {
        RpcError::Adapter(AdapterError::MissingSecrets { keys }) => {
            assert!(keys.contains(&"slack_app_token".to_owned()));
            assert!(keys.contains(&"slack_bot_token".to_owned()));
        }
        other => panic!("expected MissingSecrets, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn set_enabled_without_validate_returns_missing_secrets() {
    let probe = Arc::new(StaticValidationProbe {
        result: ok_result(),
    });
    let rpc = rpc_with_probe(probe).await;

    let refused = rpc
        .set_chat_adapter_enabled(SetChatAdapterEnabledArgs {
            kind: ChatAdapterKind::Telegram,
            instance_id: "default".into(),
            enabled: true,
        })
        .await
        .expect_err("set true on a never-validated row must refuse");
    match refused {
        RpcError::Adapter(AdapterError::MissingSecrets { keys }) => {
            assert_eq!(keys, vec!["telegram_bot_token".to_owned()]);
        }
        other => panic!("expected MissingSecrets, got {other:?}"),
    }

    // The refused call must not have written a row.
    let listed = rpc.list_chat_adapters().await.unwrap();
    assert!(listed.adapters.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn validate_failure_does_not_unlock_enable() {
    let probe = Arc::new(StaticValidationProbe {
        result: fail_result("invalid_auth"),
    });
    let rpc = rpc_with_probe(probe).await;

    let result = rpc
        .validate_chat_adapter_secrets(ValidateChatAdapterSecretsArgs {
            kind: ChatAdapterKind::Slack,
            instance_id: "default".into(),
        })
        .await
        .expect("validate call itself succeeds");
    assert!(!result.ok, "failure result must surface ok=false");

    // A failed validate must NOT arm the cache — re-enabling has to
    // wait on a successful probe.
    let refused = rpc
        .set_chat_adapter_enabled(SetChatAdapterEnabledArgs {
            kind: ChatAdapterKind::Slack,
            instance_id: "default".into(),
            enabled: true,
        })
        .await
        .expect_err("failed validate must not unlock enable");
    assert!(
        matches!(
            refused,
            RpcError::Adapter(AdapterError::MissingSecrets { .. })
        ),
        "expected MissingSecrets after a failed validate, got {refused:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn validate_rate_limited_per_bucket() {
    let probe = Arc::new(StaticValidationProbe {
        result: ok_result(),
    });
    let rpc = rpc_with_probe(probe).await;

    let args = ValidateChatAdapterSecretsArgs {
        kind: ChatAdapterKind::Slack,
        instance_id: "default".into(),
    };
    // Five calls fit; the sixth in the same second is refused.
    for n in 0..5 {
        rpc.validate_chat_adapter_secrets(args.clone())
            .await
            .unwrap_or_else(|e| panic!("call {n} should fit in the bucket: {e:?}"));
    }
    let blocked = rpc
        .validate_chat_adapter_secrets(args.clone())
        .await
        .expect_err("6th call inside the window must refuse");
    assert!(
        matches!(blocked, RpcError::Conflict(_)),
        "expected Conflict for rate-limit, got {blocked:?}"
    );

    // A different bucket is unaffected — the limit is per-(kind,
    // instance_id), not per-connection.
    rpc.validate_chat_adapter_secrets(ValidateChatAdapterSecretsArgs {
        kind: ChatAdapterKind::Telegram,
        instance_id: "default".into(),
    })
    .await
    .expect("a different bucket is unaffected by Slack's rate limit");
}

struct NeverProbe;

#[async_trait]
impl ValidationProbe for NeverProbe {
    async fn validate(
        &self,
        _kind: ChatAdapterKind,
        _instance_id: &str,
    ) -> ValidateChatAdapterSecretsResult {
        // Sleep well past the 5s timeout so the wrapper trips first.
        // The wrapper cancels the future on timeout — sleeping
        // longer than the cap is the way to deterministically prove
        // the cap fires without measuring real wall-clock.
        tokio::time::sleep(Duration::from_secs(60)).await;
        unreachable!("the 5s timeout fires before this resolves")
    }
}

#[tokio::test(flavor = "current_thread")]
async fn validate_hard_timeout_fires_at_five_seconds() {
    // Build the runtime first under real time so sqlx's pool connect
    // does not race the paused clock; then pause and advance past
    // the 5s cap so the timeout wrapper fires deterministically.
    let rpc = rpc_with_probe(Arc::new(NeverProbe)).await;
    tokio::time::pause();
    let args = ValidateChatAdapterSecretsArgs {
        kind: ChatAdapterKind::Slack,
        instance_id: "default".into(),
    };
    let mut fut = Box::pin(rpc.validate_chat_adapter_secrets(args));
    // First poll registers the timeout + the probe sleep; both are
    // pending. Advancing fake time past 5s wakes the timeout future.
    tokio::select! {
        _ = &mut fut => panic!("validate completed before time advanced"),
        _ = tokio::time::sleep(Duration::from_millis(1)) => {}
    }
    tokio::time::advance(Duration::from_secs(6)).await;
    let result = fut.await.expect("rpc ok");
    assert!(!result.ok);
    assert_eq!(result.problems, vec![ChatAdapterSecretProblem::Timeout]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn runner_round_trip() {
    let rpc = InProcessRpc::new().await.unwrap();

    assert!(rpc.list_runners().await.unwrap().runners.is_empty());

    rpc.set_runner_enabled(SetRunnerEnabledArgs {
        runner_id: "claude".into(),
        enabled: true,
    })
    .await
    .unwrap();

    let listed = rpc.list_runners().await.unwrap();
    assert_eq!(listed.runners.len(), 1);
    assert_eq!(listed.runners[0].runner_id, "claude");
    assert!(listed.runners[0].enabled);

    // Toggling off must work without a validate step.
    rpc.set_runner_enabled(SetRunnerEnabledArgs {
        runner_id: "claude".into(),
        enabled: false,
    })
    .await
    .unwrap();
    let listed = rpc.list_runners().await.unwrap();
    assert!(!listed.runners[0].enabled);

    // Empty runner_id is rejected so the table doesn't grow ghost
    // rows on a UI fat-finger.
    let err = rpc
        .set_runner_enabled(SetRunnerEnabledArgs {
            runner_id: "  ".into(),
            enabled: true,
        })
        .await
        .expect_err("empty runner_id must refuse");
    assert!(matches!(err, RpcError::InvalidArgument(_)));
}
