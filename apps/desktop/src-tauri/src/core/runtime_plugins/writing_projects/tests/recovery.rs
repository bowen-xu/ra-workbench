use super::*;
use crate::runtime_plugins::{
    literature_catalog::save::SaveOutcome,
    writing_projects::recovery::{clear_draft, list_drafts, save_draft},
};
use research_workbench_native_sdk::writing_project::RecoveryDraftInput;
use rusqlite::hooks::{AuthAction, AuthContext, Authorization};

fn draft(id: &str, path: &str) -> RecoveryDraftInput {
    RecoveryDraftInput {
        manuscript_id: id.into(),
        relative_path: path.into(),
        base_sha256: "z".repeat(64),
        content: "正文\r\n".into(),
    }
}

async fn save(
    fixture: &Fixture,
    input: RecoveryDraftInput,
) -> NativeResult<research_workbench_native_sdk::writing_project::RecoveryDraft> {
    save_draft(
        Arc::clone(&fixture.database),
        Arc::clone(&fixture.service),
        Arc::clone(&fixture.catalog),
        fixture.key.clone(),
        input,
    )
    .await
}

#[tokio::test]
#[ignore = "requires an independently built writing v2 package; run the explicit C05 package acceptance"]
async fn actual_package_recovery_save_applies_original_expiry_and_per_manuscript_global_bounds() {
    let fixture = Fixture::new();
    {
        let db = fixture.database.lock().unwrap();
        for (id, count) in [(&fixture.active, 40), (&fixture.archived, 140)] {
            for index in 0..count {
                db.execute("insert into manuscript_recovery_drafts(manuscript_id,relative_path,base_sha256,content,updated_at) values(?1,?2,?3,'old',strftime('%Y-%m-%dT%H:%M:%fZ','now',?4))",
                    rusqlite::params![id,format!("old-{index:03}.tex"),"a".repeat(64),if id==&fixture.active {"-1 day"} else {"-2 days"}]).unwrap();
            }
        }
        db.execute("insert into manuscript_recovery_drafts(manuscript_id,relative_path,base_sha256,content,updated_at) values(?1,'expired.tex',?2,'expired',strftime('%Y-%m-%dT%H:%M:%fZ','now','-31 days'))",rusqlite::params![fixture.archived,"a".repeat(64)]).unwrap();
    }
    let original = draft(&fixture.active, "notes/草稿.tex");
    let saved = save(&fixture, original.clone()).await.unwrap();
    assert_eq!(saved.manuscript_id, original.manuscript_id);
    assert_eq!(saved.relative_path, original.relative_path);
    assert_eq!(saved.base_sha256, original.base_sha256);
    assert_eq!(saved.content, original.content);
    let rows = list_drafts(
        Arc::clone(&fixture.database),
        &fixture.service,
        Arc::clone(&fixture.catalog),
        fixture.key.clone(),
        fixture.active.clone(),
    )
    .await
    .unwrap();
    assert_eq!(rows.len(), 32);
    assert_eq!(rows[0], saved);
    let db = fixture.database.lock().unwrap();
    assert_eq!(
        db.query_row(
            "select count(*) from manuscript_recovery_drafts",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        128
    );
    assert_eq!(
        db.query_row(
            "select count(*) from manuscript_recovery_drafts where relative_path='expired.tex'",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    drop(db);
    fixture.service.shutdown().await.unwrap();
}

#[tokio::test]
#[ignore = "requires an independently built writing v2 package; run the explicit C05 package acceptance"]
async fn actual_package_recovery_preserves_missing_path_hash_fk_and_clear_semantics() {
    let fixture = Fixture::new();
    let missing = save(&fixture, draft("missing", "../bad.tex"))
        .await
        .unwrap_err();
    assert_eq!(missing.code, "not_found");
    assert_eq!(missing.message, "稿件不存在");
    let invalid = save(&fixture, draft(&fixture.active, "../bad.tex"))
        .await
        .unwrap_err();
    assert_eq!(invalid.code, "invalid_input");
    assert_eq!(invalid.message, "稿件路径无效");
    let padded = save(
        &fixture,
        draft(&format!(" {} ", fixture.active), "main.tex"),
    )
    .await
    .unwrap_err();
    assert_eq!(padded.code, "storage_error");
    assert_eq!(
        padded.data_error.unwrap().resource_id,
        "writing.projects.draft-upsert"
    );
    let mut unicode_hash = draft(&fixture.active, "main.tex");
    unicode_hash.base_sha256 = "é".repeat(32);
    let rejected = save(&fixture, unicode_hash).await.unwrap_err();
    assert_eq!(rejected.code, "storage_error");
    assert_eq!(
        rejected.data_error.unwrap().sqlite_code.as_deref(),
        Some("ConstraintViolation")
    );
    save(&fixture, draft(&fixture.active, "main.tex"))
        .await
        .unwrap();
    fixture
        .database
        .lock()
        .unwrap()
        .authorizer(Some(|context: AuthContext<'_>| match context.action {
            // DELETE's foreign-key check reads manuscripts.id. The original
            // project projection also reads title, which clear must not query.
            AuthAction::Read {
                table_name: "manuscripts",
                column_name: "title",
            } => Authorization::Deny,
            _ => Authorization::Allow,
        }));
    assert!(
        fixture
            .database
            .lock()
            .unwrap()
            .prepare("select title from manuscripts")
            .is_err()
    );
    clear_draft(
        Arc::clone(&fixture.database),
        Arc::clone(&fixture.service),
        Arc::clone(&fixture.catalog),
        fixture.key.clone(),
        format!(" {} ", fixture.active),
        " main.tex ".into(),
    )
    .await
    .unwrap();
    clear_draft(
        Arc::clone(&fixture.database),
        Arc::clone(&fixture.service),
        Arc::clone(&fixture.catalog),
        fixture.key.clone(),
        "missing".into(),
        "main.tex".into(),
    )
    .await
    .unwrap();
    let bad_clear = clear_draft(
        Arc::clone(&fixture.database),
        Arc::clone(&fixture.service),
        Arc::clone(&fixture.catalog),
        fixture.key.clone(),
        "missing".into(),
        "../bad.tex".into(),
    )
    .await
    .unwrap_err();
    assert_eq!(bad_clear.code, "invalid_input");
    fixture
        .database
        .lock()
        .unwrap()
        .authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
    assert!(
        list_drafts(
            Arc::clone(&fixture.database),
            &fixture.service,
            Arc::clone(&fixture.catalog),
            fixture.key.clone(),
            fixture.active.clone()
        )
        .await
        .unwrap()
        .is_empty()
    );
    fixture.service.shutdown().await.unwrap();
}

#[tokio::test]
#[ignore = "requires an independently built writing v2 package; run the explicit C05 package acceptance"]
async fn actual_package_recovery_cleanup_failure_preserves_saved_prefix_and_cold_reopen() {
    let fixture = Fixture::new();
    fixture.database.lock().unwrap().execute("insert into manuscript_recovery_drafts(manuscript_id,relative_path,base_sha256,content,updated_at) values(?1,'expired.tex',?2,'old',strftime('%Y-%m-%dT%H:%M:%fZ','now','-31 days'))",rusqlite::params![fixture.archived,"a".repeat(64)]).unwrap();
    fixture.database.lock().unwrap().execute_batch("create table synthetic_recovery_prefix(value text not null);
        create temp trigger test_recovery_expire_fail before delete on manuscript_recovery_drafts
        begin insert into synthetic_recovery_prefix values('prefix');select raise(fail,'PRIVATE_RECOVERY_FAILURE');end;").unwrap();
    let original = draft(&fixture.active, "main.tex");
    let error = save(&fixture, original.clone()).await.unwrap_err();
    assert_eq!(error.code, "storage_error");
    assert_eq!(error.save_outcome, Some(SaveOutcome::Unknown));
    assert_eq!(
        error.data_error.unwrap().resource_id,
        "writing.projects.draft-expire"
    );
    assert!(error.instance_id.is_some() && error.request_id.is_some());
    assert!(!error.message.contains("PRIVATE_RECOVERY_FAILURE"));
    fixture.service.shutdown().await.unwrap();
    fixture.database.shutdown().unwrap();
    drop(fixture.database);
    let sql =
        rusqlite::Connection::open(fixture.directory.path().join("synthetic.sqlite3")).unwrap();
    let reopened = Database::open(sql, &fixture.directory.path().join("data-log")).unwrap();
    let service = NativePreviewService::new(Arc::clone(&fixture.catalog));
    let drafts = list_drafts(
        Arc::clone(&reopened),
        &service,
        fixture.catalog,
        fixture.key,
        fixture.active,
    )
    .await
    .unwrap();
    assert_eq!(drafts.len(), 1);
    assert_eq!(drafts[0].relative_path, original.relative_path);
    assert_eq!(drafts[0].content, original.content);
    assert_eq!(
        reopened
            .lock()
            .unwrap()
            .query_row(
                "select count(*) from synthetic_recovery_prefix",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
    service.shutdown().await.unwrap();
}
