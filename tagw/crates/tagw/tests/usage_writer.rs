use std::time::Duration;

use tagw::db::Db;
use tagw::usage::{spawn_usage_writer, UsageEvent, USAGE_CHANNEL_CAPACITY};

fn sample_event(i: usize) -> UsageEvent {
    UsageEvent {
        id: format!("evt-{i}"),
        created_at: "2026-08-03T00:00:00Z".into(),
        member_key_id: Some(format!("key-{i}")),
        provider_id: Some("prov-1".into()),
        account_id: Some("acct-1".into()),
        model: Some("gpt-4o".into()),
        tool: Some("openai".into()),
        status: Some(200),
        prompt_tokens: 10,
        completion_tokens: 20,
        cached_tokens: 0,
        cost_est: 0.0,
        latency_ms: Some(100),
        ttft_ms: Some(50),
        usage_incomplete: false,
        error: None,
    }
}

#[tokio::test]
async fn usage_writer_batches_100_events() {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path().join("gateway.db")).unwrap();
    db.migrate().unwrap();

    let (tx, rx) = tokio::sync::mpsc::channel(USAGE_CHANNEL_CAPACITY);
    // Non-blocking buffer: capacity must be at least 1024.
    assert!(
        tx.max_capacity() >= 1024,
        "usage channel capacity must be >= 1024, got {}",
        tx.max_capacity()
    );

    let _writer = spawn_usage_writer(db.clone(), rx);

    // try_send is non-blocking; 100 events must fit without awaiting SQLite.
    for i in 0..100 {
        tx.try_send(sample_event(i))
            .expect("try_send must not block or fail under capacity");
    }

    // Drop sender so the writer flushes remaining events and exits cleanly.
    drop(tx);

    // Allow the writer task to flush (interval is 50ms; close-path flush is immediate).
    tokio::time::sleep(Duration::from_millis(200)).await;

    let n: i64 = db
        .with_conn(|c| c.query_row("SELECT COUNT(*) FROM request_logs", [], |r| r.get(0)))
        .unwrap();
    assert_eq!(n, 100, "all 100 usage events must be persisted");
}

#[tokio::test]
async fn estimate_cost_known_model_is_nonzero() {
    let c = tagw::usage::estimate_cost(Some("gpt-4o"), 100, 50, 10);
    assert!(c > 0.0, "gpt-4o cost table must produce a positive estimate, got {c}");
    assert_eq!(tagw::usage::estimate_cost(Some("unknown-xyz"), 100, 50, 0), 0.0);
}
