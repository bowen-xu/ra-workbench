use super::*;
use crate::runtime_plugins::native::package::{stage, tests::fixture};
use futures_util::poll;
use std::{path::PathBuf, time::Duration};
use tokio::time::timeout;

const WAIT: Duration = Duration::from_secs(8);
const READ_REQUEST: &str = r#"IFS= read -r line
identity=${line%\"body\":*}
"#;
const INITIALIZE: &str = r#"IFS= read -r line
identity=${line%\"body\":*}
printf '%s"body":{"kind":"initialized","capability":"writing.import-preview","version":1}}\n' "$identity"
"#;
const SUCCESS: &str = r#"printf '%s"body":{"kind":"previewImportResult","outcome":{"status":"success","preview":{"fileCount":1,"byteSize":3,"rootCandidates":["main.tex"]}}}}\n' "$identity""#;
const DOMAIN_FAILURE: &str = r#"printf '%s"body":{"kind":"previewImportResult","outcome":{"status":"failure","error":{"code":"invalid_input","message":"synthetic domain rejection"}}}}\n' "$identity""#;
const SHUTDOWN: &str = r#"IFS= read -r line
identity=${line%\"body\":*}
printf '%s"body":{"kind":"shutdownComplete"}}\n' "$identity"
"#;

fn script(body: &str) -> Vec<u8> {
    format!("#!/bin/sh\n{INITIALIZE}{body}\n").into_bytes()
}

// Complete startup before polling a request so concurrency tests observe the
// real pipe wait rather than racing an unrelated blocking staging task.
async fn pending_fixture() -> (tempfile::TempDir, NativePreviewService, String, PathBuf) {
    let (root, catalog, key) = fixture(&script("IFS= read -r line\nIFS= read -r another\nexit 0"));
    let staged = stage(&catalog, &key).unwrap();
    let directory = staged.directory.path().to_owned();
    let mut host = transport::NativeHost::spawn(staged, &key).unwrap();
    timeout(WAIT, host.initialize()).await.unwrap().unwrap();
    let service = NativePreviewService::new(catalog);
    *service.host.lock().await = Some(host);
    (root, service, key, directory)
}

#[tokio::test]
async fn domain_rejection_keeps_the_same_healthy_instance_for_manual_calls() {
    let body = format!(
        "{READ_REQUEST}{DOMAIN_FAILURE}\n{READ_REQUEST}{SUCCESS}\n{READ_REQUEST}{DOMAIN_FAILURE}\n{SHUTDOWN}"
    );
    let (_root, catalog, key) = fixture(&script(&body));
    let service = NativePreviewService::new(catalog);
    let first = timeout(
        WAIT,
        service.preview(key.clone(), "/synthetic/first".into()),
    )
    .await
    .unwrap()
    .unwrap_err();
    assert_eq!(first.code, "invalid_input");
    assert_eq!(first.message, "synthetic domain rejection");
    assert!(!service.host.lock().await.as_ref().unwrap().is_failed());
    let preview = timeout(
        WAIT,
        service.preview(key.clone(), "/synthetic/second".into()),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(preview.file_count, 1);
    assert_eq!(preview.root_candidates, ["main.tex"]);
    let third = timeout(WAIT, service.preview(key, "/synthetic/third".into()))
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(third.code, "invalid_input");
    assert!(first.instance_id.is_some() && first.request_id.is_some());
    assert_eq!(first.instance_id, third.instance_id);
    assert_ne!(first.request_id, third.request_id);
    timeout(WAIT, service.shutdown()).await.unwrap().unwrap();
    assert!(service.host.lock().await.is_none());
}

#[tokio::test]
async fn a_pending_preview_rejects_a_second_call_without_queuing_it() {
    let (_root, service, key, directory) = pending_fixture().await;
    let mut pending = Box::pin(service.preview(key.clone(), "/synthetic/pending".into()));
    assert!(poll!(&mut pending).is_pending());
    let busy = timeout(
        WAIT,
        service.preview(key.clone(), "/synthetic/second".into()),
    )
    .await
    .unwrap()
    .unwrap_err();
    assert_eq!(busy.code, "plugin_host_busy");
    assert_eq!(busy.package_key.as_deref(), Some(key.as_str()));
    drop(pending);
    assert!(service.host.lock().await.as_ref().unwrap().is_failed());
    timeout(WAIT, service.shutdown()).await.unwrap().unwrap();
    assert!(service.host.lock().await.is_none());
    assert!(!directory.exists());
}

#[tokio::test]
async fn application_shutdown_cancels_and_reaps_the_pending_owned_host() {
    let (_root, service, key, directory) = pending_fixture().await;
    let mut pending = Box::pin(service.preview(key.clone(), "/synthetic/pending".into()));
    assert!(poll!(&mut pending).is_pending());
    let (preview, shutdown) = timeout(WAIT, async { tokio::join!(pending, service.shutdown()) })
        .await
        .unwrap();
    shutdown.unwrap();
    let error = preview.unwrap_err();
    assert_eq!(error.code, "plugin_host_stopping");
    assert_eq!(error.package_key.as_deref(), Some(key.as_str()));
    assert!(error.instance_id.is_some() && error.request_id.is_some());
    assert!(error.exit_code.is_some() || error.signal.is_some());
    assert!(error.cleanup_error.is_none());
    assert!(service.host.lock().await.is_none());
    assert!(!directory.exists());
    assert_eq!(
        service
            .preview(key, "/synthetic/after-shutdown".into())
            .await
            .unwrap_err()
            .code,
        "plugin_host_stopping"
    );
}

#[tokio::test]
async fn only_a_new_manual_call_rebuilds_a_failed_instance_with_new_identity() {
    let body = format!(
        r#"while IFS= read -r line; do
identity=${{line%\"body\":*}}
case "$line" in
*'"sourcePath":"/synthetic/fail"'*) printf 'invalid JSON\n' ;;
*'"sourcePath":"/synthetic/recover"'*) {SUCCESS} ;;
*'"sourcePath":"/synthetic/identity"'*) {DOMAIN_FAILURE} ;;
*'"kind":"shutdown"'*) printf '%s"body":{{"kind":"shutdownComplete"}}}}\n' "$identity"; exit 0 ;;
*) exit 47 ;;
esac
done
"#
    );
    let (_root, catalog, key) = fixture(&script(&body));
    let service = NativePreviewService::new(catalog);
    let failed = timeout(WAIT, service.preview(key.clone(), "/synthetic/fail".into()))
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(failed.code, "plugin_host_protocol_invalid");
    assert!(failed.instance_id.is_some() && failed.request_id.is_some());
    assert!(failed.cleanup_error.is_none());
    assert!(service.host.lock().await.is_none());
    assert!(!service.cleanup_failed.load(Ordering::Acquire));

    let recovered = timeout(
        WAIT,
        service.preview(key.clone(), "/synthetic/recover".into()),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(recovered.file_count, 1);
    let new_identity = timeout(WAIT, service.preview(key, "/synthetic/identity".into()))
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(new_identity.code, "invalid_input");
    assert!(new_identity.instance_id.is_some() && new_identity.request_id.is_some());
    assert_ne!(failed.instance_id, new_identity.instance_id);
    assert_ne!(failed.request_id, new_identity.request_id);
    timeout(WAIT, service.shutdown()).await.unwrap().unwrap();
    assert!(service.host.lock().await.is_none());
}

#[tokio::test]
async fn transferred_project_lease_keeps_shutdown_waiting_through_worker_settlement() {
    let (_root, catalog, key) = fixture(&script("exit 0"));
    let service = NativePreviewService::new(catalog);
    let operation = service.project_operation(&key).await.unwrap();
    let worker_operation = operation.clone();
    drop(operation);
    let mut shutdown = Box::pin(service.shutdown());
    assert!(poll!(&mut shutdown).is_pending());
    // Reuse the accepted lease without reacquiring the fair lock behind shutdown.
    let file_operation = worker_operation.clone();
    drop(worker_operation);
    assert!(poll!(&mut shutdown).is_pending());
    drop(file_operation);
    timeout(WAIT, shutdown).await.unwrap().unwrap();
    let rejected = service.project_operation(&key).await.unwrap_err();
    assert_eq!(rejected.code, "plugin_host_stopping");
}
