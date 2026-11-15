use super::*;
use super::{
    file_save::note,
    files::{seed, store},
};
use crate::runtime_plugins::writing_projects::files::{
    catalog_preparation::prepare_catalog, catalog_query::query_catalog,
};
use research_workbench_native_sdk::{
    note_catalog_query::{PageCatalogQuery, PageCatalogSort},
    writing_catalog::PreparationScope,
    writing_catalog_query::CatalogPage,
};
use rusqlite::hooks::{AuthAction, AuthContext, Authorization, TransactionOperation};
use sha2::Digest;
fn request() -> PageCatalogQuery {
    PageCatalogQuery {
        category: "all".into(),
        view: "recent".into(),
        limit: Some(2),
        cursor: None,
        search: None,
        sort: None,
    }
}
async fn query(
    fixture: &Fixture,
    store: &AppStore,
    request: PageCatalogQuery,
) -> NativeResult<CatalogPage> {
    query_catalog(
        store.clone(),
        Arc::clone(&fixture.service),
        Arc::clone(&fixture.catalog),
        fixture.key.clone(),
        request,
    )
    .await
}
async fn ready(fixture: &Fixture, store: &AppStore) {
    for (id, path, text) in [
        (&fixture.active, "notes/标题2.ranote", "RareBody 研究 alpha"),
        (&fixture.active, "notes/标题10.ranote", "RareBody 研究 beta"),
        (&fixture.active, "notes/标题1.ranote", "RareBody 研究 gamma"),
        (
            &fixture.archived,
            "archived.ranote",
            "RareBody 研究 archive",
        ),
    ] {
        seed(store, id, path, &note(path, text));
        store.connection().unwrap().execute("update manuscript_files set modified_at='2026-11-09T00:00:00.000Z' where manuscript_id=?1 and relative_path=?2",rusqlite::params![id,path]).unwrap();
    }
    let prepared = prepare_catalog(
        store.clone(),
        Arc::clone(&fixture.service),
        Arc::clone(&fixture.catalog),
        fixture.key.clone(),
        false,
        PreparationScope::Live,
    )
    .await
    .unwrap();
    assert_eq!(prepared.processed_count, 4);
}
#[tokio::test]
#[ignore = "requires an independently built writing v2 package; run the explicit C05 package acceptance"]
async fn actual_package_queries_all_original_sort_seek_scopes_without_file_reads() {
    let fixture = Fixture::new();
    let store = store(&fixture);
    ready(&fixture, &store).await;
    std::fs::remove_dir_all(store.manuscript_directory()).unwrap();
    for field in ["title", "source", "category", "updatedAt"] {
        for direction in ["ascending", "descending"] {
            let mut q = request();
            q.sort = Some(PageCatalogSort {
                field: field.into(),
                direction: direction.into(),
            });
            let first = query(&fixture, &store, q.clone()).await.unwrap();
            // Compare the independently built host with the original Core hash.
            let cursor: serde_json::Value =
                serde_json::from_str(first.next_cursor.as_ref().unwrap()).unwrap();
            let scope = serde_json::json!([q.category, q.view, q.sort, Vec::<String>::new()]);
            assert_eq!(
                cursor["scope"],
                format!("{:x}", sha2::Sha256::digest(scope.to_string().as_bytes()))
            );
            assert_eq!(first.items.len(), 2);
            assert_eq!(first.pending_count, 0);
            assert_eq!(first.error_count, 0);
            if field == "title" {
                assert_eq!(
                    first.items[0].title,
                    if direction == "ascending" {
                        "标题1"
                    } else {
                        "标题10"
                    }
                );
            }
            q.cursor = first.next_cursor.clone();
            assert!(q.cursor.is_some());
            let second = query(&fixture, &store, q).await.unwrap();
            assert_eq!(second.items.len(), 1);
            assert!(second.next_cursor.is_none());
            assert_eq!(first.revision, second.revision);
            assert!(first.items.iter().all(|item| item.id != second.items[0].id));
            let item = &first.items[0];
            assert_eq!(item.category, "dependent");
            assert_eq!(item.owner.id, fixture.active);
            assert_eq!(item.target.kind, "writing");
            assert!(!item.is_favorite);
            assert!(item.snippet.is_empty());
        }
    }
    let mut q = request();
    q.category = "independent".into();
    assert!(
        query(&fixture, &store, q.clone())
            .await
            .unwrap()
            .items
            .is_empty()
    );
    q.search = Some("RareBody 研究".into());
    q.limit = Some(200);
    let searched = query(&fixture, &store, q.clone()).await.unwrap();
    assert_eq!(searched.items.len(), 4);
    assert!(
        searched
            .items
            .iter()
            .all(|item| item.snippet.contains("RareBody"))
    );
    q.view = "trash".into();
    assert!(query(&fixture, &store, q).await.unwrap().items.is_empty());
    let mut archived = request();
    archived.view = "archive".into();
    let result = query(&fixture, &store, archived).await.unwrap();
    assert_eq!(result.items.len(), 1);
    assert_eq!(result.items[0].target.manuscript_id, fixture.archived);
    fixture.service.shutdown().await.unwrap();
}
#[tokio::test]
#[ignore = "requires an independently built writing v2 package; run the explicit C05 package acceptance"]
async fn actual_package_query_preserves_cursor_scope_generation_and_sql_error_precedence() {
    let fixture = Fixture::new();
    let store = store(&fixture);
    ready(&fixture, &store).await;
    let first = query(&fixture, &store, request()).await.unwrap();
    let mut q = request();
    q.cursor = first.next_cursor.clone();
    q.view = "archive".into();
    assert!(
        query(&fixture, &store, q.clone())
            .await
            .unwrap_err()
            .message
            .contains("cursor_scope")
    );
    q.view = "recent".into();
    fixture
        .database
        .lock()
        .unwrap()
        .execute(
            "update writing_note_catalog_state set generation=generation+1 where id=1",
            [],
        )
        .unwrap();
    let error = query(&fixture, &store, q).await.unwrap_err();
    assert_eq!(error.code, "revision_conflict");
    assert_eq!(error.current_revision, Some(first.revision + 1));
    let mut malformed = request();
    malformed.cursor = Some("{".into());
    let error = query(&fixture, &store, malformed.clone())
        .await
        .unwrap_err();
    assert!(error.message.contains("cursor_invalid"));
    fixture
        .database
        .lock()
        .unwrap()
        .authorizer(Some(|ctx: AuthContext<'_>| match ctx.action {
            AuthAction::Read {
                table_name: "writing_note_catalog_state",
                ..
            } => Authorization::Deny,
            _ => Authorization::Allow,
        }));
    let error = query(&fixture, &store, malformed).await.unwrap_err();
    assert_eq!(error.code, "storage_error");
    assert!(error.query_database_error.is_some());
    assert!(!error.message.contains("cursor_invalid"));
    fixture
        .database
        .lock()
        .unwrap()
        .authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
    assert!(fixture.database.lock().unwrap().is_autocommit());
    fixture.service.shutdown().await.unwrap();
}
#[tokio::test]
#[ignore = "requires an independently built writing v2 package; run the explicit C05 package acceptance"]
async fn actual_package_query_keeps_version_timestamp_counts_and_missing_cursor_boundaries() {
    let fixture = Fixture::new();
    let store = store(&fixture);
    ready(&fixture, &store).await;
    let original_cursor = query(&fixture, &store, request())
        .await
        .unwrap()
        .next_cursor
        .unwrap();
    {
        let db = fixture.database.lock().unwrap();
        db.execute(
            "update writing_note_catalog_state set projection_version='other' where id=1",
            [],
        )
        .unwrap();
        db.execute(
            "update writing_note_catalog set updated_ms=null where manuscript_id=?1",
            [&fixture.active],
        )
        .unwrap();
    }
    assert!(
        query(&fixture, &store, request())
            .await
            .unwrap_err()
            .message
            .contains("catalog_version")
    );
    fixture.database.lock().unwrap().execute("update writing_note_catalog_state set projection_version='writing-note-text/v1' where id=1",[]).unwrap();
    let error = query(&fixture, &store, request()).await.unwrap_err();
    assert!(error.message.contains("timestamp_invalid"));
    assert!(error.message.contains(&fixture.active));
    {
        let db = fixture.database.lock().unwrap();
        db.execute(
            "update writing_note_catalog set updated_ms=1794182400000",
            [],
        )
        .unwrap();
        db.execute("update writing_note_catalog set state='error',error_cause='synthetic issue' where manuscript_id=?1 and relative_path='notes/标题1.ranote'",[&fixture.active]).unwrap();
    }
    let page = query(&fixture, &store, request()).await.unwrap();
    assert_eq!(page.error_count, 1);
    assert_eq!(page.issues.len(), 1);
    assert_eq!(page.issues[0].cause, "synthetic issue");
    let mut q = request();
    let mut cursor: serde_json::Value = serde_json::from_str(&original_cursor).unwrap();
    cursor["revision"] = serde_json::json!(page.revision);
    cursor["id"] = serde_json::json!("missing");
    q.cursor = Some(cursor.to_string());
    assert!(
        query(&fixture, &store, q)
            .await
            .unwrap_err()
            .message
            .contains("cursor_missing")
    );
    fixture.service.shutdown().await.unwrap();
}

