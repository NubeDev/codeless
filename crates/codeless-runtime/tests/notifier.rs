//! End-to-end exercise of the webhook notifier. Stands up a
//! `wiremock` server, points a `WebhookNotifier` at it, fires both
//! trigger events through the `EventBus` via `spawn_notifier`, and
//! asserts the captured requests: JSON body, content-type, HMAC
//! signature header (verified against the same key), and the
//! `kind` discriminator.

use std::sync::Arc;
use std::time::Duration;

use codeless_runtime::{
    spawn_notifier, EventBus, NotificationKind, WebhookConfig, WebhookNotifier, MIGRATOR,
};
use codeless_types::{Event, JobId, ReviewId, StageId, UnixMillis};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use sqlx::SqlitePool;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn setup_bus() -> Arc<EventBus> {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    MIGRATOR.run(&pool).await.unwrap();
    Arc::new(EventBus::new(pool, 1024))
}

fn verify_signature(key: &[u8], body: &[u8], signature_hex: &str) -> bool {
    let expected = hex::decode(signature_hex).expect("valid hex");
    let mut mac = Hmac::<Sha256>::new_from_slice(key).unwrap();
    mac.update(body);
    mac.verify_slice(&expected).is_ok()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn webhook_fires_on_job_failed_and_review_requested() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/hook"))
        .and(header("content-type", "application/json"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;

    let key = b"unit-test-key".to_vec();
    let notifier = Arc::new(WebhookNotifier::new(
        format!("{}/hook", server.uri()),
        key.clone(),
    ));

    let bus = setup_bus().await;
    let handle = spawn_notifier(Arc::clone(&bus), notifier).await.unwrap();

    let job_id = JobId::new();
    let stage_id = StageId::new();
    let review_id = ReviewId::new();

    bus.publish(
        Some(job_id),
        None,
        None,
        Event::JobFailed { job_id },
        UnixMillis(1_000),
    )
    .await
    .unwrap();
    bus.publish(
        None,
        Some(stage_id),
        None,
        Event::ReviewRequested {
            review_id,
            stage_id,
        },
        UnixMillis(2_000),
    )
    .await
    .unwrap();
    bus.publish(
        Some(job_id),
        None,
        None,
        Event::JobStarted { job_id },
        UnixMillis(1_500),
    )
    .await
    .unwrap();

    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        let received = server.received_requests().await.unwrap_or_default();
        if received.len() >= 2 {
            assert_eq!(received.len(), 2, "JobStarted must not trigger a webhook");
            let kinds: Vec<NotificationKind> = received
                .iter()
                .map(|r| {
                    let v: serde_json::Value = serde_json::from_slice(&r.body).unwrap();
                    serde_json::from_value(v["kind"].clone()).unwrap()
                })
                .collect();
            assert!(kinds.contains(&NotificationKind::JobFailed));
            assert!(kinds.contains(&NotificationKind::ReviewRequested));
            for req in &received {
                let sig = req
                    .headers
                    .get(WebhookNotifier::SIGNATURE_HEADER)
                    .expect("signature header present");
                assert!(
                    verify_signature(&key, &req.body, sig.to_str().unwrap()),
                    "signature must verify with the shared key"
                );
            }
            break;
        }
        if std::time::Instant::now() >= deadline {
            panic!(
                "timed out waiting for 2 webhook deliveries; got {}",
                received.len()
            );
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    handle.abort();
}

#[test]
fn config_round_trip_through_toml() {
    let cfg = WebhookConfig {
        url: "https://hooks.example.com/x".into(),
        hmac_key_hex: hex::encode(b"unit-test-key"),
    };
    let serialised = toml::to_string(&cfg).unwrap();
    let decoded: WebhookConfig = toml::from_str(&serialised).unwrap();
    assert_eq!(cfg, decoded);
    let notifier = WebhookNotifier::from_config(decoded).unwrap();
    assert_eq!(WebhookNotifier::SIGNATURE_HEADER, "x-codeless-signature");
    drop(notifier);
}

#[test]
fn empty_hmac_key_is_rejected_at_setup() {
    let cfg = WebhookConfig {
        url: "https://hooks.example.com/x".into(),
        hmac_key_hex: String::new(),
    };
    let err = match WebhookNotifier::from_config(cfg) {
        Ok(_) => panic!("expected error for empty hmac_key_hex"),
        Err(e) => e,
    };
    let msg = format!("{err}");
    assert!(msg.contains("empty key"), "got {msg}");
}
