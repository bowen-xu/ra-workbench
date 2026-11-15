//! The new claim preflight uses the invocation's real worker lease. Preparation
//! cannot bypass it, while the existing TurnStarted transaction remains final.
use super::*;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::oneshot;

struct Lease {
    dropped: Arc<AtomicBool>,
    finished: Option<oneshot::Sender<()>>,
}
impl Drop for Lease {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::Release);
        let _ = self.finished.take().unwrap().send(());
    }
}

#[tokio::test]
async fn canceled_waiter_retains_the_real_queued_claim_worker_until_sql_releases() {
    let f = fixture("/compact").await;
    // Hold the same SQLite mutex the actual preflight must acquire. Dropping the
    // outer JoinHandle cannot cancel this physical blocking operation.
    let db = f.store.connection().unwrap();
    let dropped = Arc::new(AtomicBool::new(false));
    let (finished, finish_wait) = oneshot::channel();
    let retention = WorkerRetention::new(Lease {
        dropped: dropped.clone(),
        finished: Some(finished),
    });
    let worker = queued_claim_worker(
        f.store.clone(),
        f.record.conversation_id,
        f.record.request_id,
        &retention,
    );
    drop(retention);
    let (entered, started) = oneshot::channel();
    let outer = tokio::spawn(async move {
        let _ = entered.send(());
        worker.await
    });
    started.await.unwrap();
    outer.abort();
    assert!(outer.await.unwrap_err().is_cancelled());
    assert!(!dropped.load(Ordering::Acquire));
    drop(db);
    tokio::time::timeout(std::time::Duration::from_secs(10), finish_wait)
        .await
        .unwrap()
        .unwrap();
    assert!(dropped.load(Ordering::Acquire));
    assert_eq!(
        f.sessions
            .open(ConversationId::from_uuid(f.record.conversation_id))
            .await
            .unwrap()
            .seq(),
        0
    );
}

#[tokio::test]
async fn missing_queued_claim_rejects_before_factory_or_model_preparation() {
    let f = fixture("/compact").await;
    {
        let db = f.store.connection().unwrap();
        let input = Uuid::new_v4();
        db.execute(
            "insert into agent_input_queue_identities(input_id,conversation_id) values(?1,?2)",
            params![input.to_string(), f.record.conversation_id.to_string()],
        )
        .unwrap();
        // An origin without its immutable claim is not permission to execute.
        db.execute("insert into agent_queue_submission_origins(conversation_id,request_id,input_id) values(?1,?2,?3)", params![f.record.conversation_id.to_string(),f.record.request_id.to_string(),input.to_string()]).unwrap();
    }
    let result = start_checked(
        &f.store,
        f.sessions.clone(),
        f.request.clone(),
        |_, _| async { panic!("invalid queue claim prepared credentials or transport") },
    )
    .await;
    assert!(matches!(result, Err(Error::ConfirmationMismatch)));
    assert_eq!(
        f.sessions
            .open(ConversationId::from_uuid(f.record.conversation_id))
            .await
            .unwrap()
            .seq(),
        0
    );
}
