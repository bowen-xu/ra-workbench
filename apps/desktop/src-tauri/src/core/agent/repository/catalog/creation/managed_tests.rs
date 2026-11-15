//! The catalog port exposes immutable proof separately from current metadata.
use super::*;
use crate::data_interface::Database;
use research_agent_core::conversation::{CatalogDataFailureKind, RenameConversation};

fn command(operation: u128, id: u128, resource: &str) -> CreateBoundConversation {
    CreateBoundConversation {
        operation_id: Uuid::from_u128(operation),
        id: ConversationId::from_uuid(Uuid::from_u128(id)),
        binding: ConversationBinding {
            kind: BindingKind::Note,
            resource_id: resource.into(),
        },
    }
}
fn managed() -> (tempfile::TempDir, SharedDatabase, SqliteConversationCatalog) {
    let root = tempfile::tempdir().unwrap();
    let mut sql = Connection::open_in_memory().unwrap();
    crate::storage_migration_tests::migrate_through(&mut sql, 213);
    sql.execute("insert into note_pages(id,title,created_at,updated_at) values('one','Note','now','now'),('two','Other','now','now')",[]).unwrap();
    let db = Database::open(sql, &root.path().join("data-log")).unwrap();
    let catalog = SqliteConversationCatalog::new(db.clone());
    (root, db, catalog)
}
#[tokio::test]
async fn legacy_binding_fixture_refuses_managed_owner_before_mutation() {
    let (_root, db, catalog) = managed();
    let original = command(1, 2, "one");
    catalog.create_bound(original.clone()).await.unwrap();
    let extra = ConversationBinding {
        kind: BindingKind::Note,
        resource_id: "two".into(),
    };
    let before = db.lock().unwrap().total_changes();
    assert_eq!(
        catalog
            .bind_legacy_for_tests(original.id, extra.clone())
            .await,
        Err(CatalogError::InvalidInput)
    );
    assert_eq!(db.lock().unwrap().total_changes(), before);
    assert_eq!(
        catalog.read(original.id, Some(extra)).await.unwrap_err(),
        CatalogError::NotFound
    );
}
#[tokio::test]
async fn creation_proof_replay_and_status_never_substitute_current_metadata() {
    let (_root, db, catalog) = managed();
    let command = command(1, 2, "one");
    let proof = catalog.create_bound(command.clone()).await.unwrap();
    let initial = catalog
        .read(command.id, Some(command.binding.clone()))
        .await
        .unwrap();
    let binding_time: String = db
        .lock()
        .unwrap()
        .query_row(
            "select created_at from agent_conversation_bindings",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(initial.created_at, binding_time);
    assert_eq!(initial.created_at, initial.updated_at);
    catalog
        .rename(RenameConversation {
            operation_id: Uuid::from_u128(3),
            id: command.id,
            expected_title: "新对话".into(),
            title: "Current".into(),
        })
        .await
        .unwrap();
    catalog
        .set_archived(ArchiveConversation {
            operation_id: Uuid::from_u128(50),
            id: command.id,
            archived: true,
            expected_watermark: Some(0),
        })
        .await
        .unwrap();
    let current = catalog.read(command.id, None).await.unwrap();
    let mut replay = proof;
    replay.replayed = true;
    assert_eq!(catalog.create_bound(command.clone()).await.unwrap(), replay);
    assert_eq!(
        catalog.create_bound_status(command.clone()).await.unwrap(),
        CreateBoundStatus::Committed(replay)
    );
    let second = catalog
        .create_bound(CreateBoundConversation {
            operation_id: Uuid::from_u128(4),
            ..command.clone()
        })
        .await
        .unwrap();
    assert!(!second.replayed);
    assert_eq!(catalog.read(command.id, None).await.unwrap(), current);
    assert!(current.archived);
    assert_eq!(current.title, "Current");
    assert_eq!(
        catalog
            .create_bound_legacy_for_tests(command.id, command.binding)
            .await,
        Err(CatalogError::InvalidInput)
    );
    db.shutdown().unwrap();
}
#[tokio::test]
async fn replay_rechecks_live_target_but_status_only_inspects_original_receipt() {
    let (_root, db, catalog) = managed();
    let first = command(1, 2, "one");
    catalog.create_bound(first.clone()).await.unwrap();
    db.lock()
        .unwrap()
        .execute("delete from note_pages where id='one'", [])
        .unwrap();
    assert!(matches!(
        catalog.create_bound(first.clone()).await,
        Err(CatalogMutationError::Domain(CatalogError::NotFound))
    ));
    assert!(matches!(
        catalog.create_bound_status(first.clone()).await,
        Ok(CreateBoundStatus::Committed(_))
    ));
    assert!(catalog.read(first.id, Some(first.binding)).await.is_ok());
    db.shutdown().unwrap();
}
#[tokio::test]
async fn managed_creation_rolls_back_initial_binding_and_never_adopts_an_unbound_id() {
    let (_root, db, catalog) = managed();
    db.lock().unwrap().execute_batch("create trigger reject_initial_binding before insert on agent_conversation_bindings begin select raise(abort,'synthetic binding failure'); end;").unwrap();
    assert!(catalog.create_bound(command(1, 2, "one")).await.is_err());
    assert_eq!(
        catalog.read(command(1, 2, "one").id, None).await,
        Err(CatalogError::NotFound)
    );
    let sql = db.lock().unwrap();
    sql.execute_batch("drop trigger reject_initial_binding;")
        .unwrap();
    sql.execute(
        "insert into agent_conversations(id,title) values(?1,'Unbound')",
        [Uuid::from_u128(2).to_string()],
    )
    .unwrap();
    drop(sql);
    assert!(matches!(
        catalog.create_bound(command(1, 2, "one")).await,
        Err(CatalogMutationError::Domain(CatalogError::Conflict))
    ));
    assert!(matches!(
        catalog.create_bound_status(command(1, 2, "one")).await,
        Ok(CreateBoundStatus::NoSqlReceipt { .. })
    ));
    db.shutdown().unwrap();
}
#[tokio::test]
async fn unmanaged_port_preserves_operation_diagnostic_and_has_no_implicit_sql_fallback() {
    let mut sql = Connection::open_in_memory().unwrap();
    crate::storage_migration_tests::migrate_through(&mut sql, 144);
    let catalog = SqliteConversationCatalog::new(Arc::new(Mutex::new(sql)));
    for result in [
        catalog.create_bound(command(1, 2, "one")).await.map(|_| ()),
        catalog
            .create_bound_status(command(1, 2, "one"))
            .await
            .map(|_| ()),
    ] {
        let Err(CatalogMutationError::Data(error)) = result else {
            panic!("managed port must reject SQL-only owner")
        };
        assert_eq!(error.kind, CatalogDataFailureKind::Unmanaged);
        assert_eq!(error.diagnostic.operation_id, Uuid::from_u128(1));
        assert_eq!(error.diagnostic.epoch, None);
        assert_eq!(error.diagnostic.sql_fence, None);
    }
}
