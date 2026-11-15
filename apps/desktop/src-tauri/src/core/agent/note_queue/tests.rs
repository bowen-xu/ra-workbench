use super::*;
use crate::agent::repository::create_conversation;
use research_agent_core::conversation::ConversationId;
use rusqlite::params;
use serde_json::{Value, json};

struct Fixture {
    root: tempfile::TempDir,
    store: AppStore,
    conversation: Uuid,
    resource: Uuid,
}
impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let mut db = Connection::open(root.path().join("queue.sqlite")).unwrap();
        crate::storage::migrate(&mut db).unwrap();
        let conversation = Uuid::new_v4();
        let resource = Uuid::new_v4();
        create_conversation(
            &mut db,
            ConversationId::from_uuid(conversation),
            "Synthetic queue",
        )
        .unwrap();
        db.execute("insert into note_pages(id,title,created_at,updated_at) values (?1,'Synthetic','now','now')", [resource.to_string()]).unwrap();
        crate::notes::insert_reading_fixture(&db, &resource.to_string(), "Queue source");
        for kind in ["note", "readingNote"] {
            db.execute("insert into agent_conversation_bindings(conversation_id,resource_kind,resource_id) values (?1,?2,?3)", params![conversation.to_string(),kind,resource.to_string()]).unwrap();
        }
        let db = crate::data_interface::Database::open(db, &root.path().join("log")).unwrap();
        let store = AppStore::for_test_database(db, root.path().join("data/objects"));
        Self {
            root,
            store,
            conversation,
            resource,
        }
    }
    fn scope(&self, reading: bool) -> Scope {
        if reading {
            Scope::ReadingNote {
                resource_id: self.resource,
            }
        } else {
            Scope::Note {
                resource_id: self.resource,
            }
        }
    }
    fn reopen(self) -> Self {
        let Self {
            root,
            store,
            conversation,
            resource,
        } = self;
        drop(store);
        let db = crate::data_interface::Database::open(
            Connection::open(root.path().join("queue.sqlite")).unwrap(),
            &root.path().join("log"),
        )
        .unwrap();
        let store = AppStore::for_test_database(db, root.path().join("data/objects"));
        Self {
            root,
            store,
            conversation,
            resource,
        }
    }
    async fn read(&self, reading: bool) -> Result<Value, CommandError> {
        read(
            &self.store,
            ReadRequest {
                conversation_id: self.conversation,
                scope: self.scope(reading),
            },
        )
        .await
        .map(|r| serde_json::to_value(r).unwrap())
    }
    fn request(&self, id: Uuid, revision: u64, reading: bool, action: Value) -> EditRequest {
        serde_json::from_value(
            json!({"conversationId":self.conversation,"scope":self.scope(reading),
            "requestId":id,"expectedRevision":revision,"edit":action}),
        )
        .unwrap()
    }
}

