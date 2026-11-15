use super::*;
use crate::runtime_plugins::{
    literature_catalog::save::SaveOutcome, writing_projects::creation::create_project,
};
use research_workbench_native_sdk::writing_project::{ManuscriptDraft, STANDALONE_PROJECT_ID};

pub(super) async fn create(
    fixture: &Fixture,
    id: uuid::Uuid,
    project: Option<&str>,
    title: &str,
) -> NativeResult<Manuscript> {
    create_project(
        Arc::clone(&fixture.database),
        fixture.directory.path().join("manuscripts"),
        Arc::clone(&fixture.service),
        Arc::clone(&fixture.catalog),
        fixture.key.clone(),
        ManuscriptDraft {
            project_id: project.map(str::to_owned),
            title: title.into(),
            root_path: Some("sections/main.tex".into()),
        },
        id,
    )
    .await
}

fn assert_absent(fixture: &Fixture, id: uuid::Uuid) {
    let db = fixture.database.lock().unwrap();
    let count: i64 = db
        .query_row(
            "select count(*) from manuscripts where id=?1",
            [id.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
    assert!(
        !fixture
            .directory
            .path()
            .join("manuscripts")
            .join(id.to_string())
            .join("source")
            .exists()
    );
}

#[tokio::test]
#[ignore = "requires an independently built writing v2 package; run the explicit C05 package acceptance"]
async fn actual_package_creates_original_template_index_and_project_identity() {
    let fixture = Fixture::new();
    let id = uuid::Uuid::new_v4();
    let created = create(&fixture, id, None, "  A {研究}  ").await.unwrap();
    assert_eq!(created.id, id.to_string());
    assert_eq!(created.project_id, STANDALONE_PROJECT_ID);
    assert_eq!(created.title, "A {研究}");
    assert_eq!(created.root_path, "sections/main.tex");
    assert_eq!(created.status, "active");
    assert_eq!(created.created_at, created.updated_at);
    let bytes = std::fs::read(
        fixture
            .directory
            .path()
            .join("manuscripts")
            .join(&created.id)
            .join("source/sections/main.tex"),
    )
    .unwrap();
    assert_eq!(bytes, "\\documentclass{article}\n\\usepackage[utf8]{inputenc}\n\\title{A 研究}\n\\begin{document}\n\\maketitle\n\n\\bibliographystyle{plain}\n\\bibliography{references}\n\\end{document}\n".as_bytes());
    {
        let db = fixture.database.lock().unwrap();
        let indexed: (String, String, i64, String) = db.query_row(
            "select relative_path,file_kind,byte_size,sha256 from manuscript_files where manuscript_id=?1", [&created.id],
            |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?)),
        ).unwrap();
        assert_eq!(indexed.0, created.root_path);
        assert_eq!(indexed.1, "tex");
        assert_eq!(indexed.2, bytes.len() as i64);
        assert_eq!(indexed.3, crate::scoped_text_files::legacy::sha256(&bytes));
    }
    let existing = create(
        &fixture,
        uuid::Uuid::new_v4(),
        Some(" synthetic "),
        "Explicit",
    )
    .await
    .unwrap();
    assert_eq!(existing.project_id, "synthetic");
    fixture.service.shutdown().await.unwrap();
    fixture.database.shutdown().unwrap();
    drop(fixture.database);
    let sql =
        rusqlite::Connection::open(fixture.directory.path().join("synthetic.sqlite3")).unwrap();
    let reopened = Database::open(sql, &fixture.directory.path().join("data-log")).unwrap();
    let service = NativePreviewService::new(Arc::clone(&fixture.catalog));
    let reloaded = get_project(
        reopened,
        &service,
        fixture.catalog,
        fixture.key,
        created.id.clone(),
    )
    .await
    .unwrap();
    assert_eq!(reloaded.id, created.id);
    assert_eq!(reloaded.created_at, created.created_at);
    assert_eq!(reloaded.root_path, created.root_path);
    assert_eq!(reloaded.title, created.title);
    service.shutdown().await.unwrap();
}

