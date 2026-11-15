use super::*;
use crate::{agent::repository::create_conversation, ai::JsonCredentialStore};
use research_agent_core::config::ConfigTier;
use research_agent_core::conversation::ConversationId;
use research_agent_core::submission::ContextText;
use sha2::{Digest, Sha256};
#[path = "tests/application.rs"]
mod application;
#[path = "tests/child_submission.rs"]
mod child_submission;
#[path = "tests/native_trace.rs"]
mod native_trace;
#[path = "tests/proposals.rs"]
mod proposals;
#[path = "tests/reading_note.rs"]
mod reading_note;
#[path = "tests/recovery.rs"]
mod recovery;
#[path = "tests/rejection.rs"]
mod rejection;
#[path = "tests/review.rs"]
mod review;
#[path = "tests/runner.rs"]
mod runner;
#[path = "tests/start.rs"]
mod start;
#[path = "tests/undo.rs"]
mod undo;

struct Fixture {
    root: tempfile::TempDir,
    store: AppStore,
    models: AgentModelResolver,
    conversation: Uuid,
    page: Uuid,
}
fn fixture(bound: bool) -> Fixture {
    fixture_with_owner(bound, false)
}
fn fixture_with_owner(bound: bool, managed: bool) -> Fixture {
    fixture_with_owner_at(bound, managed, tempfile::tempdir().unwrap())
}
fn fixture_with_owner_at(bound: bool, managed: bool, root: tempfile::TempDir) -> Fixture {
    let mut db = Connection::open(root.path().join("fixture.sqlite")).unwrap();
    if managed {
        crate::storage::migrate(&mut db).unwrap();
    } else {
        crate::storage_migration_tests::migrate_through(&mut db, 144);
    }
    let conversation = Uuid::new_v4();
    let page = Uuid::new_v4();
    create_conversation(
        &mut db,
        ConversationId::from_uuid(conversation),
        "Synthetic notes",
    )
    .unwrap();
    db.execute("insert into note_pages(id,title,created_at,updated_at) values (?1,'Synthetic note','now','now')", [page.to_string()]).unwrap();
    let content = serde_json::json!({"kind":"inline","items":[{"type":"text","text":"Frozen note evidence","styles":[]}]}).to_string();
    db.execute("insert into note_blocks(id,page_id,scope_kind,block_type,content_json,created_at,updated_at) values ('selected',?1,'page','paragraph',?2,'now','now')", params![page.to_string(),content]).unwrap();
    if bound {
        db.execute("insert into agent_conversation_bindings(conversation_id,resource_kind,resource_id) values (?1,'note',?2)", params![conversation.to_string(),page.to_string()]).unwrap();
    }
    db.execute("insert into llm_provider_configs(provider_id,enabled,base_url,timeout_ms,updated_at) values ('deepseek',1,'',60000,'now') on conflict(provider_id) do update set enabled=1,base_url=''", []).unwrap();
    for (kind, id, model) in [
        ("default", "text", "default-fixture"),
        ("use_case", "note_agent", "note-fixture"),
    ] {
        db.execute("insert into llm_model_routes(route_kind,route_id,model_kind,provider_id,model_id,updated_at) values (?1,?2,'text','deepseek',?3,'now') on conflict(route_kind,route_id) do update set provider_id='deepseek',model_id=excluded.model_id", params![kind,id,model]).unwrap();
    }
    let store = if managed {
        let db = crate::data_interface::Database::open(db, &root.path().join("log")).unwrap();
        AppStore::for_test_database(db, root.path().join("data/objects"))
    } else {
        AppStore::for_test(db, root.path().join("data/objects"))
    };
    let credentials = JsonCredentialStore::from_path(root.path().join("credentials.json"));
    credentials
        .set("deepseek", "synthetic-note-fixture")
        .unwrap();
    let models = AgentModelResolver::from_app_store(&store, credentials);
    Fixture {
        root,
        store,
        models,
        conversation,
        page,
    }
}
fn request(f: &Fixture, id: Uuid) -> PrepareRequest {
    PrepareRequest {
        user_input: vec![],
        conversation_id: f.conversation,
        request_id: id,
        text: "Explain the selected note".into(),
        context: NoteContextRequest {
            page_id: f.page,
            expected_revision: 0,
            block_id: Some("selected".into()),
        },
        retrieval: None,
        model: None,
        session_config: None,
        turn_config: None,
    }
}
fn receipt(f: &Fixture, request_id: Uuid) -> Receipt {
    Receipt {
        conversation_id: f.conversation,
        request_id,
        page_id: f.page,
    }
}
fn count(f: &Fixture) -> i64 {
    f.store
        .connection()
        .unwrap()
        .query_row("select count(*) from agent_submissions", [], |r| r.get(0))
        .unwrap()
}

