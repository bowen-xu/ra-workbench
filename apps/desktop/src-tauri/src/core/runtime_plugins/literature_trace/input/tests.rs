#![cfg(unix)]
use super::*;
use research_workbench_native_sdk::literature_trace_protocol::{self as protocol, RequestBody};
use sha2::{Digest, Sha256};
use std::os::unix::fs::PermissionsExt;

fn request() -> Request {
    Request {
        protocol_version: 2,
        plugin_id: "literature_library".into(),
        package_key: "a".repeat(64),
        instance_id: "8cbfd428-3ac1-4bce-b4f6-d41f31063d8e".into(),
        request_id: uuid::Uuid::new_v4().to_string(),
        body: RequestBody::Initialize,
    }
}

#[test]
fn lease_writes_complete_readonly_input_with_real_digest_and_drops_only_its_file() {
    let directory = tempfile::tempdir().unwrap();
    let request = request();
    let input = SnapshotInput {
        query: " raw\t问题\0 ".into(),
        row: None,
    };
    let cleanup = Arc::new(InputCleanup::default());
    let (lease, reference) =
        InputLease::stage(directory.path(), &request, &input, Arc::clone(&cleanup)).unwrap();
    assert_eq!(
        reference.relative_handle,
        protocol::snapshot_handle(&request.request_id)
    );
    assert_eq!(reference.package_key, request.package_key);
    assert_eq!(reference.instance_id, request.instance_id);
    assert_eq!(reference.request_id, request.request_id);
    let path = directory.path().join(&reference.relative_handle);
    let bytes = fs::read(&path).unwrap();
    assert_eq!(reference.byte_size, bytes.len() as u64);
    assert_eq!(reference.sha256, format!("{:x}", Sha256::digest(&bytes)));
    assert_eq!(
        serde_json::from_slice::<SnapshotInput>(&bytes).unwrap(),
        input
    );
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o400
    );
    fs::write(directory.path().join("unrelated"), b"preserve").unwrap();
    drop(lease);
    assert!(!path.exists());
    assert!(directory.path().join("unrelated").exists());
    assert!(cleanup.failure().is_none());
}

#[test]
fn failed_create_new_never_removes_existing_input_or_exposes_its_path() {
    let directory = tempfile::tempdir().unwrap();
    let request = request();
    let path = directory
        .path()
        .join(protocol::snapshot_handle(&request.request_id));
    fs::write(&path, b"PRIVATE_ORIGINAL").unwrap();
    let result = InputLease::stage(
        directory.path(),
        &request,
        &SnapshotInput {
            query: "PRIVATE_QUERY".into(),
            row: None,
        },
        Arc::default(),
    );
    let error = match result {
        Err(error) => error,
        Ok(_) => panic!("collision accepted"),
    };
    assert_eq!(error.code, "plugin_host_snapshot_stage_failed");
    assert_eq!(error.plugin_id, "literature_library");
    let serialized = serde_json::to_string(&error).unwrap();
    assert!(!serialized.contains("PRIVATE_"));
    assert!(!serialized.contains(directory.path().to_str().unwrap()));
    assert_eq!(fs::read(path).unwrap(), b"PRIVATE_ORIGINAL");
}

#[test]
fn unlink_failure_latches_safely_even_when_later_cleanup_can_remove_the_file() {
    let directory = tempfile::tempdir().unwrap();
    let request = request();
    let cleanup = Arc::new(InputCleanup::default());
    let (mut lease, reference) = InputLease::stage(
        directory.path(),
        &request,
        &SnapshotInput {
            query: "q".into(),
            row: None,
        },
        Arc::clone(&cleanup),
    )
    .unwrap();
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o500)).unwrap();
    let result = lease.clear();
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(result, Err(io::ErrorKind::PermissionDenied));
    let cause = cleanup.failure().unwrap();
    assert!(cause.contains("PermissionDenied"));
    assert!(!cause.contains(directory.path().to_str().unwrap()));
    lease.clear().unwrap();
    assert!(!directory.path().join(reference.relative_handle).exists());
    assert!(cleanup.failure().is_some());
}
