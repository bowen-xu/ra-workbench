use super::*;
fn db() -> Connection {
    let mut db = Connection::open_in_memory().unwrap();
    crate::storage::migrate(&mut db).unwrap();
    db
}
#[test]
fn independent_preparation_follows_the_callers_transaction_boundary() {
    let root = tempfile::tempdir().unwrap();
    let connection = db();
    let conversation = Uuid::new_v4();
    let request_id = Uuid::new_v4();
    connection
        .execute(
            "insert into agent_conversations(id,title) values (?1,'Independent')",
            [conversation.to_string()],
        )
        .unwrap();
    let store = AppStore::for_test(connection, root.path().join("objects"));
    let request = || {
        serde_json::from_value::<PrepareRequest>(serde_json::json!({"conversationId":conversation,"requestId":request_id,"text":"Synthetic question","resources":[],"model":null,"retrieval":null,"queue":null})).unwrap()
    };
    let model = || {
        (
            research_agent_core::protocol::ModelIdentity {
                provider_id: "fixture".into(),
                model_id: "fixture".into(),
                revision: None,
            },
            None,
            None,
        )
    };
    let mut db = store.connection().unwrap();
    let tx = db.transaction().unwrap();
    let first = materialize_in(&tx, &store, request(), model()).unwrap();
    assert_eq!(first.request_id, request_id);
    tx.rollback().unwrap();
    assert_eq!(
        db.query_row::<u64, _, _>("select count(*) from agent_submissions", [], |row| row
            .get(0))
            .unwrap(),
        0
    );
    let tx = db.transaction().unwrap();
    let committed = materialize_in(&tx, &store, request(), model()).unwrap();
    tx.commit().unwrap();
    let saved = submissions::read(&db, conversation, request_id)
        .unwrap()
        .unwrap();
    assert_eq!(saved.turn_id, committed.turn_id);
    assert_eq!(saved.input.text, "Synthetic question");
}