#[tokio::test]
#[ignore = "requires an independently built writing v2 package; run the explicit C05 package acceptance"]
async fn actual_package_query_binds_all_thirty_two_literal_terms_without_sql_interpolation() {
    let fixture = Fixture::new();
    let store = store(&fixture);
    let mut terms = (0..30)
        .map(|index| format!("term{index:02}"))
        .collect::<Vec<_>>();
    terms.extend(["a".into(), "quoted\"term".into()]);
    seed(
        &store,
        &fixture.active,
        "search.ranote",
        &note("search-note", &terms.join(" ")),
    );
    store
        .connection()
        .unwrap()
        .execute(
            "update manuscript_files set modified_at='2026-11-09T00:00:00.000Z'",
            [],
        )
        .unwrap();
    prepare_catalog(
        store.clone(),
        Arc::clone(&fixture.service),
        Arc::clone(&fixture.catalog),
        fixture.key.clone(),
        false,
        PreparationScope::Live,
    )
    .await
    .unwrap();
    let mut q = request();
    q.search = Some(terms.join(" "));
    let page = query(&fixture, &store, q.clone()).await.unwrap();
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].target.path, "search.ranote");
    terms[29] = "missingterm".into();
    q.search = Some(terms.join(" "));
    assert!(
        query(&fixture, &store, q.clone())
            .await
            .unwrap()
            .items
            .is_empty()
    );
    terms.push("thirtythird".into());
    q.search = Some(terms.join(" "));
    assert_eq!(
        query(&fixture, &store, q).await.unwrap_err().code,
        "invalid_input"
    );
    fixture.service.shutdown().await.unwrap();
}