#[tokio::test]
#[ignore = "requires an independently built writing v2 package; run the explicit C05 package acceptance"]
async fn actual_package_creation_preserves_project_and_sql_failure_before_metadata() {
    let fixture = Fixture::new();
    let missing_id = uuid::Uuid::new_v4();
    let missing = create(&fixture, missing_id, Some("missing"), "")
        .await
        .unwrap_err();
    assert_eq!(missing.code, "not_found");
    assert_eq!(missing.message, "研究项目不存在");
    assert_eq!(missing.operation_id, Some(missing_id.to_string()));
    assert_absent(&fixture, missing_id);
    let invalid_id = uuid::Uuid::new_v4();
    let invalid = create(&fixture, invalid_id, None, "").await.unwrap_err();
    assert_eq!(invalid.code, "invalid_input");
    assert_eq!(invalid.message, "稿件信息无效");
    assert_eq!(invalid.save_outcome, Some(SaveOutcome::RolledBack));
    assert_absent(&fixture, invalid_id);
    {
        let db = fixture.database.lock().unwrap();
        let defaults: i64 = db
            .query_row(
                "select count(*) from research_projects where id=?1",
                [STANDALONE_PROJECT_ID],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            defaults, 0,
            "default project must roll back with invalid metadata"
        );
        db.execute_batch("create temp trigger test_writing_default_abort before insert on research_projects when new.id='__standalone_writing__' begin select raise(abort,'PRIVATE_DEFAULT_FAILURE');end;").unwrap();
    }
    let failed_id = uuid::Uuid::new_v4();
    let sql = create(&fixture, failed_id, None, "").await.unwrap_err();
    assert_eq!(sql.code, "storage_error");
    assert_eq!(
        sql.data_error.unwrap().resource_id,
        "writing.projects.standalone-upsert"
    );
    assert_eq!(sql.save_outcome, Some(SaveOutcome::RolledBack));
    assert!(!sql.message.contains("PRIVATE_DEFAULT_FAILURE"));
    assert!(sql.instance_id.is_some() && sql.request_id.is_some());
    assert_absent(&fixture, failed_id);
    fixture.service.shutdown().await.unwrap();
}

#[tokio::test]
#[ignore = "requires an independently built writing v2 package; run the explicit C05 package acceptance"]
async fn actual_package_creation_cleans_template_on_original_index_and_commit_failures() {
    let fixture = Fixture::new();
    let blocked_id = uuid::Uuid::new_v4();
    let blocked_source = fixture
        .directory
        .path()
        .join("manuscripts")
        .join(blocked_id.to_string())
        .join("source");
    std::fs::create_dir_all(blocked_source.parent().unwrap()).unwrap();
    std::fs::write(&blocked_source, b"original file").unwrap();
    let blocked = create(&fixture, blocked_id, Some("synthetic"), "File failure")
        .await
        .unwrap_err();
    assert_eq!(blocked.code, "storage_error");
    assert!(blocked.message.starts_with("稿件文件操作失败："));
    assert_eq!(blocked.save_outcome, Some(SaveOutcome::RolledBack));
    assert_eq!(std::fs::read(blocked_source).unwrap(), b"original file");
    assert_eq!(
        fixture
            .database
            .lock()
            .unwrap()
            .query_row(
                "select count(*) from manuscripts where id=?1",
                [blocked_id.to_string()],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        0
    );
    fixture.database.lock().unwrap().execute_batch("create temp trigger test_writing_index_abort before insert on manuscript_files begin select raise(abort,'PRIVATE_INDEX_FAILURE');end;").unwrap();
    let indexed_id = uuid::Uuid::new_v4();
    let index = create(&fixture, indexed_id, Some("synthetic"), "Index failure")
        .await
        .unwrap_err();
    assert_eq!(index.code, "storage_error");
    assert_eq!(
        index.data_error.unwrap().resource_id,
        "writing.projects.file-upsert"
    );
    assert_eq!(index.save_outcome, Some(SaveOutcome::RolledBack));
    assert_absent(&fixture, indexed_id);
    fixture.database.lock().unwrap().execute_batch("drop trigger test_writing_index_abort;
        create table synthetic_deferred_writing_failure(project_id text references research_projects(id) deferrable initially deferred);
        create temp trigger test_writing_commit_abort after insert on manuscripts begin insert into synthetic_deferred_writing_failure values('missing');end;").unwrap();
    let commit_id = uuid::Uuid::new_v4();
    let commit = create(&fixture, commit_id, Some("synthetic"), "Commit failure")
        .await
        .unwrap_err();
    assert_eq!(commit.code, "storage_error");
    assert_eq!(commit.save_database_error.unwrap().phase, "commit");
    assert_eq!(commit.save_outcome, Some(SaveOutcome::RolledBack));
    assert_absent(&fixture, commit_id);
    let db = fixture.database.lock().unwrap();
    assert_eq!(
        db.query_row(
            "select count(*) from synthetic_deferred_writing_failure",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    drop(db);
    fixture.service.shutdown().await.unwrap();
}
