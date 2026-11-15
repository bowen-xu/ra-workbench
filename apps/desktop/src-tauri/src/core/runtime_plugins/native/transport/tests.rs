use super::*;
use crate::runtime_plugins::native::package::{stage, tests::fixture};

const INITIALIZE: &str = r#"IFS= read -r line
identity=${line%\"body\":*}
printf '%s"body":{"kind":"initialized","capability":"writing.import-preview","version":1}}\n' "$identity"
"#;

fn script(body: &str) -> Vec<u8> {
    format!("#!/bin/sh\n{INITIALIZE}{body}\n").into_bytes()
}

#[tokio::test]
async fn shutdown_rejects_output_written_only_after_the_valid_acknowledgement() {
    let (_root, catalog, key) = fixture(&script(
        r#"
IFS= read -r line
identity=${line%\"body\":*}
printf '%s"body":{"kind":"shutdownComplete"}}\n' "$identity"
IFS= read -r eof
printf 'unassociated late stdout\n'
# Keep the faulting root owned and alive until the parent terminates it. This
# controls the failure state without an arbitrary sleep or a reap/signal race.
kill -STOP "$$"
"#,
    ));
    let mut host = NativeHost::spawn(stage(&catalog, &key).unwrap(), &key).unwrap();
    host.initialize().await.unwrap();
    let error = host.shutdown().await.unwrap_err();
    assert_eq!(error.code, "plugin_host_protocol_invalid");
    assert!(error.cleanup_error.is_none(), "{error:?}");
    assert_eq!(error.signal, Some(9));
}

#[tokio::test]
async fn owned_transport_handshakes_and_shuts_down_without_inheriting_environment() {
    let (_root, catalog, key) = fixture(&script(
        r#"
IFS= read -r line
identity=${line%\"body\":*}
printf '%s"body":{"kind":"previewImportResult","outcome":{"status":"success","preview":{"fileCount":1,"byteSize":3,"rootCandidates":["main.tex"]}}}}\n' "$identity"
IFS= read -r line
identity=${line%\"body\":*}
printf '%s"body":{"kind":"shutdownComplete"}}\n' "$identity"
"#,
    ));
    let mut host = NativeHost::spawn(stage(&catalog, &key).unwrap(), &key).unwrap();
    host.initialize().await.unwrap();
    let response = host
        .request(
            RequestBody::PreviewImport {
                source_path: "/synthetic".into(),
            },
            Duration::from_secs(2),
        )
        .await
        .unwrap();
    assert!(matches!(response, ResponseBody::PreviewImportResult { .. }));
    host.shutdown().await.unwrap();
    assert_eq!(host.process.exit().unwrap().code, Some(0));
}

#[tokio::test]
async fn abnormal_exit_preserves_actual_exit_and_request_identities() {
    let (_root, catalog, key) = fixture(&script("IFS= read -r line\nexit 41"));
    let mut host = NativeHost::spawn(stage(&catalog, &key).unwrap(), &key).unwrap();
    host.initialize().await.unwrap();
    let mut error = host
        .request(
            RequestBody::PreviewImport {
                source_path: "/synthetic".into(),
            },
            Duration::from_secs(2),
        )
        .await
        .unwrap_err();
    host.finish_failed(&mut error).await;
    assert_eq!(error.code, "plugin_host_exited");
    assert_eq!(error.exit_code, Some(41));
    assert!(error.instance_id.is_some() && error.request_id.is_some());
    assert!(error.cleanup_error.is_none());
}

#[tokio::test]
async fn timeout_poisoning_reaps_only_the_owned_host_and_never_reuses_the_connection() {
    let (_root, catalog, key) = fixture(&script("IFS= read -r line\nIFS= read -r another"));
    let mut host = NativeHost::spawn(stage(&catalog, &key).unwrap(), &key).unwrap();
    host.initialize().await.unwrap();
    let mut error = host
        .request(
            RequestBody::PreviewImport {
                source_path: "/synthetic".into(),
            },
            Duration::from_millis(100),
        )
        .await
        .unwrap_err();
    assert_eq!(error.code, "plugin_host_timeout");
    host.finish_failed(&mut error).await;
    assert!(error.cleanup_error.is_none());
    assert!(host.process.exit().is_some());
    assert!(host.is_failed());
    assert_eq!(
        host.request(RequestBody::Shutdown, Duration::from_secs(2))
            .await
            .unwrap_err()
            .code,
        "plugin_host_connection_failed"
    );
}

#[tokio::test]
async fn mismatched_instance_cannot_complete_the_matching_request() {
    let body = r#"IFS= read -r line
printf '%s\n' '{"protocolVersion":1,"pluginId":"writing_projects","packageKey":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","instanceId":"00000000-0000-0000-0000-000000000000","requestId":"00000000-0000-0000-0000-000000000000","body":{"kind":"previewImportResult","outcome":{"status":"success","preview":{"fileCount":0,"byteSize":0,"rootCandidates":[]}}}}'
"#;
    let (_root, catalog, key) = fixture(&script(body));
    let mut host = NativeHost::spawn(stage(&catalog, &key).unwrap(), &key).unwrap();
    host.initialize().await.unwrap();
    let mut error = host
        .request(
            RequestBody::PreviewImport {
                source_path: "/synthetic".into(),
            },
            Duration::from_secs(2),
        )
        .await
        .unwrap_err();
    host.finish_failed(&mut error).await;
    assert_eq!(error.code, "plugin_host_response_mismatch");
    assert!(host.is_failed());
}
