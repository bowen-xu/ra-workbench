//! Real paired SQL/log owner: authorization receipts do not replace live authority.
use super::*;
use crate::data_interface::{AuthorizeRollback, Database, SharedDatabase, SqlImmediateReceipt};

pub(super) struct Fixture {
    pub(super) catalog: SqliteConversationCatalog,
    pub(super) service: Arc<AgentSessionService>,
    pub(super) database: SharedDatabase,
    pub(super) id: Uuid,
    pub(super) turn: Uuid,
    pub(super) _directory: tempfile::TempDir,
}
pub(super) async fn zero_fixture() -> Fixture {
    let directory = tempfile::tempdir().unwrap();
    let mut sql = Connection::open(directory.path().join("rollback.sqlite3")).unwrap();
    crate::storage::migrate(&mut sql).unwrap();
    sql.execute("insert into note_pages(id,title,created_at,updated_at) values ('note-one','Note','now','now'),('note-two','Other','now','now')", []).unwrap();
    let database = Database::open(sql, &directory.path().join("data-log")).unwrap();
    let catalog = SqliteConversationCatalog::new(database.clone());
    let id = Uuid::new_v4();
    catalog
        .create_bound(research_agent_core::conversation::CreateBoundConversation {
            operation_id: Uuid::new_v5(&id, b"rollback-fixture:create-bound"),
            id: ConversationId::from_uuid(id),
            binding: binding("note-one"),
        })
        .await
        .unwrap();
    let service = Arc::new(AgentSessionService::new(Arc::new(
        SqliteConversationStore::new(database.clone(), "Managed rollback".into()).unwrap(),
    )));
    service.open(ConversationId::from_uuid(id)).await.unwrap();
    let turn = Uuid::nil();
    Fixture {
        catalog,
        service,
        database,
        id,
        turn,
        _directory: directory,
    }
}
pub(super) async fn fixture() -> Fixture {
    let mut fixture = zero_fixture().await;
    fixture.turn = user_turn(&fixture.service, fixture.id).await;
    fixture
}
fn binding(resource: &str) -> ConversationBinding {
    ConversationBinding {
        kind: BindingKind::Note,
        resource_id: resource.into(),
    }
}
async fn receipt(fixture: &Fixture, operation: Uuid) -> SqlImmediateReceipt {
    let database = fixture.database.clone();
    let command = AuthorizeRollback {
        operation_id: operation,
        conversation_id: ConversationId::from_uuid(fixture.id),
        binding: binding("note-one"),
    };
    tokio::task::spawn_blocking(move || database.authorize_rollback(&command))
        .await
        .unwrap()
        .unwrap()
}
async fn count(database: &SharedDatabase, query: &'static str) -> u64 {
    let database = database.clone();
    tokio::task::spawn_blocking(move || {
        database
            .lock()
            .unwrap()
            .query_row(query, [], |row| row.get(0))
            .unwrap()
    })
    .await
    .unwrap()
}
pub(super) async fn revoke(fixture: &Fixture) {
    let database = fixture.database.clone();
    let id = fixture.id;
    tokio::task::spawn_blocking(move || {
        database
            .lock()
            .unwrap()
            .execute(
                "delete from agent_conversation_bindings where conversation_id=?1",
                [id.to_string()],
            )
            .unwrap()
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn paired_authorization_and_actual_rollback_replay_preserve_later_history() {
    let fixture = fixture().await;
    let request = Uuid::new_v4();
    let value = wire(fixture.id, fixture.turn, request);
    let response = execute(
        &fixture.catalog,
        &fixture.service,
        serde_json::from_value(value.clone()).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(response.watermark, 6);
    let authorization = receipt(&fixture, request).await;
    assert!(authorization.replayed);
    assert_eq!(authorization.operation_id, request);
    assert!(!authorization.epoch.is_nil());
    assert_eq!(
        count(
            &fixture.database,
            "select count(*) from agent_rollback_authority"
        )
        .await,
        1
    );
    assert_eq!(
        count(
            &fixture.database,
            "select count(*) from data_sql_operation_receipts where kind='agent.rollback_authority'"
        )
        .await,
        1
    );
    let later = user_turn(&fixture.service, fixture.id).await;
    let retry = execute(
        &fixture.catalog,
        &fixture.service,
        serde_json::from_value(value).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(retry.watermark, 6);
    let state = fixture
        .service
        .open(ConversationId::from_uuid(fixture.id))
        .await
        .unwrap();
    assert_eq!(state.seq(), 11);
    assert_eq!(state.turn_order(), &[later]);
    assert_eq!(
        receipt(&fixture, request).await.sql_fence,
        authorization.sql_fence
    );
    assert!(
        fixture
            .catalog
            .authorize_rollback_legacy_for_tests(
                ConversationId::from_uuid(fixture.id),
                Uuid::new_v4(),
                binding("note-one")
            )
            .await
            .is_err()
    );
}

#[tokio::test]
async fn paired_receipt_never_bypasses_live_revocation_or_actual_rollback_recheck() {
    let fixture = fixture().await;
    let request = Uuid::new_v4();
    let value = wire(fixture.id, fixture.turn, request);
    fixture
        .catalog
        .authorize_rollback(
            ConversationId::from_uuid(fixture.id),
            request,
            binding("note-one"),
        )
        .await
        .unwrap();
    assert!(receipt(&fixture, request).await.replayed);
    revoke(&fixture).await;
    assert!(matches!(
        execute(
            &fixture.catalog,
            &fixture.service,
            serde_json::from_value(value.clone()).unwrap()
        )
        .await,
        Err(Error::HostUnavailable)
    ));
    let (request, _) = serde_json::from_value::<RollbackRequest>(value)
        .unwrap()
        .decode()
        .unwrap();
    assert!(
        fixture
            .service
            .rollback_conversation(request)
            .await
            .is_err()
    );
    let id = ConversationId::from_uuid(fixture.id);
    assert!(matches!(
        fixture.service.open(id).await,
        Err(AgentServiceError::Session(
            research_agent_core::session::SessionError::RecoveryRequired
        ))
    ));
    // A failed write keeps the session's recovery barrier. Read the durable
    // prefix independently instead of clearing that barrier for the fixture.
    let page = fixture.service.read_since(id, 0, 10).await.unwrap();
    assert_eq!(page.durable_count, 5);
    assert_eq!(page.items.len(), 5);
    assert!(
        page.items
            .iter()
            .all(|event| !matches!(event.payload(), EventPayload::ConversationRolledBack { .. }))
    );
    let mut state = research_agent_core::protocol::ConversationProjection::new(id);
    for event in &page.items {
        state.apply(event).unwrap();
    }
    assert_eq!(state.seq(), 5);
    assert_eq!(state.turn_order(), &[fixture.turn]);
    assert_eq!(
        count(&fixture.database, "select count(*) from agent_items").await,
        5
    );
    assert_eq!(
        count(&fixture.database, "select count(*) from agent_input_queues").await,
        0
    );
    assert_eq!(
        count(
            &fixture.database,
            "select count(*) from data_sql_operation_receipts where kind='agent.rollback_authority'"
        )
        .await,
        1
    );
}

#[tokio::test]
async fn paired_registration_checks_live_host_before_prior_identity_conflict() {
    let fixture = fixture().await;
    let request = Uuid::new_v4();
    fixture
        .catalog
        .authorize_rollback(
            ConversationId::from_uuid(fixture.id),
            request,
            binding("note-one"),
        )
        .await
        .unwrap();
    let mut changed = wire(fixture.id, fixture.turn, request);
    changed["binding"]["resourceId"] = serde_json::json!("note-two");
    assert!(matches!(
        execute(
            &fixture.catalog,
            &fixture.service,
            serde_json::from_value(changed.clone()).unwrap()
        )
        .await,
        Err(Error::HostUnavailable)
    ));
    // This fixture represents pre-existing multiple host associations. The
    // production API never adds a host through a standalone bind command.
    let database = fixture.database.clone();
    let conversation = fixture.id;
    tokio::task::spawn_blocking(move || {
        database.lock().unwrap().execute(
            "insert into agent_conversation_bindings(conversation_id,resource_kind,resource_id) values(?1,'note','note-two')",
            [conversation.to_string()],
        ).unwrap();
    }).await.unwrap();
    assert!(matches!(
        execute(
            &fixture.catalog,
            &fixture.service,
            serde_json::from_value(changed).unwrap()
        )
        .await,
        Err(Error::Conflict)
    ));
    assert_eq!(
        fixture
            .service
            .open(ConversationId::from_uuid(fixture.id))
            .await
            .unwrap()
            .seq(),
        5
    );
    assert_eq!(
        count(
            &fixture.database,
            "select count(*) from agent_rollback_authority"
        )
        .await,
        1
    );
}

#[tokio::test]
async fn unmanaged_production_entry_fails_without_a_managed_receipt_and_legacy_is_explicit() {
    let (database, catalog, service, id, turn) = setup().await;
    let request = Uuid::new_v4();
    let value = wire(id, turn, request);
    let error = execute(
        &catalog,
        &service,
        serde_json::from_value(value.clone()).unwrap(),
    )
    .await
    .unwrap_err();
    let Error::Storage(SqlImmediateFailure::Unmanaged(diagnostic)) = error else {
        panic!("expected unmanaged storage diagnostic")
    };
    assert_eq!(diagnostic.operation_id, request);
    assert!(diagnostic.epoch.is_none());
    assert_eq!(
        database
            .lock()
            .unwrap()
            .query_row("select count(*) from agent_rollback_authority", [], |row| {
                row.get::<_, u64>(0)
            })
            .unwrap(),
        0
    );
    assert_eq!(
        execute_legacy_for_tests(&catalog, &service, serde_json::from_value(value).unwrap())
            .await
            .unwrap()
            .watermark,
        6
    );
    assert_eq!(
        database
            .lock()
            .unwrap()
            .query_row(
                "select count(*) from data_sql_operation_receipts where kind='agent.rollback_authority'",
                [],
                |row| row.get::<_, u64>(0)
            )
            .unwrap(),
        0
    );
}
