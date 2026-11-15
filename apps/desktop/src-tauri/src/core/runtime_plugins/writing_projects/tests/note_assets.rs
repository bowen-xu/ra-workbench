//! Actual independent package, original asset identity and Core settlement.
use super::{
    files::{seed, store},
    *,
};
use crate::runtime_plugins::{
    literature_catalog::save::SaveOutcome,
    writing_projects::files::{
        FileRequest, FileResult,
        bibliography::{Call as BibliographyCall, project_transfer::commands::run_import},
        note_assets::Call,
        operate_files,
    },
};
use research_workbench_native_sdk::writing_note_asset::{MAX_BYTES, NoteAsset};
use std::{fs, path::PathBuf};

async fn call(f: &Fixture, store: &AppStore, id: &str, call: Call) -> NativeResult<FileResult> {
    operate_files(
        store.clone(),
        f.service.clone(),
        f.catalog.clone(),
        f.key.clone(),
        id.into(),
        FileRequest::NoteAsset { call },
    )
    .await
}
fn import(bytes: &[u8], name: &str, mime: &str) -> Call {
    Call::Import {
        display_name: name.into(),
        mime_type: mime.into(),
        bytes: bytes.into(),
    }
}
fn root(f: &Fixture, store: &AppStore) -> PathBuf {
    crate::scoped_text_files::legacy::manuscript_source_directory(
        store.manuscript_directory(),
        &f.active,
    )
    .unwrap()
}
fn count_assets(f: &Fixture, store: &AppStore) -> usize {
    let directory = root(f, store).join(".ranote-assets");
    if !directory.exists() {
        return 0;
    }
    fs::read_dir(directory).unwrap().count()
}
fn asset(result: FileResult) -> NoteAsset {
    let FileResult::NoteAsset(asset) = result else {
        panic!("asset receipt")
    };
    asset
}