#[test]
fn independent_queued_preparation_rolls_back_with_the_submission() {
    use crate::agent::repository::input_queue;
    use research_agent_core::input_queue::{QueueEdit, QueuedInput};
    let root = tempfile::tempdir().unwrap();
    let connection = db();
    let conversation = Uuid::new_v4();
    let input_id = Uuid::new_v4();
    let request_id = Uuid::new_v4();
    connection
        .execute(
            "insert into agent_conversations(id,title) values (?1,'Independent')",
            [conversation.to_string()],
        )
        .unwrap();
    let store = AppStore::for_test(connection, root.path().join("objects"));
    let mut db = store.connection().unwrap();
    input_queue::apply_checked(
        &mut db,
        conversation,
        Uuid::new_v4(),
        0,
        QueueEdit::Add(QueuedInput {
            id: input_id,
            text: "queued".into(),
            user_input: vec![],
            origin: None,
        }),
        |connection| queue::authorize(connection, conversation),
    )
    .unwrap();
    let request = || {
        serde_json::from_value::<PrepareRequest>(serde_json::json!({"conversationId":conversation,"requestId":request_id,"text":"queued","resources":[],"model":null,"retrieval":null,"queue":{"inputId":input_id,"expectedRevision":1}})).unwrap()
    };
    let model = || {
        (
            research_agent_core::protocol::ModelIdentity {
                provider_id: "fixture".into(),
                model_id: "fixture".into(),
                revision: None,
            },
            None,
            None,
        )
    };
    let tx = db.transaction().unwrap();
    let preview = materialize_in(&tx, &store, request(), model()).unwrap();
    assert_eq!(preview.queue.unwrap().input_id, input_id);
    tx.rollback().unwrap();
    let queue = input_queue::read_checked(&mut db, conversation, |connection| {
        queue::authorize(connection, conversation)
    })
    .unwrap();
    assert_eq!(queue.revision(), 1);
    assert!(queue.preparing().is_none());
    assert!(
        submissions::read(&db, conversation, request_id)
            .unwrap()
            .is_none()
    );
    let tx = db.transaction().unwrap();
    let preview = materialize_in(&tx, &store, request(), model()).unwrap();
    tx.commit().unwrap();
    assert_eq!(preview.queue.unwrap().expected_revision, 2);
    let queue = input_queue::read_checked(&mut db, conversation, |connection| {
        queue::authorize(connection, conversation)
    })
    .unwrap();
    assert_eq!(queue.preparing(), Some(input_id));
}
#[test]
fn conversations_do_not_require_resources_and_directory_configuration_selects_the_adapter() {
    let db = db();
    let id = Uuid::new_v4();
    db.execute("insert into agent_conversations(id,title,archived,durable_count,created_at,updated_at) values (?1,'Independent',0,0,'2026-10-10','2026-10-10')",[id.to_string()]).unwrap();
    assert_eq!(check_conversation(&db, id), Ok(()));
    db.execute("insert into agent_conversation_bindings(conversation_id,resource_kind,resource_id) values (?1,'paper','other-paper')",[id.to_string()]).unwrap();
    assert_eq!(check_conversation(&db, id), Ok(()));
    db.execute("insert into agent_conversation_bindings(conversation_id,resource_kind,resource_id) values (?1,'manuscript','editable-directory')",[id.to_string()]).unwrap();
    assert_eq!(
        check_conversation(&db, id),
        Err(Error::ConversationUnavailable)
    );
    db.execute("delete from agent_conversation_bindings", [])
        .unwrap();
    db.execute("update agent_conversations set archived=1", [])
        .unwrap();
    assert_eq!(
        check_conversation(&db, id),
        Err(Error::ConversationUnavailable)
    );
    assert_eq!(
        check_conversation(&db, Uuid::new_v4()),
        Err(Error::ConversationUnavailable)
    );
}
#[test]
fn pdf_grants_load_only_current_docling_on_read_without_index_fallback() {
    let db = db();
    let root = tempfile::tempdir().unwrap();
    let hash = "a".repeat(64);
    db.execute("insert into bibliographic_items(id,item_type,title,created_at,updated_at) values ('paper','document','Synthetic','now','now')",[]).unwrap();
    db.execute("insert into content_objects(sha256,relative_path,mime_type,byte_size,created_at) values (?1,'synthetic.pdf','application/pdf',100,'now')",[&hash]).unwrap();
    db.execute("insert into attachments(id,item_id,content_sha256,display_name,created_at) values ('attachment','paper',?1,'Synthetic.pdf','now')",[&hash]).unwrap();
    db.execute(
        "insert into attachment_page_texts values ('attachment',0,'Do not use indexed text','now')",
        [],
    )
    .unwrap();
    let store = AppStore::for_test(db, root.path().join("objects"));
    std::fs::create_dir_all(store.content_directory()).unwrap();
    std::fs::write(store.content_directory().join("synthetic.pdf"), b"fixture").unwrap();
    let resource = sources::Resource::Paper {
        resource_id: "paper".into(),
        attachment_id: Some("attachment".into()),
        page_index: None,
    };
    let db = store.connection().unwrap();
    assert!(
        resource_grants::grants(
            &db,
            std::slice::from_ref(&resource),
            Some(crate::agent::writing_resources::test_sources::source())
        )
        .is_ok()
    );
    assert_eq!(
        sources::materialize(&db, &store, std::slice::from_ref(&resource)),
        Err(Error::PaperNotParsed)
    );
    let document = serde_json::json!({"texts":[{"self_ref":"#/texts/0","text":"Docling evidence","prov":[{"page_no":1}]}],"tables":[{"data":{"table_cells":[{"text":"Cell evidence"}]}}]});
    db.execute("insert into docling_parse_runs(id,item_id,attachment_id,content_sha256,service_base_url,docling_status,parse_status,json_content,raw_response,created_at,finished_at) values ('parse','paper','attachment',?1,'fixture','success','success',?2,'{}','now','now')",rusqlite::params![hash,document.to_string()]).unwrap();
    let selected = sources::materialize(&db, &store, std::slice::from_ref(&resource)).unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&selected[0].text).unwrap(),
        document
    );
    assert!(!selected[0].text.contains("indexed text"));
    let page = sources::Resource::Paper {
        resource_id: "paper".into(),
        attachment_id: Some("attachment".into()),
        page_index: Some(0),
    };
    let scoped = sources::materialize(&db, &store, &[page]).unwrap();
    assert!(scoped[0].text.contains("Docling evidence"));
    assert!(!scoped[0].text.contains("Cell evidence"));
    db.execute("insert into content_objects(sha256,relative_path,mime_type,byte_size,created_at) values (?1,'changed.pdf','application/pdf',100,'now')",["b".repeat(64)]).unwrap();
    db.execute(
        "update attachments set content_sha256=?1 where id='attachment'",
        ["b".repeat(64)],
    )
    .unwrap();
    assert_eq!(
        sources::materialize(&db, &store, &[resource]),
        Err(Error::PaperNotParsed)
    );
}
#[test]
fn source_deserialization_rejects_client_content_or_arbitrary_paths() {
    assert!(
        serde_json::from_value::<sources::Resource>(
            serde_json::json!({"kind":"paper","resourceId":"x","text":"injected"})
        )
        .is_err()
    );
    assert!(
        serde_json::from_value::<sources::Resource>(
            serde_json::json!({"kind":"unknown","resourceId":"x"})
        )
        .is_err()
    );
}
#[tokio::test]
async fn reopening_restores_frozen_note_content_and_isolates_receipts() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("context.sqlite");
    let mut db = Connection::open(&path).unwrap();
    crate::storage::migrate(&mut db).unwrap();
    let id = Uuid::new_v4();
    let other = Uuid::new_v4();
    for conversation in [id, other] {
        db.execute(
            "insert into agent_conversations(id,title) values (?1,'Independent')",
            [conversation.to_string()],
        )
        .unwrap();
    }
    let note_id = Uuid::new_v4();
    db.execute("insert into note_pages(id,title,created_at,updated_at) values (?1,'Original saved note','now','now')",[note_id.to_string()]).unwrap();
    let store = AppStore::for_test(db, root.path().join("objects"));
    let request = Uuid::new_v4();
    let frozen = {
        let mut db = store.connection().unwrap();
        let tx = db.transaction().unwrap();
        let context = sources::materialize(
            &tx,
            &store,
            &[sources::Resource::Note {
                resource_id: note_id,
            }],
        )
        .unwrap();
        let input = SubmissionInput {
            remote_compaction_v2: None,
            text: "Read this note".into(),
            user_input: vec![],
            skill_instructions: vec![],
            model: research_agent_core::protocol::ModelIdentity {
                provider_id: "fixture".into(),
                model_id: "fixture".into(),
                revision: None,
            },
            reasoning_effort: None,
            service_tier: None,
            context,
            instructions: vec![],
            instruction_configs: vec![],
            skills: vec![],
        };
        let record = submissions::insert_in(&tx, id, request, input).unwrap();
        tx.commit().unwrap();
        record
    };
    store
        .connection()
        .unwrap()
        .execute(
            "update note_pages set title='New saved version',revision=revision+1 where id=?1",
            [note_id.to_string()],
        )
        .unwrap();
    drop(store);
    let reopened = AppStore::for_test(
        Connection::open(&path).unwrap(),
        root.path().join("objects"),
    );
    let restored = restore(
        &reopened,
        Receipt {
            conversation_id: id,
            request_id: request,
        },
    )
    .await
    .unwrap();
    assert_eq!(restored, frozen);
    assert!(
        restored.input.context[0]
            .text
            .contains("Original saved note")
    );
    let refreshed = sources::materialize(
        &reopened.connection().unwrap(),
        &reopened,
        &[sources::Resource::Note {
            resource_id: note_id,
        }],
    )
    .unwrap();
    assert!(refreshed[0].text.contains("New saved version"));
    assert_ne!(refreshed[0].revision, restored.input.context[0].revision);
    assert_eq!(
        restore(
            &reopened,
            Receipt {
                conversation_id: other,
                request_id: request
            }
        )
        .await
        .unwrap_err(),
        Error::Unavailable
    );
}
