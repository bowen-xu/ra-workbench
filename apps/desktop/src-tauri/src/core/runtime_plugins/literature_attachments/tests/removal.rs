use super::*;
use fixture::{PDF, text};
use std::fs;

#[test]
fn remove_deletes_only_target_and_source_survives_with_same_database_reopen() {
    let f = Fixture::new();
    let first = f.attach("paper", "source.pdf", PDF);
    let second = f.attach("paper", "other.pdf", b"%PDF-1.7\nDifferent synthetic bytes");
    let path = f.stored(&first.id);
    let prefix = f.prefix();
    let failure = f
        .run(
            AttachmentRequest::Remove {
                item_id: "other".into(),
                attachment_id: first.id.clone(),
            },
            None,
        )
        .unwrap_err();
    assert_eq!(failure.code, "not_found");
    assert_eq!(f.prefix(), prefix);
    let result = f.remove(&first.id).unwrap();
    assert_eq!(
        serde_json::to_value(result).unwrap(),
        serde_json::json!({"fileStatus":"deleted","warning":null})
    );
    assert!(!path.exists());
    assert!(f.directory.path().join("source.pdf").exists());
    assert!(f.stored(&second.id).exists());
    assert_eq!(
        f.rows("select id from attachments"),
        vec![vec![text(&second.id)]]
    );
    assert_eq!(
        f.rows("select count(*) from content_objects"),
        vec![vec![1_i64.into()]]
    );
    let prefix = f.prefix();
    assert_eq!(f.reopen().prefix(), prefix);
}

#[test]
fn remove_keeps_shared_and_session_owned_content_but_missing_file_still_deletes_record() {
    for retention in ["shared", "session", "missing", "missing-root"] {
        let f = Fixture::new();
        let attachment = f.attach("paper", "source.pdf", PDF);
        let path = f.stored(&attachment.id);
        if retention == "shared" {
            f.attach("other", "copy.pdf", PDF);
        }
        if retention == "session" {
            f.store
                .retain_content_for_session(&attachment.content_sha256)
                .unwrap();
        }
        if retention == "missing" {
            fs::remove_file(&path).unwrap();
        }
        if retention == "missing-root" {
            fs::remove_dir_all(f.store.content_directory()).unwrap();
        }
        let result = f.remove(&attachment.id).unwrap();
        assert_eq!(
            result.file_status,
            if retention.starts_with("missing") {
                "missing"
            } else {
                "retained"
            }
        );
        assert!(result.warning.is_none());
        assert_eq!(
            f.rows("select count(*) from attachments where item_id='paper'"),
            vec![vec![0_i64.into()]]
        );
        assert_eq!(path.exists(), !retention.starts_with("missing"));
        assert_eq!(
            f.rows("select count(*) from content_objects"),
            vec![vec![i64::from(!retention.starts_with("missing")).into()]]
        );
        assert!(f.store.connection().unwrap().is_autocommit());
    }
}

#[test]
fn protected_context_reference_retains_attachment_file_fts_and_job_after_original_rollback() {
    let f = Fixture::new();
    let attachment = f.attach("paper", "source.pdf", PDF);
    {
        let db = f.store.connection().unwrap();
        db.execute("insert into docling_parse_runs(id,item_id,attachment_id,content_sha256,service_base_url,docling_status,parse_status,raw_response,created_at,finished_at) values('parse','paper',?1,?2,'http://127.0.0.1','done','success','{}','now','now')", rusqlite::params![attachment.id, attachment.content_sha256]).unwrap();
        db.execute("insert into paper_context_snapshots(id,paper_id,context_hash,context_schema_version,created_at) values('snapshot','paper',?1,1,'now')", ["c".repeat(64)]).unwrap();
        db.execute("insert into paper_context_attachments(snapshot_id,ordinal,attachment_id,content_sha256,parse_run_id) values('snapshot',0,?1,?2,'parse')", rusqlite::params![attachment.id, attachment.content_sha256]).unwrap();
    }
    let prefix = f.prefix();
    let path = f.stored(&attachment.id);
    let failure = f.remove(&attachment.id).unwrap_err();
    assert_eq!(failure.code, "attachment_in_use");
    assert_eq!(failure.progress.sql, AttachmentSql::RolledBack);
    assert_eq!(failure.progress.phase, "remove-delete");
    assert!(failure.rollback.is_none());
    assert_eq!(f.prefix(), prefix);
    assert_eq!(fs::read(path).unwrap(), PDF);
    assert_eq!(f.reopen().prefix(), prefix);
}

#[cfg(unix)]
#[test]
fn remove_is_stricter_than_read_for_internal_symlinks_and_preserves_both_targets() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    let attachment = f.attach("paper", "source.pdf", PDF);
    let stored = f.stored(&attachment.id);
    let other = f.store.content_directory().join("another.bin");
    fs::write(&other, b"keep actual internal target").unwrap();
    fs::remove_file(&stored).unwrap();
    symlink(&other, &stored).unwrap();
    let prefix = f.prefix();
    assert_eq!(
        f.read(&attachment.id).unwrap(),
        b"keep actual internal target"
    );
    let failure = f.remove(&attachment.id).unwrap_err();
    assert_eq!(failure.code, "storage_error");
    assert_eq!(failure.progress.phase, "remove-path");
    assert_eq!(failure.progress.sql, AttachmentSql::NotStarted);
    assert_eq!(f.prefix(), prefix);
    assert_eq!(fs::read(other).unwrap(), b"keep actual internal target");
    assert!(
        fs::symlink_metadata(stored)
            .unwrap()
            .file_type()
            .is_symlink()
    );
}
