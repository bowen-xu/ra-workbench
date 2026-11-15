use super::*;
use crate::agent::config::ConfigOverride;
use crate::agent::reading_note_submission as reading;
use crate::notes::reading_agent_context;
use research_agent_core::config::ConfigTier;
use std::fs;
#[path = "reading_note/profiles.rs"]
mod profiles;
#[path = "reading_note/queue.rs"]
mod queue;
#[path = "reading_note/steering.rs"]
mod steering;

fn reading_fixture() -> Fixture {
    let f = fixture(true);
    add_reading_source(f)
}
fn managed_reading_fixture() -> Fixture {
    add_reading_source(fixture_with_owner(true, true))
}
fn add_reading_source(f: Fixture) -> Fixture {
    {
        let mut db = f.store.connection().unwrap();
        let tx = db.transaction().unwrap();
        // Deliberately share the identifier with an independent page in another table.
        crate::notes::insert_reading_fixture(&tx, &f.page.to_string(), "My reading interpretation");
        tx.execute(
            "update reading_note_sources set quote_exact='Retained source quote' where block_id=?1",
            [f.page.to_string()],
        )
        .unwrap();
        tx.execute("insert into agent_conversation_bindings(conversation_id,resource_kind,resource_id) values (?1,'readingNote',?2)", params![f.conversation.to_string(),f.page.to_string()]).unwrap();
        tx.commit().unwrap();
    }
    f
}
fn reading_request(f: &Fixture, id: Uuid) -> reading::PrepareRequest {
    reading::PrepareRequest {
        user_input: vec![],
        conversation_id: f.conversation,
        request_id: id,
        text: "Explain this reading note".into(),
        context: reading_agent_context::Request {
            reading_note_id: f.page,
            expected_revision: 0,
        },
        retrieval: None,
        model: None,
        session_config: None,
        turn_config: None,
    }
}
fn reading_receipt(f: &Fixture, id: Uuid) -> reading::Receipt {
    reading::Receipt {
        conversation_id: f.conversation,
        request_id: id,
        reading_note_id: f.page,
    }
}

#[tokio::test]
async fn reading_note_profile_overrides_are_frozen_in_submission() {
    let f = reading_fixture();
    let root = f.store.agent_configuration_directory().join("agent");
    fs::create_dir_all(&root).unwrap();
    fs::write(
        root.join("config.toml"),
        "schema_version=1\n[profiles.reading_note]\ntext='123456789'\nmax_bytes=9\n",
    )
    .unwrap();
    let mut request = reading_request(&f, Uuid::new_v4());
    request.session_config = Some(ConfigOverride {
        max_bytes: Some(6),
        fallback_filenames: None,
    });
    request.turn_config = Some(ConfigOverride {
        max_bytes: Some(3),
        fallback_filenames: None,
    });
    let record = reading::prepare(&f.store, &f.models, request)
        .await
        .unwrap();
    assert_eq!(record.input.instructions[0].text, "123");
    assert_eq!(
        record.input.instruction_configs[0]
            .layers
            .last()
            .unwrap()
            .origin
            .tier,
        ConfigTier::Turn
    );
    fs::write(root.join("config.toml"), "invalid configuration").unwrap();
    assert_eq!(
        reading::restore(&f.store, reading_receipt(&f, record.request_id))
            .await
            .unwrap(),
        record
    );
}

