use super::*;
use fixture::{PDF, text};
use sha2::{Digest, Sha256};
use std::fs;

#[test]
fn attach_keeps_original_header_hash_metadata_and_duplicate_order_without_transaction() {
    let f = Fixture::new();
    let before = f.prefix();
    let invalid = f.source("invalid.pdf", b"Not a PDF, even though the suffix says PDF");
    let error = f
        .run(
            AttachmentRequest::Attach {
                item_id: "absent".into(),
                path: invalid.to_str().unwrap().into(),
            },
            None,
        )
        .unwrap_err();
    assert_eq!(error.code, "not_found");
    assert_eq!(error.progress.phase, "attach-active-parent");
    let error = f
        .run(
            AttachmentRequest::Attach {
                item_id: "paper".into(),
                path: invalid.to_str().unwrap().into(),
            },
            None,
        )
        .unwrap_err();
    assert_eq!(error.code, "invalid_input");
    assert_eq!(error.progress.phase, "attach-probe");
    assert_eq!(f.prefix(), before);

    let attachment = f.attach("paper", "中文原稿.pdf", PDF);
    let hash = format!("{:x}", Sha256::digest(PDF));
    assert_eq!(attachment.content_sha256, hash);
    assert_eq!(attachment.byte_size, PDF.len() as u64);
    assert_eq!(attachment.mime_type, "application/pdf");
    assert_eq!(attachment.display_name, "中文原稿.pdf");
    uuid::Uuid::parse_str(&attachment.id).unwrap();
    assert_eq!(fs::read(f.stored(&attachment.id)).unwrap(), PDF);
    let source = f.directory.path().join("中文原稿.pdf");
    assert_eq!(
        f.rows("select source_path from attachments")[0][0],
        rusqlite::types::Value::Text(source.to_str().unwrap().into())
    );
    let prefix = f.prefix();
    let copy = f.source("same-bytes-new-name.pdf", PDF);
    let error = f
        .run(
            AttachmentRequest::Attach {
                item_id: "paper".into(),
                path: copy.to_str().unwrap().into(),
            },
            None,
        )
        .unwrap_err();
    assert_eq!(error.code, "attachment_duplicate");
    assert!(!error.progress.content_publish_returned);
    assert_eq!(f.prefix(), prefix);
    let other = f.attach("other", "same-content.pdf", PDF);
    assert_eq!(other.content_sha256, attachment.content_sha256);
    assert_eq!(
        f.rows("select left_item_id,right_item_id,reason,confidence from duplicate_candidates"),
        vec![vec![
            text("other"),
            text("paper"),
            text("content_hash"),
            1.0.into()
        ]]
    );
    assert!(f.store.connection().unwrap().is_autocommit());
    let prefix = f.prefix();
    assert_eq!(f.reopen().prefix(), prefix);
}

#[test]
fn rename_consumes_prepared_name_and_changes_only_original_metadata() {
    let f = Fixture::new();
    let attachment = f.attach("paper", "source.pdf", PDF);
    let path = f.stored(&attachment.id);
    let mut request = RenameAttachmentInputV1 {
        item_id: "paper".into(),
        attachment_id: attachment.id.clone(),
        display_name: "  中文新名.pdf  ".into(),
    };
    let rename = |request: &RenameAttachmentInputV1| AttachmentRequest::Rename {
        item_id: request.item_id.clone(),
        attachment_id: request.attachment_id.clone(),
        display_name: request.display_name.clone(),
    };
    f.run(
        rename(&request),
        Some(AttachmentPlanV1::Rename {
            request: request.clone(),
            display_name: "中文新名.pdf".into(),
        }),
    )
    .unwrap();
    assert_eq!(
        f.rows("select display_name,content_sha256 from attachments"),
        vec![vec![
            text("中文新名.pdf"),
            attachment.content_sha256.clone().into()
        ]]
    );
    assert_eq!(f.stored(&attachment.id), path);
    assert_eq!(fs::read(path).unwrap(), PDF);
    let prefix = f.prefix();
    request.item_id = "other".into();
    let error = f
        .run(
            rename(&request),
            Some(AttachmentPlanV1::Rename {
                request: request.clone(),
                display_name: "unused.pdf".into(),
            }),
        )
        .unwrap_err();
    assert_eq!(error.code, "not_found");
    assert_eq!(f.prefix(), prefix);
}

#[test]
fn read_keeps_raw_bytes_and_the_resolved_read_has_no_database_owner() {
    let f = Fixture::new();
    let attachment = f.attach("paper", "source.pdf", PDF);
    let stored = f.stored(&attachment.id);
    let binary = [0, 255, 42, 0, 17];
    // Ordinary reads never introduce PDF-header, hash or size revalidation.
    fs::write(&stored, binary).unwrap();
    assert_eq!(f.read(&attachment.id).unwrap(), binary);
    let _content = f.store.content_read().unwrap();
    let read = {
        let db = f.store.connection().unwrap();
        let relative: String = db
            .query_row("select relative_path from content_objects", [], |r| {
                r.get(0)
            })
            .unwrap();
        crate::content::StoredObjectRead::resolve(f.store.content_directory(), &relative).unwrap()
    };
    // Acquire the actual paired owner again before consuming the owned read.
    // This requires the first guard to have been dropped rather than retained
    // by the resolved byte adapter; no replacement file IO is involved.
    let db = f.store.connection().unwrap();
    assert!(db.is_autocommit());
    assert_eq!(read.read().unwrap(), binary);
    drop(db);
    fs::remove_file(stored).unwrap();
    assert_eq!(f.read(&attachment.id).unwrap_err().code, "not_found");
}

#[cfg(unix)]
#[test]
fn read_allows_internal_links_but_rejects_outside_links_and_parent_paths() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    let attachment = f.attach("paper", "source.pdf", PDF);
    let stored = f.stored(&attachment.id);
    let internal = f.store.content_directory().join("internal.bin");
    fs::write(&internal, [0, 255, 17]).unwrap();
    fs::remove_file(&stored).unwrap();
    symlink(&internal, &stored).unwrap();
    assert_eq!(f.read(&attachment.id).unwrap(), [0, 255, 17]);
    fs::remove_file(&stored).unwrap();
    let outside = f.source("outside.bin", b"outside");
    symlink(&outside, &stored).unwrap();
    assert_eq!(f.read(&attachment.id).unwrap_err().code, "storage_error");
    f.store
        .connection()
        .unwrap()
        .execute(
            "update content_objects set relative_path='../outside.bin'",
            [],
        )
        .unwrap();
    assert_eq!(f.read(&attachment.id).unwrap_err().code, "storage_error");
    assert_eq!(fs::read(outside).unwrap(), b"outside");
}
