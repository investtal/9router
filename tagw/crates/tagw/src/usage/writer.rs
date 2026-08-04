use std::time::Duration;

use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::{interval, MissedTickBehavior};

use crate::db::Db;
use crate::usage::UsageEvent;

/// Max events per batch flush.
const BATCH_SIZE: usize = 64;
/// Flush interval when the batch is non-empty but under capacity.
const BATCH_INTERVAL: Duration = Duration::from_millis(50);

/// Spawn the dedicated usage writer task.
///
/// Batches inserts into `request_logs` every [`BATCH_INTERVAL`] or when
/// [`BATCH_SIZE`] events accumulate — whichever comes first. Uses a single
/// SQLite connection via [`Db`] (mutex). Write failures are logged and the
/// batch is dropped so the proxy hot path is never blocked.
pub fn spawn_usage_writer(db: Db, mut rx: mpsc::Receiver<UsageEvent>) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut batch: Vec<UsageEvent> = Vec::with_capacity(BATCH_SIZE);
        let mut tick = interval(BATCH_INTERVAL);
        tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
        // Consume the immediate first tick so we don't flush on spawn.
        tick.tick().await;

        loop {
            tokio::select! {
                maybe = rx.recv() => {
                    match maybe {
                        Some(ev) => {
                            batch.push(ev);
                            if batch.len() >= BATCH_SIZE {
                                flush(&db, &mut batch);
                            }
                        }
                        None => {
                            // Channel closed: drain remaining and exit.
                            if !batch.is_empty() {
                                flush(&db, &mut batch);
                            }
                            break;
                        }
                    }
                }
                _ = tick.tick() => {
                    if !batch.is_empty() {
                        flush(&db, &mut batch);
                    }
                }
            }
        }
    })
}

fn flush(db: &Db, batch: &mut Vec<UsageEvent>) {
    if batch.is_empty() {
        return;
    }
    let count = batch.len();
    let result = db.with_conn(|conn| {
        let tx = conn.unchecked_transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO request_logs (
                    id, created_at, member_id, member_key_id, provider_id, account_id,
                    model, tool, status, prompt_tokens, completion_tokens, cached_tokens,
                    cost_est, latency_ms, ttft_ms, usage_incomplete, error,
                    request_body, response_body
                ) VALUES (
                    ?1, ?2, NULL, ?3, ?4, ?5,
                    ?6, ?7, ?8, ?9, ?10, ?11,
                    ?12, ?13, ?14, ?15, ?16,
                    ?17, ?18
                )",
            )?;
            for ev in batch.iter() {
                stmt.execute(rusqlite::params![
                    ev.id,
                    ev.created_at,
                    ev.member_key_id,
                    ev.provider_id,
                    ev.account_id,
                    ev.model,
                    ev.tool,
                    ev.status,
                    ev.prompt_tokens,
                    ev.completion_tokens,
                    ev.cached_tokens,
                    ev.cost_est,
                    ev.latency_ms,
                    ev.ttft_ms,
                    i64::from(ev.usage_incomplete),
                    ev.error,
                    ev.request_body,
                    ev.response_body,
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    });
    if let Err(e) = result {
        tracing::error!(
            error = %e,
            count,
            "usage writer batch insert failed; dropping batch"
        );
    }
    batch.clear();
}
