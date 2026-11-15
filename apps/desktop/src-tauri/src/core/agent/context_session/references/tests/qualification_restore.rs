use super::fixture::{self, Fixture};
use crate::agent::{
    context_session::{self, Receipt, resource_tools},
    repository::{SqliteSubmissionStore, submissions},
    source_tools::OwnedSubmissionTools,
};
use research_agent_core::{
    model::ToolCall,
    submission::{SubmissionInput, SubmissionStore},
    tool_registry::OwnedToolRegistration,
};
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

#[tokio::test]
async fn reopened_saved_legacy_sources_are_restored_and_read_without_rematerializing() {
    let Fixture {
        root,
        store,
        conversation,
        note,
        ..
    } = Fixture::new();
    let request_id = Uuid::new_v4();
    let saved = {
        let mut db = store.connection().unwrap();
        let tx = db.transaction().unwrap();
        let note_source = crate::notes::agent_context::materialize_in(
            &tx,
            &crate::notes::agent_context::NoteContextRequest {
                page_id: note,
                expected_revision: 1,
                block_id: None,
            },
        )
        .unwrap();
        assert!(note_source.text.contains(fixture::NOTE_BODY));
        // An explicit historical-record fixture, not a current preparation/UI
        // route. The original submission repository owns its wire persistence.
        let portrait_source = context_session::catalog::source(
            "paper",
            "portrait",
            "Historical portrait 原文 Ω".into(),
        );
        let record = submissions::insert_in(
            &tx,
            conversation,
            request_id,
            SubmissionInput {
                text: "Read the saved frozen sources".into(),
                user_input: vec![],
                skill_instructions: vec![],
                model: fixture::model().0,
                reasoning_effort: None,
                service_tier: None,
                remote_compaction_v2: None,
                context: vec![note_source, portrait_source],
                instructions: vec![],
                instruction_configs: vec![],
                skills: vec![],
            },
        )
        .unwrap();
        fixture::save_note(&tx, note, 1, "New current body after freezing");
        tx.commit().unwrap();
        record
    };
    let database = store.shared_agent_connection();
    drop(store);
    database.shutdown().unwrap();
    drop(database);
    let reopened = fixture::open(root.path());
    let restored = context_session::restore_context(
        &reopened,
        Receipt {
            conversation_id: conversation,
            request_id,
        },
    )
    .await
    .unwrap();
    assert_eq!(restored.input, saved.input);
    assert_eq!(restored.turn_id, saved.turn_id);
    assert_eq!(restored.context_snapshot_id, saved.context_snapshot_id);
    assert!(restored.queue.is_none());
    let record = SqliteSubmissionStore::for_app(&reopened)
        .get(conversation, request_id)
        .await
        .unwrap();
    assert_eq!(record, saved);
    let frozen = OwnedSubmissionTools::new(record.clone());
    let (_, executor) = resource_tools::registration(
        reopened.clone(),
        record.clone(),
        OwnedToolRegistration::new(frozen.definitions(), Arc::new(frozen)),
    )
    .into_parts();
    for source in &record.input.context {
        let mut first = None;
        for _ in 0..2 {
            let output = executor
                .authorize_and_execute(ToolCall {
                    call_id: Uuid::new_v4().to_string(),
                    namespace: None,
                    name: "research_read_source".into(),
                    arguments: json!({"sourceId":source.source_id,"maxChars":8000}),
                    raw_arguments_json: None,
                })
                .await
                .unwrap();
            assert!(!output.is_error, "{}", output.content);
            let value: serde_json::Value = serde_json::from_str(&output.content).unwrap();
            assert_eq!(value["text"], source.text);
            assert_eq!(value["sourceId"], source.source_id);
            assert_eq!(value["resourceId"], source.resource_id);
            assert_eq!(value["revision"], source.revision);
            assert_eq!(value["turnId"], record.turn_id.to_string());
            assert_eq!(
                value["contextSnapshotId"],
                record.context_snapshot_id.to_string()
            );
            assert!(
                value["readViewId"]
                    .as_str()
                    .is_some_and(|id| !id.is_empty())
            );
            if let Some(previous) = first.as_ref() {
                assert_eq!(&value, previous);
            }
            first = Some(value);
        }
    }
    let db = reopened.connection().unwrap();
    let current = crate::notes::load_document_value(&db, &note.to_string()).unwrap();
    assert!(
        serde_json::to_string(&current)
            .unwrap()
            .contains("New current body after freezing")
    );
    assert_eq!(
        db.query_row::<i64, _, _>("select count(*) from agent_submissions", [], |row| row
            .get(0))
            .unwrap(),
        1
    );
}
