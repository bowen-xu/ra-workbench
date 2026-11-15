use super::*;
use crate::data_interface::Database;
use research_agent_core::conversation::CatalogDataFailureKind;

fn command(operation: u128, expected: &str, title: &str) -> RenameConversation {
    RenameConversation {
        operation_id: Uuid::from_u128(operation),
        id: ConversationId::from_uuid(Uuid::from_u128(2)),
        expected_title: expected.into(),
        title: title.into(),
    }
}
fn managed(title: &str) -> (tempfile::TempDir, SharedDatabase, SqliteConversationCatalog) {
    let root = tempfile::tempdir().unwrap();
    let mut sql = Connection::open_in_memory().unwrap();
    crate::storage_migration_tests::migrate_through(&mut sql, 213);
    sql.execute(
        "insert into agent_conversations(id,title) values(?1,?2)",
        params![Uuid::from_u128(2).to_string(), title],
    )
    .unwrap();
    let db = Database::open(sql, &root.path().join("data-log")).unwrap();
    let catalog = SqliteConversationCatalog::new(db.clone());
    (root, db, catalog)
}
#[tokio::test]
async fn replay_and_status_return_original_proof_without_rewriting_current_metadata() {
    let (_root, db, catalog) = managed("A");
    let first = command(10, "A", " B ");
    let original = catalog.rename(first.clone()).await.unwrap();
    catalog.rename(command(11, "B", "A")).await.unwrap();
    let current = catalog.read(first.id, None).await.unwrap();
    let mut expected = original.clone();
    expected.replayed = true;
    assert_eq!(catalog.rename(first.clone()).await.unwrap(), expected);
    assert_eq!(
        catalog.rename_status(first).await.unwrap(),
        RenameStatus::Committed(expected)
    );
    assert_eq!(catalog.read(current.id, None).await.unwrap(), current);
    assert_eq!(current.title, "A");
    assert_eq!(original.sql_fence, 0);
    assert!(matches!(
        catalog.rename(command(12, "stale", "A")).await,
        Err(CatalogMutationError::Domain(CatalogError::Conflict))
    ));
    assert_eq!(
        catalog
            .rename_legacy_for_tests(current.id, "A".into(), "B".into())
            .await,
        Err(CatalogError::InvalidInput)
    );
    db.shutdown().unwrap();
}
#[tokio::test]
async fn stored_padded_expected_title_is_opaque_without_expanding_target_title_policy() {
    let old = format!("{}Legacy{}", " ".repeat(300), " ".repeat(300));
    let (_root, db, catalog) = managed(&old);
    catalog.rename(command(10, &old, " Short ")).await.unwrap();
    assert_eq!(
        catalog
            .read(command(10, "", "").id, None)
            .await
            .unwrap()
            .title,
        "Short"
    );
    db.shutdown().unwrap();
}
#[tokio::test]
async fn unmanaged_production_port_keeps_operation_diagnostic_and_requires_explicit_fixture_path() {
    let mut sql = Connection::open_in_memory().unwrap();
    crate::storage_migration_tests::migrate_through(&mut sql, 144);
    sql.execute(
        "insert into agent_conversations(id,title) values(?1,'A')",
        [Uuid::from_u128(2).to_string()],
    )
    .unwrap();
    let catalog = SqliteConversationCatalog::new(Arc::new(Mutex::new(sql)));
    for error in [
        catalog.rename(command(10, "A", "B")).await.unwrap_err(),
        catalog
            .rename_status(command(10, "A", "B"))
            .await
            .unwrap_err(),
    ] {
        let CatalogMutationError::Data(error) = error else {
            panic!("unmanaged port must retain a typed data failure")
        };
        assert_eq!(error.kind, CatalogDataFailureKind::Unmanaged);
        assert_eq!(error.diagnostic.operation_id, Uuid::from_u128(10));
        assert_eq!(error.diagnostic.epoch, None);
        assert_eq!(error.diagnostic.sql_fence, None);
    }
    assert_eq!(
        catalog
            .rename_legacy_for_tests(command(10, "", "").id, "A".into(), "B".into())
            .await
            .unwrap()
            .title,
        "B"
    );
}