#[tokio::test]
async fn guarded_note_edits_preserve_epoch_across_reopen_and_reject_other_environments() {
    for reading in [false, true] {
        let f = Fixture::new();
        let add_id = Uuid::new_v4();
        let input = Uuid::new_v4();
        let action = json!({"type":"add","inputId":input,"text":"Original queue item"});
        edit(&f.store, f.request(add_id, 0, reading, action.clone()))
            .await
            .unwrap();
        let status = edit_status(&f.store, f.request(add_id, 0, reading, action))
            .await
            .unwrap();
        let epoch: Uuid = serde_json::from_value(
            serde_json::to_value(status).unwrap()["status"]["proof"]["epoch"].clone(),
        )
        .unwrap();
        let remove = json!({"type":"remove","inputId":input});
        let request_id = Uuid::new_v4();
        let mut wrong = f.request(request_id, 1, reading, remove.clone());
        wrong.expected_epoch = Some(Uuid::new_v4());
        assert!(matches!(
            edit(&f.store, wrong).await,
            Err(CommandError::Storage(_))
        ));
        assert_eq!(
            f.read(reading).await.unwrap()["inputs"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        let mut correct = f.request(request_id, 1, reading, remove.clone());
        correct.expected_epoch = Some(epoch);
        edit(&f.store, correct).await.unwrap();
        let f = f.reopen();
        let mut replay = f.request(request_id, 1, reading, remove.clone());
        replay.expected_epoch = Some(epoch);
        let replay = serde_json::to_value(edit_status(&f.store, replay).await.unwrap()).unwrap();
        assert_eq!(replay["status"]["type"], "committed");
        assert_eq!(replay["status"]["proof"]["epoch"], epoch.to_string());
        let mut mismatch = f.request(request_id, 1, reading, remove);
        mismatch.expected_epoch = Some(Uuid::new_v4());
        assert!(matches!(
            edit_status(&f.store, mismatch).await,
            Err(CommandError::Storage(_))
        ));
        assert!(
            f.read(reading).await.unwrap()["inputs"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }
}

#[tokio::test]
async fn both_note_hosts_share_one_conversation_queue_and_reopen_without_model_configuration() {
    let f = Fixture::new();
    assert_eq!(f.read(false).await.unwrap()["inputs"], json!([]));
    let request = Uuid::new_v4();
    let input = Uuid::new_v4();
    let action = json!({"type":"add","inputId":input,"text":"Review retained evidence"});
    let saved = edit(&f.store, f.request(request, 0, false, action.clone()))
        .await
        .unwrap();
    let replay = edit(&f.store, f.request(request, 0, false, action))
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(saved).unwrap(),
        serde_json::to_value(replay).unwrap()
    );
    let reading = f.read(true).await.unwrap();
    assert_eq!(reading["scope"]["kind"], "readingNote");
    assert_eq!(reading["revision"], 1);
    assert_eq!(reading["inputs"][0]["inputId"], input.to_string());
    assert_eq!(
        reading["inputs"][0]["origin"],
        serde_json::to_value(f.scope(false)).unwrap()
    );
    assert!(matches!(
        edit(
            &f.store,
            f.request(
                request,
                0,
                true,
                json!({"type":"add","inputId":input,"text":"Review retained evidence"})
            )
        )
        .await,
        Err(CommandError::Storage(error)) if serde_json::to_value(&error).unwrap()["reason"]["code"]=="DATA_SQL_OPERATION_CONFLICT"
    ));
    let f = f.reopen();
    let reopened = read(
        &f.store,
        ReadRequest {
            conversation_id: f.conversation,
            scope: f.scope(true),
        },
    )
    .await
    .unwrap();
    assert_eq!(serde_json::to_value(reopened).unwrap(), reading);
    let db = f.store.connection().unwrap();
    for table in ["agent_submissions", "agent_items"] {
        assert_eq!(
            db.query_row(&format!("select count(*) from {table}"), [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}

#[tokio::test]
async fn revoked_binding_blocks_reads_and_idempotent_replays_even_with_same_id_in_other_note_table()
{
    for reading in [false, true] {
        let f = Fixture::new();
        let request = Uuid::new_v4();
        let action = json!({"type":"add","inputId":Uuid::new_v4(),"text":"Retain this"});
        edit(&f.store, f.request(request, 0, reading, action.clone()))
            .await
            .unwrap();
        let kind = if reading { "readingNote" } else { "note" };
        assert_eq!(
            f.read(!reading).await.unwrap()["inputs"][0]["origin"]["kind"],
            kind
        );
        f.store
            .connection()
            .unwrap()
            .execute(
                "delete from agent_conversation_bindings where resource_kind=?1",
                [kind],
            )
            .unwrap();
        assert!(matches!(f.read(reading).await, Err(CommandError::Closed)));
        assert!(matches!(
            edit(&f.store, f.request(request, 0, reading, action)).await,
            Err(CommandError::Closed)
        ));
        assert_eq!(
            f.read(!reading).await.unwrap()["inputs"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }
}

#[tokio::test]
async fn archived_notes_and_conversations_cannot_read_or_write_queues() {
    for (reading, sql) in [
        (false, "update note_pages set status='archived'"),
        (
            true,
            "update note_pages set status='archived' where id in (select page_id from reading_note_sources)",
        ),
        (false, "update agent_conversations set archived=1"),
        (true, "delete from reading_note_sources"),
    ] {
        let f = Fixture::new();
        f.store.connection().unwrap().execute(sql, []).unwrap();
        assert!(matches!(f.read(reading).await, Err(CommandError::Closed)));
        assert!(matches!(
            edit(
                &f.store,
                f.request(Uuid::new_v4(), 0, reading, json!({"type":"pause"}))
            )
            .await,
            Err(CommandError::Closed)
        ));
    }
}

#[tokio::test]
async fn concurrent_hosts_use_revision_conflicts_and_retry_receipts_instead_of_overwriting() {
    let f = Fixture::new();
    let one = f.request(
        Uuid::new_v4(),
        0,
        false,
        json!({"type":"add","inputId":Uuid::new_v4(),"text":"one"}),
    );
    let two = f.request(
        Uuid::new_v4(),
        0,
        true,
        json!({"type":"add","inputId":Uuid::new_v4(),"text":"two"}),
    );
    let (one, two) = tokio::join!(edit(&f.store, one), edit(&f.store, two));
    assert!(matches!(
        (one, two),
        (Ok(_), Err(CommandError::Conflict)) | (Err(CommandError::Conflict), Ok(_))
    ));
    assert_eq!(
        f.read(false).await.unwrap()["inputs"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let id = Uuid::new_v4();
    edit(&f.store, f.request(id, 1, true, json!({"type":"pause"})))
        .await
        .unwrap();
    assert!(matches!(
        edit(&f.store, f.request(id, 1, true, json!({"type":"resume"}))).await,
        Err(CommandError::Storage(error)) if serde_json::to_value(&error).unwrap()["reason"]["code"]=="DATA_SQL_OPERATION_CONFLICT"
    ));
    assert_eq!(f.read(false).await.unwrap()["paused"], true);
}

#[tokio::test]
async fn invalid_identifiers_and_unsafe_javascript_revisions_are_rejected() {
    let f = Fixture::new();
    for (id, revision) in [(Uuid::nil(), 0), (Uuid::new_v4(), 9_007_199_254_740_991)] {
        assert!(matches!(
            edit(
                &f.store,
                f.request(id, revision, false, json!({"type":"pause"}))
            )
            .await,
            Err(CommandError::Invalid)
        ));
    }
    assert!(matches!(
        read(
            &f.store,
            ReadRequest {
                conversation_id: f.conversation,
                scope: Scope::Note {
                    resource_id: Uuid::nil()
                }
            }
        )
        .await,
        Err(CommandError::Invalid)
    ));
    assert!(matches!(
        read(
            &f.store,
            ReadRequest {
                conversation_id: Uuid::nil(),
                scope: f.scope(false)
            }
        )
        .await,
        Err(CommandError::Invalid)
    ));
}

#[test]
fn queue_wire_excludes_runtime_authority_and_requires_explicit_note_kind() {
    for kind in ["prepare", "admitted", "release"] {
        assert!(
            serde_json::from_value::<Edit>(json!({"type":kind,"inputId":Uuid::new_v4()})).is_err()
        );
    }
    for scope in [
        json!({"resourceId":Uuid::new_v4()}),
        json!({"kind":"manuscript","resourceId":Uuid::new_v4()}),
        json!({"kind":"note","resourceId":Uuid::new_v4(),"grants":["shell"]}),
    ] {
        assert!(serde_json::from_value::<Scope>(scope).is_err());
    }
    let mut request = json!({"conversationId":Uuid::new_v4(),"scope":{"kind":"note","resourceId":Uuid::new_v4()},"requestId":Uuid::new_v4(),"expectedRevision":0,"edit":{"type":"pause"}});
    assert!(serde_json::from_value::<EditRequest>(request.clone()).is_ok());
    request["model"] = json!({"providerId":"forged"});
    assert!(serde_json::from_value::<EditRequest>(request).is_err());
}
