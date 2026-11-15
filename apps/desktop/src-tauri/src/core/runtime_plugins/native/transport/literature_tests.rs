use super::*;
use crate::runtime_plugins::native::package::{stage_for, tests::literature_fixture};

const INITIALIZE: &str = r#"IFS= read -r line
identity=${line%\"body\":*}
printf '%s"body":{"kind":"initialized","capability":"literature.note-trace-decision","version":1}}\n' "$identity"
"#;
fn script(body: &str) -> Vec<u8> {
    format!("#!/bin/sh\n{INITIALIZE}{body}\n").into_bytes()
}
fn input() -> literature::SnapshotInput {
    literature::SnapshotInput {
        query: "q".into(),
        row: None,
    }
}

#[tokio::test]
async fn literature_timeout_clears_sidecar_and_keeps_actual_identity_and_owned_reap() {
    let body = r#"IFS= read -r line
identity=${line%\"body\":*}
printf '%s"marker":true}' "$identity" > seen-identity
IFS= read -r another
"#;
    let (_root, catalog, key) = literature_fixture(&script(body));
    let stage = stage_for(&catalog, &key, NativeProfile::NoteTraceDecision).unwrap();
    let directory = stage.input_directory.as_ref().unwrap().path().to_owned();
    let executable = stage.directory.path().to_owned();
    let mut host = NativeHost::spawn_for(stage, &key, NativeProfile::NoteTraceDecision).unwrap();
    host.initialize().await.unwrap();
    let mut error = host
        .decide_note_trace(input(), Duration::from_millis(250))
        .await
        .unwrap_err();
    assert_eq!(error.code, "plugin_host_timeout");
    assert_eq!(error.plugin_id, "literature_library");
    assert!(error.instance_id.is_some() && error.request_id.is_some());
    let actual: serde_json::Value =
        serde_json::from_slice(&std::fs::read(directory.join("seen-identity")).unwrap()).unwrap();
    assert_eq!(error.instance_id.as_deref(), actual["instanceId"].as_str());
    assert_eq!(error.request_id.as_deref(), actual["requestId"].as_str());
    assert_eq!(fs_entries(&directory), 0);
    host.finish_failed(&mut error).await;
    assert!(error.cleanup_error.is_none());
    assert!(host.process.exit().is_some());
    drop(host);
    assert!(!directory.exists());
    assert!(!executable.exists());
}

#[tokio::test]
async fn malformed_zero_row_ready_poisoning_still_cleans_the_input() {
    let body = r#"IFS= read -r line
identity=${line%\"body\":*}
printf '%s"body":{"kind":"noteTraceDecision","decision":{"status":"ready","markSent":{"expectedRows":0,"onMismatch":"already_consumed"}}}}\n' "$identity"
IFS= read -r another
"#;
    let (_root, catalog, key) = literature_fixture(&script(body));
    let stage = stage_for(&catalog, &key, NativeProfile::NoteTraceDecision).unwrap();
    let directory = stage.input_directory.as_ref().unwrap().path().to_owned();
    let mut host = NativeHost::spawn_for(stage, &key, NativeProfile::NoteTraceDecision).unwrap();
    host.initialize().await.unwrap();
    let mut error = host
        .decide_note_trace(input(), Duration::from_secs(2))
        .await
        .unwrap_err();
    assert_eq!(error.code, "plugin_host_protocol_invalid");
    assert_eq!(fs_entries(&directory), 0);
    assert!(host.is_failed());
    host.finish_failed(&mut error).await;
    assert!(error.cleanup_error.is_none());
    assert!(host.process.exit().is_some());
}

fn fs_entries(path: &std::path::Path) -> usize {
    std::fs::read_dir(path)
        .unwrap()
        .filter(|entry| {
            entry
                .as_ref()
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("note-trace-")
        })
        .count()
}
