use super::*;

fn bridge(generation: Uuid) -> CodexAppServerManager {
    CodexAppServerManager {
        codex_path: PathBuf::new(),
        codex_home: PathBuf::new(),
        work_root: PathBuf::new(),
        next_id: Arc::new(AtomicU64::new(1)),
        pending: Arc::new(StdMutex::new(HashMap::new())),
        notifications: broadcast::channel(8).0,
        process: Arc::new(Mutex::new(ProcessState {
            generation,
            initialized: true,
            ..ProcessState::default()
        })),
        start_lock: Arc::new(Mutex::new(())),
        auth_lock: Arc::new(Mutex::new(())),
        auth_epoch: watch::channel(Uuid::new_v4()).0,
    }
}

#[tokio::test]
async fn obsolete_reader_cannot_revoke_or_drain_replacement_process() {
    let old = Uuid::new_v4();
    let current = Uuid::new_v4();
    let bridge = bridge(current);
    let epoch = *bridge.auth_epoch.borrow();
    let mut notifications = bridge.notifications.subscribe();
    let (sender, mut response) = oneshot::channel();
    bridge.pending.lock().unwrap().insert(7, sender);
    assert!(
        !bridge
            .read_message(old, json!({"id":7,"result":"stale"}))
            .await
    );
    assert!(
        !bridge
            .read_message(old, json!({"method":"account/updated"}))
            .await
    );
    bridge.reader_closed(old).await;
    assert!(bridge.process.lock().await.initialized);
    assert_eq!(*bridge.auth_epoch.borrow(), epoch);
    assert!(matches!(
        response.try_recv(),
        Err(oneshot::error::TryRecvError::Empty)
    ));
    assert!(matches!(
        notifications.try_recv(),
        Err(broadcast::error::TryRecvError::Empty)
    ));
    assert!(
        bridge
            .read_message(current, json!({"id":7,"result":"current"}))
            .await
    );
    assert_eq!(response.await.unwrap().unwrap(), json!("current"));
}

#[tokio::test]
async fn current_reader_eof_fails_requests_and_revokes_once() {
    let current = Uuid::new_v4();
    let bridge = bridge(current);
    let original_epoch = *bridge.auth_epoch.borrow();
    let (sender, response) = oneshot::channel();
    bridge.pending.lock().unwrap().insert(7, sender);
    bridge.reader_closed(current).await;
    assert!(matches!(
        response.await.unwrap(),
        Err(AiError::TemporaryUnavailable)
    ));
    assert!(!bridge.process.lock().await.initialized);
    let revoked_epoch = *bridge.auth_epoch.borrow();
    assert_ne!(original_epoch, revoked_epoch);
    bridge.reader_closed(current).await;
    assert_eq!(*bridge.auth_epoch.borrow(), revoked_epoch);
}

#[tokio::test]
async fn stop_invalidates_reader_before_delayed_eof() {
    let current = Uuid::new_v4();
    let bridge = bridge(current);
    let (sender, response) = oneshot::channel();
    bridge.pending.lock().unwrap().insert(7, sender);
    bridge.stop().await;
    assert!(matches!(
        response.await.unwrap(),
        Err(AiError::TemporaryUnavailable)
    ));
    let epoch = *bridge.auth_epoch.borrow();
    bridge.reader_closed(current).await;
    assert_eq!(*bridge.auth_epoch.borrow(), epoch);
}

#[tokio::test]
async fn cancel_during_write_wait_removes_only_its_request() {
    use std::{
        future::Future,
        task::{Context, Waker},
    };
    let bridge = bridge(Uuid::new_v4());
    let (other, _receiver) = oneshot::channel();
    bridge.pending.lock().unwrap().insert(99, other);
    let _blocked_write = bridge.process.lock().await;
    let mut request = Box::pin(bridge.request_started("test", Value::Null, REQUEST_TIMEOUT));
    assert!(
        request
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    assert_eq!(bridge.pending.lock().unwrap().len(), 2);
    drop(request);
    let pending = bridge.pending.lock().unwrap();
    assert_eq!(pending.len(), 1);
    assert!(pending.contains_key(&99));
}

#[tokio::test]
async fn write_failure_releases_request_registration() {
    let bridge = bridge(Uuid::new_v4());
    assert!(matches!(
        bridge
            .request_started("test", Value::Null, REQUEST_TIMEOUT)
            .await,
        Err(AiError::TemporaryUnavailable)
    ));
    assert!(bridge.pending.lock().unwrap().is_empty());
}

#[cfg(unix)]
#[tokio::test]
async fn cancellation_after_pipe_write_releases_waiter() {
    let bridge = bridge(Uuid::new_v4());
    // Echo bytes only; intentionally never dispatch an RPC response to the manager.
    let mut child = Command::new("/bin/cat")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    {
        let mut process = bridge.process.lock().await;
        process.stdin = child.stdin.take();
        process.child = Some(child);
    }
    let request_bridge = bridge.clone();
    let request = tokio::spawn(async move {
        request_bridge
            .request_started("test", Value::Null, REQUEST_TIMEOUT)
            .await
    });
    let mut line = String::new();
    let mut output = BufReader::new(stdout);
    tokio::time::timeout(Duration::from_secs(5), output.read_line(&mut line))
        .await
        .unwrap()
        .unwrap();
    assert!(line.contains("test"));
    assert_eq!(bridge.pending.lock().unwrap().len(), 1);
    request.abort();
    assert!(request.await.unwrap_err().is_cancelled());
    assert!(bridge.pending.lock().unwrap().is_empty());
    assert!(matches!(
        bridge
            .request_started("test", Value::Null, Duration::from_millis(20))
            .await,
        Err(AiError::Timeout)
    ));
    assert!(bridge.pending.lock().unwrap().is_empty());
    bridge.stop().await;
}
