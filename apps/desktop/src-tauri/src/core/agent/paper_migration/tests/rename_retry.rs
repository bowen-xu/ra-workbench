//! Managed phase retries preserve the first pending title and canonical history.
use super::*;
use research_agent_core::conversation::RenameConversation;
#[path = "rename_retry/archive_retry.rs"]
mod archive_retry;
#[path = "rename_retry/partial_resume.rs"]
mod partial_resume;

struct Fixture {
    store: AppStore,
    catalog: SqliteConversationCatalog,
    sessions: AgentSessionService,
    root: tempfile::TempDir,
}
impl Fixture {
    fn new(title: &str) -> Self {
        let root = tempfile::tempdir().unwrap();
        let mut sql = rusqlite::Connection::open(root.path().join("migration.sqlite3")).unwrap();
        crate::storage_migration_tests::migrate_through(&mut sql, 217);
        sql.execute("insert into bibliographic_items(id,item_type,created_at,updated_at) values('paper','document','now','now')",[]).unwrap();
        sql.execute("insert into llm_provider_configs(provider_id,enabled,base_url,timeout_ms,updated_at) values('legacy-provider',1,'',60000,'now')",[]).unwrap();
        sql.execute("insert into paper_ai_reading_sessions(id,paper_id,attachment_ids_json,parse_run_ids_json,provider_id,model_id,title,status,created_at,updated_at) values('legacy-session','paper','[]','[]','legacy-provider','legacy-model',?1,'archived','2026-01-01','2026-01-02')",[title]).unwrap();
        sql.execute("insert into paper_ai_reading_turns(id,session_id,ordinal,user_message,assistant_message,messages_json,provider_id,model_id,status,input_tokens,output_tokens,created_at,finished_at) values('legacy-turn','legacy-session',0,'Question','Answer','[{\"role\":\"user\",\"content\":\"Question\"},{\"role\":\"assistant\",\"content\":\"Answer\"}]','legacy-provider','legacy-model','succeeded',5,4,'2026-01-01','2026-01-01T01:00:00Z')",[]).unwrap();
        let database =
            crate::data_interface::Database::open(sql, &root.path().join("data-log")).unwrap();
        let store = AppStore::for_test_database(database, root.path().join("objects"));
        let catalog = SqliteConversationCatalog::new(store.shared_agent_connection());
        let sessions = AgentSessionService::new(Arc::new(
            SqliteConversationStore::new(store.shared_agent_connection(), "新对话".into()).unwrap(),
        ));
        Self {
            store,
            catalog,
            sessions,
            root,
        }
    }
    fn legacy(&self) -> LegacySession {
        load_legacy(&self.store, "paper").unwrap().remove(0)
    }
    fn close(self) {
        let Self {
            store,
            catalog,
            sessions,
            root,
        } = self;
        drop(sessions);
        drop(catalog);
        store.shared_agent_connection().shutdown().unwrap();
        drop(store);
        drop(root);
    }
}
fn phase_receipt(store: &AppStore, conversation: Uuid) -> String {
    store.connection().unwrap().query_row(
        "select json_array(operation_id,epoch,kind,version,hex(fingerprint),sql_fence_position,result_json) from data_sql_operation_receipts where operation_id=?1",
        [Uuid::new_v5(&conversation,b"agent.paper_migration.rename:v1").to_string()], |row| row.get(0),
    ).unwrap()
}
fn snapshot_title(store: &AppStore, conversation: Uuid) -> String {
    store
        .connection()
        .unwrap()
        .query_row(
            "select title from agent_paper_reading_legacy_sessions where conversation_id=?1",
            [conversation.to_string()],
            |row| row.get(0),
        )
        .unwrap()
}
fn raw_metadata(store: &AppStore, conversation: Uuid) -> (String, Vec<String>) {
    let db = store.connection().unwrap();
    let session=db.query_row("select json_array(conversation_id,paper_id,legacy_session_id,attachment_ids_json,parse_run_ids_json,bootstrap_kind,source_workflow_id,source_workflow_revision,bootstrap_messages_json,bootstrap_hash,status,title,created_at,updated_at) from agent_paper_reading_legacy_sessions where conversation_id=?1",[conversation.to_string()],|row| row.get(0)).unwrap();
    let turns=db.prepare("select json_array(conversation_id,legacy_turn_id,request_id,turn_id,ordinal,provider_id,model_id,reasoning_effort,status,input_hash,output_hash,input_tokens,output_tokens,context_window_tokens,context_window_source,error_code,error_message,messages_json,raw_request_json,raw_response_json,created_at,finished_at) from agent_paper_reading_legacy_turns where conversation_id=?1 order by ordinal").unwrap().query_map([conversation.to_string()],|row| row.get(0)).unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap();
    (session, turns)
}
async fn named_pending(
    f: &Fixture,
    conversation: Uuid,
    complete_prefix: bool,
) -> Vec<EventPayload> {
    let legacy = f.legacy();
    let turns = load_turns(&f.store, &legacy.id).unwrap();
    let payloads = turns
        .iter()
        .flat_map(|turn| turn_payloads(&legacy.id, turn))
        .collect::<Vec<_>>();
    reserve_mapping(
        &f.store,
        "paper",
        &legacy.id,
        conversation,
        payloads.len() as i64,
        None,
    )
    .unwrap();
    let frozen =
        prepare_pending_rename_title(&f.store, "paper", &legacy, &turns, conversation, &payloads)
            .unwrap()
            .unwrap();
    rename::apply(&f.catalog, conversation, &frozen)
        .await
        .unwrap();
    if complete_prefix {
        f.sessions
            .open(ConversationId::from_uuid(conversation))
            .await
            .unwrap();
        f.sessions
            .commit_payloads(
                ConversationId::from_uuid(conversation),
                Uuid::from_u128(900),
                payloads.clone(),
            )
            .await
            .unwrap();
    }
    payloads
}