#[tokio::test]
#[ignore = "requires the actual independently built writing v2 package"]
async fn actual_package_note_assets_keep_bytes_names_zip_identity_and_cold_reopen() {
    let f = Fixture::new();
    let store = store(&f);
    seed(
        &store,
        &f.active,
        "main.tex",
        "\\documentclass{article}\n\\begin{document}Asset test\\end{document}",
    );
    let before = crate::runtime_plugins::writing_projects::compile::snapshot::source_manifest_hash(
        &root(&f, &store),
    )
    .unwrap();
    let image_bytes = b"\xff\xd8\0synthetic jpeg\xff";
    let image = asset(
        call(
            &f,
            &store,
            &f.active,
            import(image_bytes, "original.jpeg", "image/jpeg"),
        )
        .await
        .unwrap(),
    );
    assert_eq!(image.display_name, format!("{}.jpg", image.asset_id));
    let pdf = asset(
        call(
            &f,
            &store,
            &f.active,
            import(b"%PDF-fixture", "paper.pdf", "application/pdf"),
        )
        .await
        .unwrap(),
    );
    assert_eq!(pdf.display_name, "paper.pdf");
    for (asset, expected) in [
        (&image, image_bytes.as_slice()),
        (&pdf, b"%PDF-fixture".as_slice()),
    ] {
        let FileResult::AssetBytes(bytes) = call(
            &f,
            &store,
            &f.active,
            Call::Read {
                asset_id: asset.asset_id.clone(),
            },
        )
        .await
        .unwrap() else {
            panic!("read receipt")
        };
        assert_eq!(bytes, expected);
        let indexed: (String, i64, String) = store
            .connection()
            .unwrap()
            .query_row(
                "select file_kind,byte_size,sha256 from manuscript_files
                 where manuscript_id=?1 and relative_path=?2",
                rusqlite::params![f.active, format!(".ranote-assets/{}", asset.asset_id)],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(
            indexed,
            (
                "other".into(),
                expected.len() as i64,
                crate::scoped_text_files::legacy::sha256(expected)
            )
        );
    }
    let note = r#"{"schemaVersion":1,"noteId":"ideas","blocks":[]}"#;
    seed(&store, &f.active, "notes/Ideas.ranote", note);
    assert_eq!(
        crate::runtime_plugins::writing_projects::compile::snapshot::source_manifest_hash(&root(
            &f, &store
        ))
        .unwrap(),
        before
    );
    let exported = f.directory.path().join("image.jpg");
    for suffix in ["image.jpg", "image (1).jpg"] {
        let FileResult::Exported(receipt) = call(
            &f,
            &store,
            &f.active,
            Call::Export {
                asset_id: image.asset_id.clone(),
                destination_path: exported.to_string_lossy().into(),
            },
        )
        .await
        .unwrap() else {
            panic!("export receipt")
        };
        assert_eq!(
            receipt.destination_path,
            f.directory.path().join(suffix).to_string_lossy()
        );
        assert_eq!(fs::read(&receipt.destination_path).unwrap(), image_bytes);
    }
    let zip = f.directory.path().join("notes.zip");
    let FileResult::Exported(export) = operate_files(
        store.clone(),
        f.service.clone(),
        f.catalog.clone(),
        f.key.clone(),
        f.active.clone(),
        FileRequest::ExportSource {
            destination: zip.to_string_lossy().into(),
            rename_on_conflict: false,
        },
    )
    .await
    .unwrap() else {
        panic!("zip receipt")
    };
    assert_eq!(export.file_count, 4);
    let imported = run_import(
        store.clone(),
        f.service.clone(),
        f.catalog.clone(),
        f.key.clone(),
        BibliographyCall::ImportProject {
            title: "Asset roundtrip".into(),
            source_path: zip.to_string_lossy().into(),
            requested_root: Some("main.tex".into()),
        },
    )
    .await
    .unwrap();
    let FileResult::AssetBytes(bytes) = call(
        &f,
        &store,
        &imported.manuscript.id,
        Call::Read {
            asset_id: image.asset_id.clone(),
        },
    )
    .await
    .unwrap() else {
        panic!("roundtrip bytes")
    };
    assert_eq!(bytes, image_bytes);
    let imported_root = crate::scoped_text_files::legacy::manuscript_source_directory(
        store.manuscript_directory(),
        &imported.manuscript.id,
    )
    .unwrap();
    assert_eq!(
        fs::read_to_string(imported_root.join("notes/Ideas.ranote")).unwrap(),
        note
    );
    assert!(
        call(
            &f,
            &store,
            &f.archived,
            Call::Read {
                asset_id: image.asset_id.clone()
            }
        )
        .await
        .is_err()
    );
    f.service.shutdown().await.unwrap();
    drop(store);
    let Fixture {
        directory,
        database,
        catalog,
        key,
        ..
    } = f;
    drop(database);
    let sql = rusqlite::Connection::open(directory.path().join("synthetic.sqlite3")).unwrap();
    let store = AppStore::for_test(sql, directory.path().join("objects"));
    let service = Arc::new(NativePreviewService::new(catalog.clone()));
    let FileResult::AssetBytes(bytes) = operate_files(
        store.clone(),
        service.clone(),
        catalog,
        key,
        imported.manuscript.id,
        FileRequest::NoteAsset {
            call: Call::Read {
                asset_id: image.asset_id,
            },
        },
    )
    .await
    .unwrap() else {
        panic!("reopened bytes")
    };
    assert_eq!(bytes, image_bytes);
    service.shutdown().await.unwrap();
}

#[tokio::test]
#[ignore = "requires the actual independently built writing v2 package"]
async fn actual_package_note_assets_preserve_error_order_limits_and_scoped_paths() {
    let f = Fixture::new();
    let store = store(&f);
    seed(&store, &f.active, "main.tex", "Original");
    for (id, request, code) in [
        (
            "missing",
            import(&vec![0; MAX_BYTES + 1], "../bad", "bad/type"),
            "not_found",
        ),
        (
            f.active.as_str(),
            import(&vec![0; MAX_BYTES + 1], "../bad", "bad/type"),
            "compile_process_error",
        ),
        (
            f.active.as_str(),
            import(b"", "../bad", "bad/type"),
            "invalid_input",
        ),
        (
            f.active.as_str(),
            import(b"", "a.svg", "image/svg+xml"),
            "invalid_input",
        ),
    ] {
        let error = call(&f, &store, id, request).await.unwrap_err();
        assert_eq!(error.code, code);
        assert!(error.request_id.is_some());
        assert!(error.instance_id.is_some());
        assert_eq!(count_assets(&f, &store), 0);
    }
    let error = call(
        &f,
        &store,
        &format!(" {} ", f.active),
        import(b"a", "a", "text/plain"),
    )
    .await
    .unwrap_err();
    assert_eq!(error.code, "not_found");
    let error = call(
        &f,
        &store,
        &f.active,
        Call::Read {
            asset_id: "../main.tex".into(),
        },
    )
    .await
    .unwrap_err();
    assert_eq!(error.code, "invalid_input");
    let big = asset(
        call(
            &f,
            &store,
            &f.active,
            import(&vec![7; MAX_BYTES], "large.bin", "application/octet-stream"),
        )
        .await
        .unwrap(),
    );
    let path = root(&f, &store).join(".ranote-assets").join(&big.asset_id);
    fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_len(MAX_BYTES as u64 + 1)
        .unwrap();
    assert_eq!(
        call(
            &f,
            &store,
            &f.active,
            Call::Read {
                asset_id: big.asset_id.clone()
            }
        )
        .await
        .unwrap_err()
        .code,
        "file_too_large"
    );
    fs::remove_file(&path).unwrap();
    let outside = f.directory.path().join("outside");
    fs::write(&outside, "keep").unwrap();
    std::os::unix::fs::symlink(&outside, &path).unwrap();
    assert_eq!(
        call(
            &f,
            &store,
            &f.active,
            Call::Read {
                asset_id: big.asset_id.clone()
            }
        )
        .await
        .unwrap_err()
        .code,
        "unsafe_path"
    );
    fs::remove_file(&path).unwrap();
    fs::remove_dir(path.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(f.directory.path(), path.parent().unwrap()).unwrap();
    assert_eq!(
        call(&f, &store, &f.active, import(b"new", "a.txt", "text/plain"))
            .await
            .unwrap_err()
            .code,
        "unsafe_path"
    );
    assert_eq!(fs::read_to_string(outside).unwrap(), "keep");
    f.service.shutdown().await.unwrap();
}

#[tokio::test]
#[ignore = "requires the actual independently built writing v2 package"]
async fn actual_package_note_asset_index_failure_rolls_back_and_removes_new_bytes() {
    let f = Fixture::new();
    let store = store(&f);
    seed(&store, &f.active, "main.tex", "Original");
    store.connection().unwrap().execute_batch("create trigger reject_asset after insert on manuscript_files when new.file_kind='other' begin select raise(fail,'synthetic asset failure'); end;").unwrap();
    let error = call(
        &f,
        &store,
        &f.active,
        import(b"asset", "a.txt", "text/plain"),
    )
    .await
    .unwrap_err();
    assert_eq!(error.code, "storage_error");
    assert!(matches!(error.save_outcome, Some(SaveOutcome::RolledBack)));
    assert_eq!(count_assets(&f, &store), 0);
    assert_eq!(
        store
            .connection()
            .unwrap()
            .query_row(
                "select count(*) from manuscript_files where file_kind='other'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    store
        .connection()
        .unwrap()
        .execute_batch("drop trigger reject_asset")
        .unwrap();
    let saved = asset(
        call(
            &f,
            &store,
            &f.active,
            import(b"retry", "a.txt", "text/plain"),
        )
        .await
        .unwrap(),
    );
    assert_eq!(saved.display_name, "a.txt");
    assert_eq!(count_assets(&f, &store), 1);
    f.service.shutdown().await.unwrap();
}

#[tokio::test]
#[ignore = "requires the actual independently built writing v2 package"]
async fn actual_package_note_asset_settles_once_when_the_waiter_is_cancelled() {
    let f = Fixture::new();
    let store = store(&f);
    seed(&store, &f.active, "main.tex", "Original");
    let gate = Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
    let worker_gate = gate.clone();
    let (reached, waiting) = tokio::sync::oneshot::channel::<()>();
    let reached = std::sync::Mutex::new(Some(reached));
    {
        let db = store.connection().unwrap();
        db.create_scalar_function(
            "asset_index_pause",
            0,
            rusqlite::functions::FunctionFlags::SQLITE_UTF8,
            move |_| {
                if let Some(reached) = reached.lock().unwrap().take() {
                    reached.send(()).unwrap();
                    let (released, signal) = &*worker_gate;
                    let mut released = released.lock().unwrap();
                    while !*released {
                        released = signal.wait(released).unwrap();
                    }
                }
                Ok(0)
            },
        )
        .unwrap();
        db.execute_batch("create trigger pause_asset after insert on manuscript_files when new.file_kind='other' begin select asset_index_pause(); end;").unwrap();
    }
    let worker_store = store.clone();
    let service = f.service.clone();
    let catalog = f.catalog.clone();
    let key = f.key.clone();
    let id = f.active.clone();
    let task = tokio::spawn(async move {
        operate_files(
            worker_store,
            service,
            catalog,
            key,
            id,
            FileRequest::NoteAsset {
                call: import(b"owned", "a.txt", "text/plain"),
            },
        )
        .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(30), waiting)
        .await
        .unwrap()
        .unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    let (released, signal) = &*gate;
    *released.lock().unwrap() = true;
    signal.notify_all();
    tokio::time::timeout(std::time::Duration::from_secs(30), f.service.shutdown())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(count_assets(&f, &store), 1);
    assert_eq!(
        store
            .connection()
            .unwrap()
            .query_row(
                "select count(*) from manuscript_files where file_kind='other'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
}