#[tokio::test]
async fn reading_preparation_is_shared_idempotent_and_preserves_distinct_evidence() {
    let f = reading_fixture();
    let id = Uuid::new_v4();
    let (one, two) = tokio::join!(
        reading::prepare(&f.store, &f.models, reading_request(&f, id)),
        reading::prepare(&f.store, &f.models, reading_request(&f, id))
    );
    let saved = one.unwrap();
    assert_eq!(two.unwrap(), saved);
    assert_eq!(saved.input.model.model_id, "note-fixture");
    assert_eq!(
        saved.input.context[0].source_id,
        format!("reading-note:{}", f.page)
    );
    let body: serde_json::Value = serde_json::from_str(&saved.input.context[0].text).unwrap();
    assert_eq!(
        body["content"]["blocks"][0]["content"]["items"][0]["text"],
        "My reading interpretation"
    );
    assert_eq!(body["source"]["quoteExact"], "Retained source quote");
    assert!(
        saved.input.instructions.is_empty()
            && saved.input.instruction_configs.is_empty()
            && saved.input.skills.is_empty()
    );
    assert!(
        !serde_json::to_string(&saved.input)
            .unwrap()
            .contains("synthetic-note-fixture")
    );
    assert_eq!(count(&f), 1);
    let db = f.store.connection().unwrap();
    assert_eq!(
        db.query_row("select count(*) from agent_items", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        db.query_row(
            "select p.revision from note_pages p join reading_note_sources s on s.page_id=p.id where s.block_id=?1",
            [f.page.to_string()],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}

#[tokio::test]
async fn reading_restore_keeps_original_after_edits_route_disable_and_database_reopen() {
    let f = reading_fixture();
    let id = Uuid::new_v4();
    let saved = reading::prepare(&f.store, &f.models, reading_request(&f, id))
        .await
        .unwrap();
    f.store
        .connection()
        .unwrap()
        .execute(
            "update reading_note_sources set quote_exact='Changed source'",
            [],
        )
        .unwrap();
    assert_eq!(
        reading::prepare(&f.store, &f.models, reading_request(&f, id)).await,
        Err(Error::Conflict)
    );
    {
        let db = f.store.connection().unwrap();
        db.execute(
            "update note_pages set revision=1 where id in (select page_id from reading_note_sources)",
            [],
        )
        .unwrap();
        db.execute("update llm_provider_configs set enabled=0", [])
            .unwrap();
    }
    assert_eq!(
        reading::restore(&f.store, reading_receipt(&f, id))
            .await
            .unwrap(),
        saved
    );
    let reopened = AppStore::for_test(
        Connection::open(f.root.path().join("fixture.sqlite")).unwrap(),
        f.root.path().join("data/objects"),
    );
    assert_eq!(
        reading::restore(&reopened, reading_receipt(&f, id))
            .await
            .unwrap(),
        saved
    );
    assert_eq!(count(&f), 1);
}

#[tokio::test]
async fn receipts_cannot_be_relabeled_between_reading_note_and_same_id_page() {
    let f = reading_fixture();
    let page_id = Uuid::new_v4();
    let reading_id = Uuid::new_v4();
    prepare(&f.store, &f.models, request(&f, page_id))
        .await
        .unwrap();
    reading::prepare(&f.store, &f.models, reading_request(&f, reading_id))
        .await
        .unwrap();
    assert_eq!(
        reading::restore(&f.store, reading_receipt(&f, page_id)).await,
        Err(Error::Unbound)
    );
    assert_eq!(
        restore(&f.store, receipt(&f, reading_id)).await,
        Err(Error::Unbound)
    );
    assert_eq!(
        reading::prepare(&f.store, &f.models, reading_request(&f, page_id)).await,
        Err(Error::Conflict)
    );
    assert_eq!(count(&f), 2);
}

#[tokio::test]
async fn reading_lifecycle_and_binding_are_rechecked_before_returning_frozen_receipts() {
    for sql in [
        "delete from agent_conversation_bindings where resource_kind='readingNote'",
        "update note_pages set status='archived' where id in (select page_id from reading_note_sources)",
        "update note_pages set status='trashed' where id in (select page_id from reading_note_sources)",
        "delete from note_pages where id in (select page_id from reading_note_sources)",
        "update agent_conversations set archived=1",
    ] {
        let f = reading_fixture();
        let id = Uuid::new_v4();
        reading::prepare(&f.store, &f.models, reading_request(&f, id))
            .await
            .unwrap();
        f.store.connection().unwrap().execute(sql, []).unwrap();
        f.store
            .connection()
            .unwrap()
            .execute("update llm_provider_configs set enabled=0", [])
            .unwrap();
        assert_eq!(
            reading::restore(&f.store, reading_receipt(&f, id)).await,
            Err(Error::Unbound),
            "{sql}"
        );
        assert_eq!(
            reading::prepare(&f.store, &f.models, reading_request(&f, Uuid::new_v4())).await,
            Err(Error::Unbound),
            "{sql}"
        );
        assert_eq!(count(&f), 1);
    }
}

#[tokio::test]
async fn invalid_requests_and_failed_commit_never_publish_partial_reading_submissions() {
    let f = reading_fixture();
    let id = Uuid::new_v4();
    let mut invalid = reading_request(&f, id);
    invalid.context.expected_revision = -1;
    assert_eq!(
        reading::prepare(&f.store, &f.models, invalid).await,
        Err(Error::InvalidInput)
    );
    assert!(serde_json::from_value::<reading::PrepareRequest>(serde_json::json!({"conversationId":f.conversation,"requestId":id,"text":"test","context":{"readingNoteId":f.page,"expectedRevision":0},"allowDisclosure":true})).is_err());
    f.store.connection().unwrap().execute_batch("create trigger reject_reading_submission before insert on agent_submissions begin select raise(abort,'synthetic failure'); end;").unwrap();
    assert_eq!(
        reading::prepare(&f.store, &f.models, reading_request(&f, id)).await,
        Err(Error::Unavailable)
    );
    assert_eq!(count(&f), 0);
    f.store
        .connection()
        .unwrap()
        .execute_batch("drop trigger reject_reading_submission;")
        .unwrap();
    assert!(
        reading::prepare(&f.store, &f.models, reading_request(&f, id))
            .await
            .is_ok()
    );
    assert_eq!(count(&f), 1);
}

#[path = "reading_note/recovery.rs"]
mod recovery;
#[path = "reading_note/start.rs"]
mod start;

#[test]
fn a_mapped_reading_document_cannot_gain_ordinary_note_write_authority() {
    let f = reading_fixture();
    let db = f.store.connection().unwrap();
    let page: String = db
        .query_row(
            "select page_id from reading_note_sources where block_id=?1",
            [f.page.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    db.execute("insert into agent_conversation_bindings(conversation_id,resource_kind,resource_id) values(?1,'note',?2)", params![f.conversation.to_string(), page]).unwrap();
    assert!(matches!(
        check_binding(&db, f.conversation, Uuid::parse_str(&page).unwrap()),
        Err(Error::Unbound)
    ));
    assert!(check_binding(&db, f.conversation, f.page).is_ok());
    let bindable: bool = db
        .query_row(
            crate::agent::repository::binding_target_query(
                research_agent_core::conversation::BindingKind::Note,
            ),
            [&page],
            |r| r.get(0),
        )
        .unwrap();
    assert!(!bindable);
}