#[tokio::test]
async fn note_submission_consumes_a_global_retrieval_trace() {
    let f = fixture(true);
    let query = "find related concepts";
    let context = "[CN-S1] retrieved context";
    let trace_id = "note-trace-1";
    f.store
        .connection()
        .unwrap()
        .execute(
            "insert into concept_retrieval_traces
             (id, paper_id, query_sha256, status, config_json, context_text, context_sha256,
              seed_count, edge_count, source_count, external_paper_count, created_at, expires_at)
             values (?1, null, ?2, 'previewed', '{}', ?3, ?4, 0, 0, 0, 0,
                     strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                     strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '+15 minutes'))",
            params![
                trace_id,
                format!("{:x}", Sha256::digest(query.as_bytes())),
                context,
                format!("{:x}", Sha256::digest(context.as_bytes())),
            ],
        )
        .unwrap();
    let mut input = request(&f, Uuid::new_v4());
    input.retrieval = Some(crate::agent::retrieval_context::RetrievalSelection {
        trace_id: trace_id.into(),
        query: query.into(),
    });
    let prepared = prepare(&f.store, &f.models, input).await.unwrap();
    let retrieval = prepared
        .input
        .context
        .iter()
        .find(|source| source.kind == ContextSourceKind::Retrieval)
        .unwrap();
    assert_eq!(retrieval.resource_id, f.page.to_string());
    assert_eq!(retrieval.text, context);
    assert_eq!(
        f.store
            .connection()
            .unwrap()
            .query_row(
                "select status from concept_retrieval_traces where id=?1",
                [trace_id],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
        "sent"
    );
}

#[tokio::test]
async fn session_and_turn_profile_overrides_are_frozen_for_note_submission() {
    let f = fixture(true);
    let config = f
        .store
        .agent_configuration_directory()
        .join("agent/config.toml");
    std::fs::create_dir_all(config.parent().unwrap()).unwrap();
    std::fs::write(
        &config,
        "schema_version=1\n[profiles.note]\ntext='123456789'\nmax_bytes=9\n",
    )
    .unwrap();
    let mut value = request(&f, Uuid::new_v4());
    value.session_config = Some(ConfigOverride {
        max_bytes: Some(6),
        fallback_filenames: None,
    });
    value.turn_config = Some(ConfigOverride {
        max_bytes: Some(3),
        fallback_filenames: None,
    });
    let record = prepare(&f.store, &f.models, value).await.unwrap();
    assert_eq!(record.input.instructions[0].text, "123");
    let layers = &record.input.instruction_configs[0].layers;
    assert_eq!(layers[layers.len() - 2].origin.tier, ConfigTier::Session);
    assert_eq!(layers.last().unwrap().origin.tier, ConfigTier::Turn);
    std::fs::write(&config, "invalid configuration").unwrap();
    assert_eq!(
        restore(&f.store, receipt(&f, record.request_id))
            .await
            .unwrap(),
        record
    );
}

