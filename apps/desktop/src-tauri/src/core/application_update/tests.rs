use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

#[tokio::test]
async fn pause_preserves_the_same_future_and_accumulated_download() {
    let starts = Arc::new(AtomicUsize::new(0));
    let observed = Arc::new(AtomicUsize::new(0));
    let (chunk_tx, mut chunk_rx) = tokio::sync::mpsc::unbounded_channel::<usize>();
    let (ack_tx, mut ack_rx) = tokio::sync::mpsc::unbounded_channel();
    let count = starts.clone();
    let bytes = observed.clone();
    let download = async move {
        count.fetch_add(1, Ordering::SeqCst);
        while let Some(chunk) = chunk_rx.recv().await {
            bytes.fetch_add(chunk, Ordering::SeqCst);
            ack_tx.send(()).unwrap();
        }
        bytes.load(Ordering::SeqCst)
    };
    let (control, receiver) = watch::channel(true);
    let task = tokio::spawn(pausable(download, receiver));
    chunk_tx.send(5).unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(20), ack_rx.recv())
            .await
            .is_err()
    );
    assert_eq!(starts.load(Ordering::SeqCst), 0);
    control.send(false).unwrap();
    ack_rx.recv().await.unwrap();
    assert_eq!(observed.load(Ordering::SeqCst), 5);
    control.send(true).unwrap();
    chunk_tx.send(7).unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(20), ack_rx.recv())
            .await
            .is_err()
    );
    assert_eq!(observed.load(Ordering::SeqCst), 5);
    control.send(false).unwrap();
    ack_rx.recv().await.unwrap();
    drop(chunk_tx);
    assert_eq!(task.await.unwrap(), 12);
    assert_eq!(starts.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn paused_download_does_not_turn_a_verification_failure_into_success() {
    let (control, receiver) = watch::channel(true);
    let download = async { Err::<Vec<u8>, _>("invalid signature") };
    let task = tokio::spawn(pausable(download, receiver));
    control.send(false).unwrap();
    assert_eq!(task.await.unwrap(), Err("invalid signature"));
}

#[test]
fn diagnostics_keep_status_and_signature_clues_without_server_secrets() {
    use tauri_plugin_updater::Error;
    assert!(
        safe_reason(&Error::Network(
            "Download request failed with status: 403 Forbidden".into()
        ))
        .contains("403")
    );
    assert!(safe_reason(&Error::SignatureUtf8("private signature bytes".into())).contains("签名"));
    assert!(
        !safe_reason(&Error::Network(
            "https://example.test/?token=private".into()
        ))
        .contains("private")
    );
}
