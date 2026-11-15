use super::*;
use crate::runtime_plugins::native::package::{stage_for, tests::literature_fixture};
use research_workbench_native_sdk::catalog::CatalogRequest;
use sha2::{Digest, Sha256};
use std::{fs, os::unix::fs::symlink};

fn payload(bytes: &[u8]) -> literature::SnapshotRef {
    let request = literature::Request {
        protocol_version: 2,
        plugin_id: "literature_library".into(),
        package_key: "a".repeat(64),
        instance_id: uuid::Uuid::new_v4().to_string(),
        request_id: uuid::Uuid::new_v4().to_string(),
        body: literature::RequestBody::Initialize,
    };
    literature::SnapshotRef::catalog_result(
        &request,
        bytes.len() as u64,
        format!("{:x}", Sha256::digest(bytes)),
    )
}

#[test]
fn reads_complete_result_and_rejects_wrong_hash_size_trailing_data_and_shape() {
    let directory = tempfile::tempdir().unwrap();
    let bytes = br#"{"kind":"selection","ids":[" raw ","raw"," raw "]}"#;
    let reference = payload(bytes);
    fs::write(directory.path().join(&reference.relative_handle), bytes).unwrap();
    assert_eq!(
        read_result(directory.path(), &reference).unwrap(),
        CatalogOutput::Selection {
            ids: vec![" raw ".into(), "raw".into(), " raw ".into()],
        }
    );
    let mut wrong = reference.clone();
    wrong.sha256 = "0".repeat(64);
    assert_eq!(
        read_result(directory.path(), &wrong).unwrap_err().code,
        "plugin_host_payload_invalid"
    );
    wrong = reference.clone();
    wrong.byte_size += 1;
    assert!(read_result(directory.path(), &wrong).is_err());
    for invalid in [
        b"{}".as_slice(),
        b"{\"kind\":\"selection\",\"ids\":[],\"sql\":\"PRIVATE_BODY\"}",
        b"{} {}",
    ] {
        let reference = payload(invalid);
        fs::write(directory.path().join(&reference.relative_handle), invalid).unwrap();
        let error = read_result(directory.path(), &reference).unwrap_err();
        let diagnostic = serde_json::to_string(&error).unwrap();
        assert!(!diagnostic.contains("PRIVATE_BODY"));
        assert!(!diagnostic.contains(directory.path().to_str().unwrap()));
    }
}

#[test]
fn result_open_rejects_symlink_directory_and_fifo_without_following_or_blocking() {
    let directory = tempfile::tempdir().unwrap();
    let bytes = br#"{"kind":"selection","ids":[]}"#;
    let reference = payload(bytes);
    let target = directory.path().join(&reference.relative_handle);
    let private = directory.path().join("unrelated");
    fs::write(&private, bytes).unwrap();
    symlink(&private, &target).unwrap();
    assert!(read_result(directory.path(), &reference).is_err());
    fs::remove_file(&target).unwrap();
    fs::create_dir(&target).unwrap();
    assert!(read_result(directory.path(), &reference).is_err());
    fs::remove_dir(&target).unwrap();
    nix::unistd::mkfifo(
        &target,
        nix::sys::stat::Mode::S_IRUSR | nix::sys::stat::Mode::S_IWUSR,
    )
    .unwrap();
    assert!(read_result(directory.path(), &reference).is_err());
    assert_eq!(fs::read(private).unwrap(), bytes);
}

#[tokio::test]
async fn timed_out_catalog_retains_result_cleanup_name_until_confirmed_child_exit() {
    let script = br#"#!/bin/sh
IFS= read -r line
identity=${line%\"body\":*}
printf '%s"body":{"kind":"initialized","capability":"literature.note-trace-decision","version":1}}\n' "$identity"
IFS= read -r line
: > seen-request
IFS= read -r another
"#;
    let (_root, catalog, key) = literature_fixture(script);
    let staged = stage_for(&catalog, &key, NativeProfile::NoteTraceDecision).unwrap();
    let directory = staged.input_directory.as_ref().unwrap().path().to_owned();
    let mut host = NativeHost::spawn_for(staged, &key, NativeProfile::NoteTraceDecision).unwrap();
    host.initialize().await.unwrap();
    let mut error = host
        .catalog_call(
            CatalogInput::Plan {
                request: CatalogRequest::List { query: None },
            },
            Duration::from_millis(100),
        )
        .await
        .unwrap_err();
    assert_eq!(error.code, "plugin_host_timeout");
    assert!(host.payload_inflight && host.input_lease.is_some());
    // poison requests SIGKILL immediately. Simulate an in-flight filesystem
    // create completing after that request, before Core confirms the reap.
    // A child cannot be expected to keep running after SIGKILL in this test.
    let result_name = literature::catalog_result_handle(host.last_request_id.as_ref().unwrap());
    fs::write(directory.join(result_name), b"late result").unwrap();
    assert_eq!(catalog_files(&directory), 2);
    host.finish_failed(&mut error).await;
    assert!(host.process.exit().is_some());
    assert!(error.cleanup_error.is_none());
    assert_eq!(catalog_files(&directory), 0);
    assert!(!host.payload_inflight);
    drop(host);
    assert!(!directory.exists());
}

fn catalog_files(directory: &std::path::Path) -> usize {
    fs::read_dir(directory)
        .unwrap()
        .filter(|entry| {
            entry
                .as_ref()
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("catalog-")
        })
        .count()
}