#[tokio::test]
async fn native_note_route_and_source_are_one_idempotent_shared_submission() {
    let f = fixture(true);
    let id = Uuid::new_v4();
    let saved = prepare(&f.store, &f.models, request(&f, id)).await.unwrap();
    assert_eq!(saved.input.model.model_id, "note-fixture");
    assert!(saved.input.context[0].text.contains("Frozen note evidence"));
    assert_eq!(saved.input.context[0].kind, ContextSourceKind::Note);
    assert!(saved.input.instructions.is_empty());
    assert!(saved.input.skills.is_empty());
    assert_eq!(
        prepare(&f.store, &f.models, request(&f, id)).await.unwrap(),
        saved
    );
    assert_eq!(restore(&f.store, receipt(&f, id)).await.unwrap(), saved);
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
            "select revision from note_pages where id=?1",
            [f.page.to_string()],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}

#[tokio::test]
async fn restore_rejects_extra_context_sources_outside_note_and_retrieval() {
    let f = fixture(true);
    let record = prepare(&f.store, &f.models, request(&f, Uuid::new_v4()))
        .await
        .unwrap();
    let mut tampered = record;
    tampered.input.context.push(ContextText {
        source_id: "paper:unexpected".into(),
        kind: ContextSourceKind::Paper,
        resource_id: Uuid::new_v4().to_string(),
        revision: "revision:1".into(),
        anchor: None,
        text: "unexpected paper context".into(),
    });

    assert_eq!(
        validate_record(&tampered, source::Resource::Page(f.page)),
        Err(Error::Unbound)
    );
}

#[tokio::test]
async fn conflicting_retry_never_overwrites_and_restore_ignores_current_source_and_model_defaults()
{
    let f = fixture(true);
    let id = Uuid::new_v4();
    let saved = prepare(&f.store, &f.models, request(&f, id)).await.unwrap();
    let mut changed = request(&f, id);
    changed.text = "Different intent".into();
    assert_eq!(
        prepare(&f.store, &f.models, changed).await,
        Err(Error::Conflict)
    );
    f.store
        .connection()
        .unwrap()
        .execute(
            "update note_pages set revision=1,title='Changed' where id=?1",
            [f.page.to_string()],
        )
        .unwrap();
    assert_eq!(
        prepare(&f.store, &f.models, request(&f, id)).await,
        Err(Error::Conflict)
    );
    f.store
        .connection()
        .unwrap()
        .execute(
            "update llm_provider_configs set enabled=0 where provider_id='deepseek'",
            [],
        )
        .unwrap();
    assert_eq!(restore(&f.store, receipt(&f, id)).await.unwrap(), saved);
    let reopened = AppStore::for_test(
        Connection::open(f.root.path().join("fixture.sqlite")).unwrap(),
        f.root.path().join("data/objects"),
    );
    assert_eq!(restore(&reopened, receipt(&f, id)).await.unwrap(), saved);
    assert_eq!(count(&f), 1);
}

#[tokio::test]
async fn binding_is_checked_before_model_configuration_and_after_note_lifecycle_changes() {
    let f = fixture(false);
    let id = Uuid::new_v4();
    f.store
        .connection()
        .unwrap()
        .execute("update llm_provider_configs set enabled=0", [])
        .unwrap();
    assert_eq!(
        prepare(&f.store, &f.models, request(&f, id)).await,
        Err(Error::Unbound)
    );
    assert_eq!(count(&f), 0);
    let mut invalid = request(&f, id);
    invalid.context.expected_revision = -1;
    assert_eq!(
        prepare(&f.store, &f.models, invalid).await,
        Err(Error::InvalidInput)
    );
    for sql in [
        "update note_pages set status='archived'",
        "update agent_conversations set archived=1",
        "delete from agent_conversation_bindings",
    ] {
        let f = fixture(true);
        let id = Uuid::new_v4();
        prepare(&f.store, &f.models, request(&f, id)).await.unwrap();
        f.store.connection().unwrap().execute(sql, []).unwrap();
        assert_eq!(
            restore(&f.store, receipt(&f, id)).await,
            Err(Error::Unbound)
        );
        assert_eq!(
            prepare(&f.store, &f.models, request(&f, Uuid::new_v4())).await,
            Err(Error::Unbound)
        );
    }
}