#[tokio::test]
async fn release_preserves_a_managed_claim_with_committed_history() {
    let f = Fixture::new("Legacy title");
    let conversation = Uuid::from_u128(702);
    let payloads = named_pending(&f, conversation, true).await;
    release_mapping(&f.store, "paper", "legacy-session", conversation).unwrap();
    let db = f.store.connection().unwrap();
    let state: (i64, i64, i64) = db.query_row(
        "select (select count(*) from agent_paper_reading_history_migrations where conversation_id=?1 and status='pending'),
                (select count(*) from agent_conversation_bindings where conversation_id=?1 and resource_kind='paper' and resource_id='paper'),
                (select durable_count from agent_conversations where id=?1)",
        [conversation.to_string()], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    ).unwrap();
    assert_eq!(state, (1, 1, payloads.len() as i64));
    drop(db);
    assert!(journal_matches(&f.store, conversation, &payloads).unwrap());
    f.close();
}

#[tokio::test]
async fn pending_retry_replays_frozen_title_after_source_and_current_titles_change() {
    for complete_prefix in [false, true] {
        let f = Fixture::new(" Original title ");
        let conversation = Uuid::from_u128(700);
        let payloads = named_pending(&f, conversation, complete_prefix).await;
        let receipt = phase_receipt(&f.store, conversation);
        f.store.connection().unwrap().execute("update paper_ai_reading_sessions set title='Fresh legacy title' where id='legacy-session'",[]).unwrap();
        f.catalog
            .rename(RenameConversation {
                operation_id: Uuid::from_u128(701),
                id: ConversationId::from_uuid(conversation),
                expected_title: "Original title".into(),
                title: "Manual title".into(),
            })
            .await
            .unwrap();
        let fresh = f.legacy();
        let migrated = migrate_session(&f.store, &f.catalog, &f.sessions, "paper", &fresh)
            .await
            .unwrap();
        assert_eq!(migrated, !complete_prefix);
        assert_eq!(phase_receipt(&f.store, conversation), receipt);
        assert_eq!(snapshot_title(&f.store, conversation), " Original title ");
        assert_eq!(
            f.catalog
                .read(ConversationId::from_uuid(conversation), None)
                .await
                .unwrap()
                .title,
            "Manual title"
        );
        assert!(journal_matches(&f.store, conversation, &payloads).unwrap());
        // Completed mappings retain the old metadata refresh semantics without
        // revisiting the already-completed auxiliary naming phase.
        f.store.connection().unwrap().execute("update paper_ai_reading_sessions set title='Complete refreshed title' where id='legacy-session'",[]).unwrap();
        assert!(
            !migrate_session(&f.store, &f.catalog, &f.sessions, "paper", &f.legacy())
                .await
                .unwrap()
        );
        assert_eq!(
            snapshot_title(&f.store, conversation),
            "Complete refreshed title"
        );
        assert_eq!(phase_receipt(&f.store, conversation), receipt);
        assert_eq!(
            f.catalog
                .read(ConversationId::from_uuid(conversation), None)
                .await
                .unwrap()
                .title,
            "Manual title"
        );
        f.close();
    }
}

