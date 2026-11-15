#![cfg(unix)]
use super::*;
use crate::runtime_plugins::native::package::{
    stage_for,
    tests::{fixture, literature_fixture},
};
use research_workbench_native_sdk::retrieval_trace::{MarkSentCondition, TraceRejection};
use std::{
    fs,
    future::Future,
    path::{Path, PathBuf},
};
use tokio::time::{sleep, timeout};

const WAIT: Duration = Duration::from_secs(8);
const INITIALIZE: &str = r#"IFS= read -r line
identity=${line%\"body\":*}
printf '%s"body":{"kind":"initialized","capability":"literature.note-trace-decision","version":1}}\n' "$identity"
"#;
const READ: &str = r#"IFS= read -r line
identity=${line%\"body\":*}
printf '%s"marker":true}' "$identity" > seen-identity
"#;
const READY: &str = r#"printf '%s"body":{"kind":"noteTraceDecision","decision":{"status":"ready","markSent":{"expectedRows":1,"onMismatch":"already_consumed"}}}}\n' "$identity"
"#;
const REJECT: &str = r#"printf '%s"body":{"kind":"noteTraceDecision","decision":{"status":"rejected","reason":"expired"}}}\n' "$identity"
"#;
const SHUTDOWN: &str = r#"IFS= read -r line
identity=${line%\"body\":*}
printf '%s"body":{"kind":"shutdownComplete"}}\n' "$identity"
"#;
fn script(body: &str) -> Vec<u8> {
    format!("#!/bin/sh\n{INITIALIZE}{body}\n").into_bytes()
}
fn input() -> SnapshotInput {
    SnapshotInput {
        query: " raw question ".into(),
        row: None,
    }
}

async fn warm(
    body: &str,
) -> (
    tempfile::TempDir,
    LiteratureTraceService,
    String,
    PathBuf,
    PathBuf,
) {
    let (root, catalog, key): (tempfile::TempDir, Arc<RuntimePluginCatalog>, String) =
        literature_fixture(&script(body));
    let stage = stage_for(&catalog, &key, NativeProfile::NoteTraceDecision).unwrap();
    let input = stage.input_directory.as_ref().unwrap().path().to_owned();
    let executable = stage.directory.path().to_owned();
    let mut host = NativeHost::spawn_for(stage, &key, NativeProfile::NoteTraceDecision).unwrap();
    timeout(WAIT, host.initialize()).await.unwrap().unwrap();
    let service = LiteratureTraceService::new(Arc::clone(&catalog));
    *service.host.lock().await = Some(host);
    (root, service, key, input, executable)
}

async fn wait_for_request<F: Future<Output = NativeResult<DecidedTrace>> + Unpin>(
    pending: &mut F,
    directory: &Path,
) {
    timeout(WAIT,async{
        loop{
            tokio::select!{result=&mut *pending=>panic!("request finished early: {result:?}"),_=sleep(Duration::from_millis(5))=>{}}
            if directory.join("seen-request").exists(){break;}
        }
    }).await.unwrap();
}
fn inputs(directory: &Path) -> Vec<PathBuf> {
    fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("note-trace-")
        })
        .collect()
}

#[tokio::test]
async fn explicit_accept_refreshes_a_fresh_catalog_without_spawning_or_selecting_another_key() {
    let (root, _catalog, key) = literature_fixture(b"not an executable");
    let service = LiteratureTraceService::new(Arc::new(RuntimePluginCatalog::new(Some(
        root.path().into(),
    ))));
    service.accept(&key).await.unwrap();
    assert!(service.host.lock().await.is_none());
    let error = service.accept(&"b".repeat(64)).await.err().unwrap();
    assert_eq!(error.code, "plugin_host_package_unavailable");
    assert_eq!(error.plugin_id, "literature_library");
    assert_eq!(
        error.package_key.as_deref(),
        Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")
    );
    let (_root, catalog, key) = fixture(b"writing");
    let service = LiteratureTraceService::new(catalog);
    assert_eq!(
        service.accept(&key).await.err().unwrap().code,
        "plugin_host_profile_unsupported"
    );
}

#[tokio::test]
async fn domain_rejection_and_ready_clear_inputs_and_keep_the_same_healthy_child() {
    let body = format!("{READ}{REJECT}{READ}{READY}{SHUTDOWN}");
    let (_root, service, key, directory, executable) = warm(&body).await;
    let first = timeout(WAIT, service.decide(key.clone(), input()))
        .await
        .unwrap()
        .unwrap();
    let actual: serde_json::Value =
        serde_json::from_slice(&fs::read(directory.join("seen-identity")).unwrap()).unwrap();
    assert_eq!(first.instance_id, actual["instanceId"].as_str().unwrap());
    assert_eq!(first.request_id, actual["requestId"].as_str().unwrap());
    assert_eq!(
        first.decision,
        TraceDecision::Rejected {
            reason: TraceRejection::Expired
        }
    );
    assert!(inputs(&directory).is_empty());
    assert!(!service.host.lock().await.as_ref().unwrap().is_failed());
    let second = timeout(WAIT, service.decide(key, input()))
        .await
        .unwrap()
        .unwrap();
    let actual: serde_json::Value =
        serde_json::from_slice(&fs::read(directory.join("seen-identity")).unwrap()).unwrap();
    assert_eq!(second.instance_id, actual["instanceId"].as_str().unwrap());
    assert_eq!(second.request_id, actual["requestId"].as_str().unwrap());
    assert_eq!(first.instance_id, second.instance_id);
    assert_ne!(first.request_id, second.request_id);
    assert_eq!(
        second.decision,
        TraceDecision::Ready {
            mark_sent: MarkSentCondition::ORIGINAL
        }
    );
    assert!(inputs(&directory).is_empty());
    timeout(WAIT, service.shutdown()).await.unwrap().unwrap();
    assert!(!directory.exists());
    assert!(!executable.exists());
}

#[tokio::test]
async fn dropping_pending_request_removes_input_poisons_host_and_rejects_parallel_work() {
    let body = format!("{READ}: > seen-request\nIFS= read -r another\nexit 0");
    let (_root, service, key, directory, executable) = warm(&body).await;
    let mut pending = Box::pin(service.decide(key.clone(), input()));
    wait_for_request(&mut pending, &directory).await;
    assert_eq!(inputs(&directory).len(), 1);
    let busy = service.decide(key, input()).await.unwrap_err();
    assert_eq!(busy.code, "plugin_host_busy");
    assert_eq!(busy.plugin_id, "literature_library");
    drop(pending);
    assert!(inputs(&directory).is_empty());
    assert!(service.host.lock().await.as_ref().unwrap().is_failed());
    timeout(WAIT, service.shutdown()).await.unwrap().unwrap();
    assert!(!directory.exists());
    assert!(!executable.exists());
}

#[tokio::test]
async fn shutdown_cancels_pending_request_and_reaps_without_a_lost_notification() {
    let body = format!("{READ}: > seen-request\nIFS= read -r another\nexit 0");
    let (_root, service, key, directory, executable) = warm(&body).await;
    let mut pending = Box::pin(service.decide(key, input()));
    wait_for_request(&mut pending, &directory).await;
    let (result, shutdown) = timeout(WAIT, async { tokio::join!(pending, service.shutdown()) })
        .await
        .unwrap();
    assert_eq!(result.unwrap_err().code, "plugin_host_stopping");
    shutdown.unwrap();
    assert!(!directory.exists());
    assert!(!executable.exists());
}

#[tokio::test]
async fn valid_ready_with_failed_input_unlink_is_an_error_and_blocks_continuation() {
    use std::os::unix::fs::PermissionsExt;
    let body = format!("{READ}/bin/chmod 500 .\n{READY}IFS= read -r another\nexit 0");
    let (_root, service, key, directory, _executable) = warm(&body).await;
    let error = timeout(WAIT, service.decide(key.clone(), input()))
        .await
        .unwrap()
        .unwrap_err();
    if directory.exists() {
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
        fs::remove_dir_all(&directory).unwrap();
    }
    assert_eq!(error.code, "plugin_host_input_cleanup_failed");
    assert_eq!(error.plugin_id, "literature_library");
    assert!(error.instance_id.is_some() && error.request_id.is_some());
    assert!(
        error
            .cleanup_error
            .as_deref()
            .unwrap()
            .contains("PermissionDenied")
    );
    assert_eq!(
        service.decide(key, input()).await.unwrap_err().code,
        "plugin_host_cleanup_blocked"
    );
    assert_eq!(
        service.shutdown().await.unwrap_err().code,
        "plugin_host_cleanup_blocked"
    );
}
