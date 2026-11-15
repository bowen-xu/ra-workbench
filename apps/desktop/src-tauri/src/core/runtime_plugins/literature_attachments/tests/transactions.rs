use super::*;
use fixture::{PDF, text};
use rusqlite::{
    Connection,
    hooks::{AuthAction, AuthContext, Authorization},
};
use std::fs;

fn clear(db: &Connection) {
    db.authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
}

#[test]
fn failed_object_row_write_can_leave_a_file_without_a_publish_return_or_attachment() {
    use sha2::{Digest, Sha256};
    let f = Fixture::new();
    f.store
        .connection()
        .unwrap()
        .execute_batch(
            "create temp trigger synthetic_object_insert_failure before insert on content_objects
         begin select raise(abort,'private object fixture abort'); end;",
        )
        .unwrap();
    let source = f.source("failed-object.pdf", PDF);
    let failure = f
        .run(
            AttachmentRequest::Attach {
                item_id: "paper".into(),
                path: source.to_str().unwrap().into(),
            },
            None,
        )
        .unwrap_err();
    assert_eq!(failure.progress.phase, "content-publish");
    assert_eq!(failure.progress.sql, AttachmentSql::StatementFailed);
    assert!(!failure.progress.content_publish_returned);
    assert!(!failure.progress.attachment_insert_returned);
    assert!(failure.progress.attachment_id.is_none());
    assert!(f.rows("select id from attachments").is_empty());
    assert!(f.rows("select sha256 from content_objects").is_empty());
    let sha = format!("{:x}", Sha256::digest(PDF));
    let path = f
        .store
        .content_directory()
        .join(&sha[..2])
        .join(&sha[2..4])
        .join(format!("{sha}.pdf"));
    assert_eq!(fs::read(path).unwrap(), PDF);
    assert!(f.store.connection().unwrap().is_autocommit());
    let error = failure.native();
    assert!(error.message.contains("可能已写入"));
    assert!(error.message.contains("禁止自动重放"));
    assert!(
        !serde_json::to_string(&error)
            .unwrap()
            .contains("private object fixture abort")
    );
}

#[test]
fn failed_attachment_insert_preserves_original_autocommit_content_publication_prefix() {
    let f = Fixture::new();
    f.store
        .connection()
        .unwrap()
        .execute_batch(
            "create temp trigger synthetic_attachment_insert_failure before insert on attachments
         begin select raise(abort,'private attachment fixture abort'); end;",
        )
        .unwrap();
    let source = f.source("failed-insert.pdf", PDF);
    let failure = f
        .run(
            AttachmentRequest::Attach {
                item_id: "paper".into(),
                path: source.to_str().unwrap().into(),
            },
            None,
        )
        .unwrap_err();
    assert_eq!(failure.progress.phase, "attach-insert");
    assert_eq!(failure.progress.sql, AttachmentSql::StatementFailed);
    assert!(failure.progress.content_publish_returned);
    assert!(!failure.progress.attachment_insert_returned);
    assert_eq!(failure.progress.duplicate_insert_returns, 0);
    assert_eq!(
        failure.data.as_ref().unwrap().resource_id,
        "literature.attachments.insert"
    );
    assert!(failure.progress.attachment_id.is_some());
    assert!(f.rows("select id from attachments").is_empty());
    assert_eq!(
        f.rows("select count(*) from content_objects"),
        vec![vec![1_i64.into()]]
    );
    let relative = f.rows("select relative_path from content_objects");
    let rusqlite::types::Value::Text(relative) = &relative[0][0] else {
        panic!("object path");
    };
    assert_eq!(
        fs::read(f.store.content_directory().join(relative)).unwrap(),
        PDF
    );
    assert!(f.store.connection().unwrap().is_autocommit());
    assert!(
        !serde_json::to_string(&failure)
            .unwrap()
            .contains("private attachment fixture abort")
    );
    let prefix = f.prefix();
    assert_eq!(f.reopen().prefix(), prefix);
}