#[tokio::test]
async fn another_note_binding_does_not_authorize_relabeling_a_saved_receipt() {
    let f = fixture(true);
    let id = Uuid::new_v4();
    prepare(&f.store, &f.models, request(&f, id)).await.unwrap();
    let other = Uuid::new_v4();
    {
        let db = f.store.connection().unwrap();
        db.execute("insert into note_pages(id,title,created_at,updated_at) values (?1,'Other','now','now')",[other.to_string()]).unwrap();
        db.execute("insert into agent_conversation_bindings(conversation_id,resource_kind,resource_id) values (?1,'note',?2)",params![f.conversation.to_string(),other.to_string()]).unwrap();
    }
    let mut wrong = receipt(&f, id);
    wrong.page_id = other;
    assert_eq!(restore(&f.store, wrong).await, Err(Error::Unbound));
}

#[tokio::test]
async fn failed_commit_creates_no_partial_submission_and_can_retry() {
    let f = fixture(true);
    let id = Uuid::new_v4();
    f.store.connection().unwrap().execute_batch("create trigger reject_note_submission before insert on agent_submissions begin select raise(abort,'synthetic failure'); end;").unwrap();
    assert_eq!(
        prepare(&f.store, &f.models, request(&f, id)).await,
        Err(Error::Unavailable)
    );
    assert_eq!(count(&f), 0);
    f.store
        .connection()
        .unwrap()
        .execute_batch("drop trigger reject_note_submission;")
        .unwrap();
    prepare(&f.store, &f.models, request(&f, id)).await.unwrap();
    assert_eq!(count(&f), 1);
}

#[tokio::test]
async fn serialized_preview_exposes_frozen_input_without_credentials_or_tool_grants() {
    let f = fixture(true);
    let id = Uuid::new_v4();
    let saved = prepare(&f.store, &f.models, request(&f, id)).await.unwrap();
    let json = serde_json::to_value(crate::agent::note_commands::Preview::from_record(
        saved, f.page,
    ))
    .unwrap();
    assert_eq!(json.as_object().unwrap().len(), 7);
    assert_eq!(json["schemaVersion"], 1);
    assert_eq!(json["pageId"], f.page.to_string());
    assert_eq!(json["input"]["context"][0]["kind"], "note");
    assert!(!json.to_string().contains("synthetic-note-fixture"));
    assert!(!json.to_string().contains("credentials.json"));
    let db_json: String = f
        .store
        .connection()
        .unwrap()
        .query_row("select input_json from agent_submissions", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert!(!db_json.contains("synthetic-note-fixture"));
}

#[test]
fn note_command_contracts_refuse_renderer_authority_fields() {
    let base = serde_json::json!({"conversationId":Uuid::new_v4(),"requestId":Uuid::new_v4(),"text":"Explain",
        "context":{"pageId":Uuid::new_v4(),"expectedRevision":0,"blockId":null},"model":null});
    assert!(serde_json::from_value::<PrepareRequest>(base.clone()).is_ok());
    let mut extra = base.clone();
    extra["tools"] = serde_json::json!(["shell"]);
    assert!(serde_json::from_value::<PrepareRequest>(extra).is_err());
    let mut extra_model = base;
    extra_model["model"] = serde_json::json!({"providerId":"deepseek","modelId":"fixture","reasoningEffort":null,"credential":"forged"});
    assert!(serde_json::from_value::<PrepareRequest>(extra_model).is_err());
    assert!(serde_json::from_value::<Receipt>(serde_json::json!({"conversationId":Uuid::new_v4(),"requestId":Uuid::new_v4(),"pageId":Uuid::new_v4(),"text":"override"})).is_err());
}