#[tokio::test]
async fn stale_pending_capture_after_completion_never_renames_appends_or_overwrites_raw_metadata() {
    let f = Fixture::new(" Original title ");
    let conversation = Uuid::from_u128(700);
    let payloads = named_pending(&f, conversation, true).await;
    let pending: String = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "select status from agent_paper_reading_history_migrations where conversation_id=?1",
            [conversation.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(pending, "pending");
    let mut stale = f.legacy();
    stale.title = "Stale caller fresh title".into();
    let mut turns = load_turns(&f.store, &stale.id).unwrap();
    assert!(
        !migrate_session(&f.store, &f.catalog, &f.sessions, "paper", &f.legacy())
            .await
            .unwrap()
    );
    let original = raw_metadata(&f.store, conversation);
    let receipt = phase_receipt(&f.store, conversation);
    assert_eq!(
        prepare_pending_rename_title(&f.store, "paper", &stale, &turns, conversation, &payloads)
            .unwrap(),
        None
    );
    assert_eq!(raw_metadata(&f.store, conversation), original);
    turns[0].messages_json="[{\"role\":\"user\",\"content\":\"Question\"},{\"role\":\"assistant\",\"content\":\"Changed source\"}]".into();
    let changed = turns
        .iter()
        .flat_map(|turn| turn_payloads(&stale.id, turn))
        .collect::<Vec<_>>();
    assert!(matches!(
        prepare_pending_rename_title(&f.store, "paper", &stale, &turns, conversation, &changed),
        Err(Error::Conflict)
    ));
    assert_eq!(raw_metadata(&f.store, conversation), original);
    assert_eq!(phase_receipt(&f.store, conversation), receipt);
    assert!(journal_matches(&f.store, conversation, &payloads).unwrap());
    assert_eq!(
        f.catalog
            .read(ConversationId::from_uuid(conversation), None)
            .await
            .unwrap()
            .durable_count,
        payloads.len() as u64
    );
    f.close();
}

#[tokio::test]
async fn invalid_legacy_titles_preserve_full_metadata_and_history_without_a_rename_receipt() {
    for title in ["旧".repeat(181), "Legacy\0title".into()] {
        let f = Fixture::new(&title);
        let legacy = f.legacy();
        let turns = load_turns(&f.store, &legacy.id).unwrap();
        let payloads = turns
            .iter()
            .flat_map(|turn| turn_payloads(&legacy.id, turn))
            .collect::<Vec<_>>();
        assert!(
            migrate_session(&f.store, &f.catalog, &f.sessions, "paper", &legacy)
                .await
                .unwrap()
        );
        let conversation=Uuid::parse_str(&f.store.connection().unwrap().query_row("select conversation_id from agent_paper_reading_history_migrations where legacy_session_id='legacy-session'",[],|row| row.get::<_,String>(0)).unwrap()).unwrap();
        assert_eq!(snapshot_title(&f.store, conversation), title);
        let summary = f
            .catalog
            .read(ConversationId::from_uuid(conversation), None)
            .await
            .unwrap();
        assert_eq!(summary.title, "新对话");
        assert_eq!(summary.durable_count, payloads.len() as u64);
        assert!(journal_matches(&f.store, conversation, &payloads).unwrap());
        assert_eq!(f.store.connection().unwrap().query_row("select count(*) from data_sql_operation_receipts where kind='agent.conversation.rename'",[],|row| row.get::<_,u64>(0)).unwrap(),0);
        assert!(matches!(
            rename::apply(&f.catalog, Uuid::nil(), &title).await,
            Err(Error::Mutation(_))
        ));
        f.close();
    }
}

#[tokio::test]
async fn unknown_creation_commit_keeps_mapping_and_retries_same_phase_after_reopen() {
    let f = Fixture::new("Legacy title");
    f.store.connection().unwrap().execute_batch("create table creation_parent(id text primary key);
      create table creation_child(id text primary key,parent_id text references creation_parent(id) deferrable initially deferred);
      create trigger reject_creation_receipt_commit after insert on data_sql_operation_receipts when new.kind='agent.conversation.create_bound' begin insert into creation_child values(new.operation_id,'missing'); end;").unwrap();
    let error = migrate_session(&f.store, &f.catalog, &f.sessions, "paper", &f.legacy())
        .await
        .unwrap_err();
    let Error::Mutation(crate::agent::catalog_commands::CreateBoundCommandError::Data(failure)) =
        error
    else {
        panic!("creation commit must retain typed storage failure")
    };
    assert_eq!(
        failure.kind,
        research_agent_core::conversation::CatalogDataFailureKind::Unknown
    );
    assert_eq!(failure.diagnostic.phase, "commit");
    assert_eq!(failure.diagnostic.sqlite_extended_code, Some(787));
    let observer = rusqlite::Connection::open(f.root.path().join("migration.sqlite3")).unwrap();
    let (conversation,status):(String,String)=observer.query_row("select conversation_id,status from agent_paper_reading_history_migrations where legacy_session_id='legacy-session'",[],|row|Ok((row.get(0)?,row.get(1)?))).unwrap();
    assert_eq!(status, "pending");
    let id = Uuid::parse_str(&conversation).unwrap();
    let operation = Uuid::new_v5(&id, b"agent.paper_migration.create_bound:v1");
    assert_eq!(failure.diagnostic.operation_id, operation);
    assert_eq!(
        observer
            .query_row(
                "select count(*) from agent_conversation_bindings where conversation_id=?1",
                [&conversation],
                |row| row.get::<_, u64>(0)
            )
            .unwrap(),
        1
    );
    assert_eq!(
        observer
            .query_row(
                "select count(*) from data_sql_operation_receipts where operation_id=?1",
                [operation.to_string()],
                |row| row.get::<_, u64>(0)
            )
            .unwrap(),
        0
    );
    let Fixture {
        store,
        catalog,
        sessions,
        root,
    } = f;
    drop(sessions);
    drop(catalog);
    drop(store);
    observer
        .execute_batch("drop trigger reject_creation_receipt_commit;")
        .unwrap();
    drop(observer);
    let sql = rusqlite::Connection::open(root.path().join("migration.sqlite3")).unwrap();
    let database =
        crate::data_interface::Database::open(sql, &root.path().join("data-log")).unwrap();
    let store = AppStore::for_test_database(database, root.path().join("objects"));
    let catalog = SqliteConversationCatalog::new(store.shared_agent_connection());
    let sessions = AgentSessionService::new(Arc::new(
        SqliteConversationStore::new(store.shared_agent_connection(), "新对话".into()).unwrap(),
    ));
    let reopened = Fixture {
        store,
        catalog,
        sessions,
        root,
    };
    assert!(
        migrate_session(
            &reopened.store,
            &reopened.catalog,
            &reopened.sessions,
            "paper",
            &reopened.legacy()
        )
        .await
        .unwrap()
    );
    assert_eq!(reopened.store.connection().unwrap().query_row("select conversation_id from agent_paper_reading_history_migrations where legacy_session_id='legacy-session'",[],|row|row.get::<_,String>(0)).unwrap(),conversation);
    assert_eq!(reopened.store.connection().unwrap().query_row("select count(*) from data_sql_operation_receipts where operation_id=?1 and kind='agent.conversation.create_bound'",[operation.to_string()],|row|row.get::<_,u64>(0)).unwrap(),1);
    reopened.close();
}