#[tokio::test]
#[ignore = "requires an independently built writing v2 package; run the explicit C05 package acceptance"]
async fn actual_package_query_commit_failure_rolls_back_before_releasing_database() {
    let fixture = Fixture::new();
    let store = store(&fixture);
    fixture
        .database
        .lock()
        .unwrap()
        .authorizer(Some(|ctx: AuthContext<'_>| match ctx.action {
            AuthAction::Transaction {
                // rusqlite 0.37 maps SQLITE_TRANSACTION's COMMIT to Unknown.
                // BEGIN and ROLLBACK have their own named variants.
                operation: TransactionOperation::Unknown,
            } => Authorization::Deny,
            _ => Authorization::Allow,
        }));
    let error = query(&fixture, &store, request()).await.unwrap_err();
    assert_eq!(error.code, "storage_error");
    assert_eq!(
        error.query_database_error.unwrap().phase,
        "catalog_query_commit"
    );
    assert!(error.query_rollback_error.is_none());
    assert!(error.instance_id.is_some());
    assert!(error.request_id.is_some());
    assert!(error.operation_id.is_some());
    let db = fixture.database.lock().unwrap();
    assert!(db.is_autocommit());
    db.authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
    drop(db);
    query(&fixture, &store, request()).await.unwrap();
    fixture.service.shutdown().await.unwrap();
}

#[tokio::test]
#[ignore = "requires an independently built writing v2 package; run the explicit C05 package acceptance"]
async fn actual_package_query_unconfirmed_read_rollback_fences_the_original_database() {
    let fixture = Fixture::new();
    let store = store(&fixture);
    fixture
        .database
        .lock()
        .unwrap()
        .authorizer(Some(|ctx: AuthContext<'_>| match ctx.action {
            AuthAction::Transaction {
                operation: TransactionOperation::Rollback,
            } => Authorization::Deny,
            _ => Authorization::Allow,
        }));
    let mut q = request();
    q.cursor = Some("{".into());
    let error = query(&fixture, &store, q).await.unwrap_err();
    assert_eq!(error.code, "storage_error");
    assert_eq!(
        error.query_rollback_error.unwrap().phase,
        "catalog_query_rollback"
    );
    assert!(!error.message.contains("cursor_invalid"));
    assert!(error.instance_id.is_some());
    assert!(error.request_id.is_some());
    assert!(fixture.database.lock().is_err());
    assert!(store.connection().is_err());
    let reopened =
        rusqlite::Connection::open(fixture.directory.path().join("synthetic.sqlite3")).unwrap();
    assert_eq!(
        reopened
            .query_row("select count(*) from manuscripts", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        2
    );
    assert_eq!(
        reopened
            .query_row(
                "select generation from writing_note_catalog_state",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    fixture.service.shutdown().await.unwrap();
}

#[test]
fn catalog_query_wire_keeps_original_request_fields_and_rejects_sql_or_unread_rows() {
    use research_workbench_native_sdk::writing_catalog_query::{QueryInput, QuerySnapshot};
    let input = QueryInput::Prepare { query: request() };
    let mut value = serde_json::to_value(&input).unwrap();
    assert_eq!(
        serde_json::from_value::<QueryInput>(value.clone()).unwrap(),
        input
    );
    value["sql"] = serde_json::json!("select synthetic");
    assert!(serde_json::from_value::<QueryInput>(value).is_err());
    assert!(
        serde_json::from_value::<QuerySnapshot>(
            serde_json::json!({"revision":1,"read":{"stage":"cursorBoundary","items":{"rows":[]}}})
        )
        .is_err()
    );
    let page = CatalogPage {
        items: vec![],
        revision: 1,
        next_cursor: None,
        pending_count: 0,
        error_count: 0,
        issues: vec![],
    };
    assert_eq!(
        serde_json::to_value(page).unwrap(),
        serde_json::json!({"items":[],"revision":1,"pendingCount":0,"errorCount":0,"issues":[]})
    );
}