#[test]
fn failed_duplicate_insert_keeps_already_autocommitted_attachment_without_replaying() {
    let f = Fixture::new();
    f.attach("paper", "first.pdf", PDF);
    f.store.connection().unwrap().execute_batch(
        "create temp trigger synthetic_duplicate_insert_failure before insert on duplicate_candidates
         begin select raise(abort,'private duplicate fixture abort'); end;"
    ).unwrap();
    let source = f.source("second.pdf", PDF);
    let failure = f
        .run(
            AttachmentRequest::Attach {
                item_id: "other".into(),
                path: source.to_str().unwrap().into(),
            },
            None,
        )
        .unwrap_err();
    assert_eq!(failure.progress.phase, "attach-duplicate-insert");
    assert!(failure.progress.content_publish_returned);
    assert!(failure.progress.attachment_insert_returned);
    assert_eq!(failure.progress.duplicate_insert_returns, 0);
    assert_eq!(
        failure.data.as_ref().unwrap().resource_id,
        "literature.catalog-save.duplicate-insert"
    );
    let id = failure.progress.attachment_id.as_deref().unwrap();
    assert_eq!(
        f.rows("select id from attachments where item_id='other'"),
        vec![vec![text(id)]]
    );
    assert!(f.rows("select id from duplicate_candidates").is_empty());
    assert!(f.store.connection().unwrap().is_autocommit());
    let error = failure.native();
    assert!(error.message.contains("禁止自动重放"));
    assert!(
        !serde_json::to_string(&error)
            .unwrap()
            .contains("private duplicate fixture abort")
    );
    let prefix = f.prefix();
    assert_eq!(f.reopen().prefix(), prefix);
}

#[test]
fn remove_fts_failure_rolls_back_attachment_pages_job_and_keeps_physical_file() {
    let f = Fixture::new();
    let attachment = f.attach("paper", "source.pdf", PDF);
    f.run(
        AttachmentRequest::Index {
            item_id: "paper".into(),
            attachment_id: attachment.id.clone(),
            pages: vec![PageText {
                page_index: 0,
                text: "Preserved original FTS".into(),
            }],
        },
        None,
    )
    .unwrap();
    let prefix = f.prefix();
    let path = f.stored(&attachment.id);
    f.store
        .connection()
        .unwrap()
        .authorizer(Some(|context: AuthContext<'_>| {
            if matches!(
                context.action,
                AuthAction::Update {
                    table_name: "literature_search",
                    ..
                }
            ) {
                Authorization::Deny
            } else {
                Authorization::Allow
            }
        }));
    let failure = f.remove(&attachment.id).unwrap_err();
    clear(&f.store.connection().unwrap());
    assert_eq!(failure.progress.phase, "remove-fts");
    assert_eq!(failure.progress.sql, AttachmentSql::RolledBack);
    assert_eq!(
        failure.data.as_ref().unwrap().resource_id,
        "literature.attachments.remove-fts"
    );
    assert_eq!(f.prefix(), prefix);
    assert_eq!(fs::read(path).unwrap(), PDF);
    assert!(f.store.connection().unwrap().is_autocommit());
    assert_eq!(f.reopen().prefix(), prefix);
}

#[test]
fn completion_failure_rolls_back_pages_job_and_fts_after_original_transaction_body() {
    let f = Fixture::new();
    let attachment = f.attach("paper", "source.pdf", PDF);
    f.run(
        AttachmentRequest::Index {
            item_id: "paper".into(),
            attachment_id: attachment.id.clone(),
            pages: vec![PageText {
                page_index: 4,
                text: "Original completed prefix".into(),
            }],
        },
        None,
    )
    .unwrap();
    let prefix = f.prefix();
    f.store.connection().unwrap().execute_batch(
        "create temp trigger synthetic_index_completed_failure before update on full_text_index_jobs
         when new.state='completed' begin select raise(abort,'private completion fixture abort'); end;"
    ).unwrap();
    let failure = f
        .run(
            AttachmentRequest::Index {
                item_id: "paper".into(),
                attachment_id: attachment.id,
                pages: vec![PageText {
                    page_index: 0,
                    text: "Rolled back replacement".into(),
                }],
            },
            None,
        )
        .unwrap_err();
    assert_eq!(failure.progress.phase, "index-job-completed");
    assert_eq!(failure.progress.sql, AttachmentSql::RolledBack);
    assert_eq!(
        failure.data.as_ref().unwrap().resource_id,
        "literature.attachments.index-job-completed"
    );
    assert_eq!(f.prefix(), prefix);
    assert!(f.store.connection().unwrap().is_autocommit());
    assert_eq!(f.reopen().prefix(), prefix);
}
