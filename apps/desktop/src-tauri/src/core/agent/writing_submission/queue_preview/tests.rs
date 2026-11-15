use super::super::tests::{fixture, managed_fixture, queue_preview_intent, request};
use super::*;
use research_agent_core::input_queue::{QueueEdit, QueuedInput};

#[tokio::test]
async fn queue_preview_commands_return_only_disclosure_and_scoped_queue() {
    use crate::agent::writing_queue_preview::{self, CancelRequest, QueuePreviewRequest};
    let f = managed_fixture(true);
    let service = WritingSubmissionService::from_app_store(&f.store);
    let id = Uuid::new_v4();
    let request_id = Uuid::new_v4();
    service
        .edit_input_queue(
            f.manuscript,
            f.conversation,
            Uuid::new_v4(),
            0,
            QueueEdit::Add(QueuedInput {
                user_input: vec![],
                origin: None,
                id,
                text: "Explain".into(),
            }),
        )
        .await
        .unwrap();
    let command:QueuePreviewRequest=serde_json::from_value(serde_json::json!({
        "inputId":id,"expectedRevision":1,"submission":{"conversationId":f.conversation,"requestId":request_id,"text":"Explain",
            "context":{"manuscriptId":f.manuscript,"files":[{"relativePath":"main.tex","expectedDiskSha256":f.hash}]},"model":null}
    })).unwrap();
    let json = serde_json::to_value(
        writing_queue_preview::prepare(&service, &f.models, command)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(json.as_object().unwrap().len(), 4);
    assert_eq!(json["preview"].as_object().unwrap().len(), 7);
    assert_eq!(json["inputId"], id.to_string());
    assert_eq!(json["queue"]["preparingInputId"], id.to_string());
    assert_eq!(json["queue"]["revision"], 2);
    assert_eq!(json["preview"]["requestId"], request_id.to_string());
    assert!(!json.to_string().contains("synthetic-fixture"));
    let restored = writing_queue_preview::read_pending(
        &service,
        crate::agent::writing_queue::ReadRequest {
            conversation_id: f.conversation,
            manuscript_id: f.manuscript,
        },
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(serde_json::to_value(restored).unwrap(), json);
    assert!(
        writing_queue_preview::read_pending(
            &service,
            crate::agent::writing_queue::ReadRequest {
                conversation_id: f.conversation,
                manuscript_id: Uuid::new_v4(),
            }
        )
        .await
        .is_err()
    );
    let result = writing_queue_preview::cancel(
        &service,
        CancelRequest {
            conversation_id: f.conversation,
            manuscript_id: f.manuscript,
            request_id: Uuid::new_v4(),
            expected_revision: 2,
            input_id: id,
            preview_request_id: request_id,
        },
    )
    .await
    .unwrap();
    let json = serde_json::to_value(result).unwrap();
    assert_eq!(json["paused"], true);
    assert_eq!(json["preparingInputId"], serde_json::Value::Null);
    assert_eq!(json["inputs"][0]["text"], "Explain");
    assert!(
        writing_queue_preview::read_pending(
            &service,
            crate::agent::writing_queue::ReadRequest {
                conversation_id: f.conversation,
                manuscript_id: f.manuscript,
            }
        )
        .await
        .unwrap()
        .is_none()
    );
}

#[tokio::test]
async fn ordinary_start_cannot_bypass_queue_confirmation_even_after_cancel() {
    use crate::agent::{
        repository::SqliteConversationStore,
        service::AgentSessionService,
        writing_start::{self, StartError, StartRequest, WritingExecutionMode},
    };
    use std::sync::Arc;
    let f = managed_fixture(true);
    let service = WritingSubmissionService::from_app_store(&f.store);
    let id = Uuid::new_v4();
    let preview = Uuid::new_v4();
    let mut input = request(&f, preview);
    input.model = Some(AgentModelSelection {
        provider_id: "deepseek".into(),
        model_id: "deepseek-v4-pro".into(),
        reasoning_effort: None,
        service_tier: None,
    });
    service
        .edit_input_queue(
            f.manuscript,
            f.conversation,
            Uuid::new_v4(),
            0,
            QueueEdit::Add(QueuedInput {
                user_input: vec![],
                origin: None,
                id,
                text: input.text.clone(),
            }),
        )
        .await
        .unwrap();
    let record = service
        .prepare_queue_preview(&f.models, queue_preview_intent(&input, id, 1), input)
        .await
        .unwrap()
        .record;
    let sessions = Arc::new(AgentSessionService::new(Arc::new(
        SqliteConversationStore::new(f.store.shared_agent_connection(), "fixture".into()).unwrap(),
    )));
    for canceled in [false, true] {
        if canceled {
            service
                .cancel_queue_preview(f.manuscript, f.conversation, Uuid::new_v4(), 2, id, preview)
                .await
                .unwrap();
        }
        // Simulate migration from the prior schema for both live and cancelled
        // previews. Cancellation removed the live link, so receipts must backfill it.
        {
            let db = f.store.connection().unwrap();
            db.execute_batch("drop table agent_queue_submission_origins")
                .unwrap();
            let sql = crate::storage_migrations::MIGRATIONS
                .iter()
                .find(|(version, _)| *version == 76)
                .unwrap()
                .1;
            db.execute_batch(sql).unwrap();
        }
        let result = writing_start::start(
            &service,
            &f.models,
            sessions.clone(),
            StartRequest {
                conversation_id: f.conversation,
                request_id: preview,
                manuscript_id: f.manuscript,
                context_snapshot_id: record.context_snapshot_id,
                model: record.input.model.clone(),
                mode: WritingExecutionMode::DeepSeekNonThinking,
            },
        )
        .await;
        assert!(matches!(
            result,
            Err(StartError::Preparation(
                crate::agent::writing_commands::PreviewError::Conflict
            ))
        ));
    }
    assert_eq!(
        f.store
            .connection()
            .unwrap()
            .query_row("select count(*) from agent_items", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn queued_preview_freezes_manuscript_and_cancel_keeps_original_text() {
    let f = managed_fixture(true);
    let service = WritingSubmissionService::from_app_store(&f.store);
    let input_id = Uuid::new_v4();
    let preview_id = Uuid::new_v4();
    let input = request(&f, preview_id);
    let text = input.text.clone();
    service
        .edit_input_queue(
            f.manuscript,
            f.conversation,
            Uuid::new_v4(),
            0,
            QueueEdit::Add(QueuedInput {
                user_input: vec![],
                origin: None,
                id: input_id,
                text: text.clone(),
            }),
        )
        .await
        .unwrap();
    let preview = service
        .prepare_queue_preview(&f.models, queue_preview_intent(&input, input_id, 1), input)
        .await
        .unwrap();
    assert_eq!(preview.queue.inputs()[0].text, text);
    assert_eq!(preview.queue.preparing(), Some(input_id));
    validate_scope(&preview.record, f.manuscript).unwrap();
    assert!(!preview.record.input.context.is_empty());
    let replay = request(&f, preview_id);
    let again = service
        .prepare_queue_preview(
            &f.models,
            queue_preview_intent(&replay, input_id, 1),
            replay,
        )
        .await
        .unwrap();
    assert_eq!(again.record, preview.record);
    let cancel = Uuid::new_v4();
    let queue = service
        .cancel_queue_preview(
            f.manuscript,
            f.conversation,
            cancel,
            2,
            input_id,
            preview_id,
        )
        .await
        .unwrap();
    assert!(queue.paused());
    assert_eq!(queue.inputs()[0].text, text);
    assert_eq!(
        service
            .cancel_queue_preview(
                f.manuscript,
                f.conversation,
                cancel,
                2,
                input_id,
                preview_id
            )
            .await
            .unwrap(),
        queue
    );
}

#[tokio::test]
async fn source_conflict_does_not_lock_head_or_create_submission() {
    let f = managed_fixture(true);
    let service = WritingSubmissionService::from_app_store(&f.store);
    let mut input = request(&f, Uuid::new_v4());
    let id = Uuid::new_v4();
    service
        .edit_input_queue(
            f.manuscript,
            f.conversation,
            Uuid::new_v4(),
            0,
            QueueEdit::Add(QueuedInput {
                user_input: vec![],
                origin: None,
                id,
                text: input.text.clone(),
            }),
        )
        .await
        .unwrap();
    input.context.files[0].expected_disk_sha256 = "0".repeat(64);
    assert!(
        service
            .prepare_queue_preview(&f.models, queue_preview_intent(&input, id, 1), input)
            .await
            .is_err()
    );
    let queue = service
        .read_input_queue(f.manuscript, f.conversation)
        .await
        .unwrap();
    assert_eq!(queue.revision(), 1);
    assert_eq!(queue.preparing(), None);
    assert_eq!(
        f.store
            .connection()
            .unwrap()
            .query_row("select count(*) from agent_submissions", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn queue_preview_and_cancel_recheck_binding() {
    let f = managed_fixture(true);
    let service = WritingSubmissionService::from_app_store(&f.store);
    let id = Uuid::new_v4();
    let preview = Uuid::new_v4();
    let input = request(&f, preview);
    service
        .edit_input_queue(
            f.manuscript,
            f.conversation,
            Uuid::new_v4(),
            0,
            QueueEdit::Add(QueuedInput {
                user_input: vec![],
                origin: None,
                id,
                text: input.text.clone(),
            }),
        )
        .await
        .unwrap();
    service
        .prepare_queue_preview(&f.models, queue_preview_intent(&input, id, 1), input)
        .await
        .unwrap();
    f.store
        .connection()
        .unwrap()
        .execute(
            "delete from agent_conversation_bindings where conversation_id=?1",
            [f.conversation.to_string()],
        )
        .unwrap();
    let replay = request(&f, preview);
    assert!(
        service
            .prepare_queue_preview(&f.models, queue_preview_intent(&replay, id, 1), replay)
            .await
            .is_err()
    );
    assert!(matches!(
        service
            .cancel_queue_preview(f.manuscript, f.conversation, Uuid::new_v4(), 2, id, preview)
            .await,
        Err(crate::data_interface::SqlImmediateFailure::Domain(
            research_agent_core::conversation::CatalogError::NotFound
        ))
    ));
}

#[tokio::test]
async fn unstarted_admission_discovery_rechecks_manuscript_binding() {
    let f = managed_fixture(true);
    let service = WritingSubmissionService::from_app_store(&f.store);
    assert!(
        service
            .list_unstarted_queue_admissions(f.manuscript, f.conversation, None, 10)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        service
            .list_unstarted_queue_admissions(Uuid::new_v4(), f.conversation, None, 10)
            .await
            .is_err()
    );
    let id = Uuid::new_v4();
    let request_id = Uuid::new_v4();
    service
        .edit_input_queue(
            f.manuscript,
            f.conversation,
            Uuid::new_v4(),
            0,
            QueueEdit::Add(QueuedInput {
                user_input: vec![],
                origin: None,
                id,
                text: "Explain".into(),
            }),
        )
        .await
        .unwrap();
    let input = request(&f, request_id);
    let preview = service
        .prepare_queue_preview(&f.models, queue_preview_intent(&input, id, 1), input)
        .await
        .unwrap();
    {
        let mut db = f.store.connection().unwrap();
        input_queue::admit_checked(
            &mut db,
            f.conversation,
            request_id,
            2,
            id,
            preview.record.input.clone(),
            |_| Ok(()),
        )
        .unwrap();
    }
    let candidates = service
        .list_unstarted_queue_admissions(f.manuscript, f.conversation, None, 10)
        .await
        .unwrap();
    let page = crate::agent::writing_queue_admissions::list(
        &service,
        crate::agent::writing_queue_admissions::Request {
            conversation_id: f.conversation,
            manuscript_id: f.manuscript,
            after: None,
            limit: 1,
        },
    )
    .await
    .unwrap();
    let json = serde_json::to_value(page).unwrap();
    assert_eq!(json["items"][0]["inputId"], id.to_string());
    assert_eq!(json["items"][0]["expectedRevision"], 2);
    assert_eq!(json["nextCursor"], id.to_string());
    assert_eq!(
        json["items"][0]["preview"]["requestId"],
        request_id.to_string()
    );
    assert!(!json.to_string().contains("synthetic-fixture"));
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].submission, preview.record);
    assert!(
        service
            .list_unstarted_queue_admissions(Uuid::new_v4(), f.conversation, None, 10)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn native_service_startup_pauses_once_but_opening_a_conversation_does_not() {
    let f = fixture(true);
    let writing = WritingSubmissionService::from_app_store(&f.store);
    writing
        .edit_input_queue(
            f.manuscript,
            f.conversation,
            Uuid::new_v4(),
            0,
            QueueEdit::Add(QueuedInput {
                user_input: vec![],
                origin: None,
                id: Uuid::new_v4(),
                text: "Later".into(),
            }),
        )
        .await
        .unwrap();
    let sessions = crate::agent::service::AgentSessionService::from_app_store(&f.store).unwrap();
    let paused = writing
        .read_input_queue(f.manuscript, f.conversation)
        .await
        .unwrap();
    assert!(paused.paused());
    writing
        .edit_input_queue(
            f.manuscript,
            f.conversation,
            Uuid::new_v4(),
            paused.revision(),
            QueueEdit::Resume,
        )
        .await
        .unwrap();
    sessions
        .open(research_agent_core::conversation::ConversationId::from_uuid(f.conversation))
        .await
        .unwrap();
    assert!(
        !writing
            .read_input_queue(f.manuscript, f.conversation)
            .await
            .unwrap()
            .paused()
    );
}
