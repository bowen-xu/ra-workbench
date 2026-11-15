use super::super::super::tests::{managed_fixture, queue_preview_intent};
use super::*;
use research_agent_core::input_queue::{QueueEdit, QueuedInput};

#[tokio::test]
async fn queue_confirmation_runs_once_and_retries_original_admission() {
    let f = managed_fixture(true);
    let service = WritingSubmissionService::from_app_store(&f.store);
    let id = Uuid::new_v4();
    let mut input = request(&f, Uuid::new_v4());
    input.model = Some(AgentModelSelection {
        provider_id: "deepseek".into(),
        model_id: "deepseek-v4-pro".into(),
        reasoning_effort: None,
        service_tier: None,
    });
    input.user_input = vec![research_agent_core::user_input::UserInput::Text {
        text: input.text.clone(),
        text_elements: vec![],
    }];
    service
        .edit_input_queue(
            f.manuscript,
            f.conversation,
            Uuid::new_v4(),
            0,
            QueueEdit::Add(QueuedInput {
                user_input: input.user_input.clone(),
                origin: None,
                id,
                text: input.text.clone(),
            }),
        )
        .await
        .unwrap();
    let receipt = WritingSubmissionReceipt {
        conversation_id: f.conversation,
        manuscript_id: f.manuscript,
        request_id: input.request_id,
    };
    service
        .prepare_queue_preview(&f.models, queue_preview_intent(&input, id, 1), input)
        .await
        .unwrap();
    let prepared = service.restore(&f.models, receipt).await.unwrap();
    let record = prepared.record.clone();
    assert!(!record.input.user_input.is_empty());
    // The complete managed preparation and current tool catalog exceed the
    // generic fixture's deliberately small 10,000-byte request guard. Compose the
    // same reviewed profile as writing_start instead of tuning a test threshold.
    let execution_profile = crate::agent::execution_profile::execution_profile(
        &record.input.model,
        crate::agent::execution_profile::AgentExecutionMode::DeepSeekNonThinking,
    )
    .unwrap();
    let sessions = Arc::new(AgentSessionService::new(Arc::new(
        SqliteConversationStore::new(f.store.shared_agent_connection(), "fixture".into()).unwrap(),
    )));
    let transport = Arc::new(Transport(AtomicUsize::new(0)));
    for attempt in 0..2 {
        service
            .validate_start_for_queue(
                &prepared,
                &confirmation(&record, f.manuscript),
                &execution_profile,
                Some((id, 2)),
            )
            .await
            .unwrap();
        assert!(
            service
                .read_input_queue(f.manuscript, f.conversation)
                .await
                .unwrap()
                .inputs()
                .is_empty()
        );
        let admitted = service
            .start_transport(
                sessions.clone(),
                record.clone(),
                execution_profile.clone(),
                transport.clone(),
            )
            .await
            .unwrap();
        match (attempt, admitted) {
            (0, SubmissionAdmission::Started(owned)) => {
                let completion = owned.task.wait().await;
                if completion != TaskCompletion::Completed {
                    let db = f.store.connection().unwrap();
                    let mut statement = db.prepare(
                        "select json_extract(payload_json,'$.payload.type'), substr(json_extract(payload_json,'$.payload.error'),1,256), json_extract(payload_json,'$.payload.category') from agent_items where conversation_id=?1 order by ordinal desc limit 12",
                    ).unwrap();
                    let diagnostics: Vec<(Option<String>, Option<String>, Option<String>)> =
                        statement
                            .query_map([record.conversation_id.to_string()], |row| {
                                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
                            })
                            .unwrap()
                            .map(Result::unwrap)
                            .collect();
                    panic!(
                        "queued fixture did not complete: {completion:?}; calls={}; bounded event kinds/safe runner errors={diagnostics:?}",
                        transport.0.load(Ordering::SeqCst)
                    );
                }
            }
            (
                1,
                SubmissionAdmission::AlreadyStarted {
                    outcome: Some(TurnOutcome::Completed),
                    ..
                },
            ) => {}
            _ => panic!("unexpected admission"),
        }
    }
    assert_eq!(transport.0.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn queue_confirmation_rejects_stale_preview_and_canceled_input_before_consumption() {
    let f = managed_fixture(true);
    let service = WritingSubmissionService::from_app_store(&f.store);
    let id = Uuid::new_v4();
    let input = request(&f, Uuid::new_v4());
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
    let receipt = WritingSubmissionReceipt {
        conversation_id: f.conversation,
        manuscript_id: f.manuscript,
        request_id: input.request_id,
    };
    service
        .prepare_queue_preview(&f.models, queue_preview_intent(&input, id, 1), input)
        .await
        .unwrap();
    let prepared = service.restore(&f.models, receipt).await.unwrap();
    let record = &prepared.record;
    let mut stale = confirmation(record, f.manuscript);
    stale.context_snapshot_id = Uuid::new_v4();
    assert!(
        service
            .validate_start_for_queue(
                &prepared,
                &stale,
                &profile(record.input.model.clone()),
                Some((id, 2))
            )
            .await
            .is_err()
    );
    assert!(
        service
            .validate_start_for_queue(
                &prepared,
                &confirmation(record, f.manuscript),
                &profile(record.input.model.clone()),
                Some((Uuid::new_v4(), 2))
            )
            .await
            .is_err()
    );
    assert_eq!(
        service
            .read_input_queue(f.manuscript, f.conversation)
            .await
            .unwrap()
            .inputs()
            .len(),
        1
    );
    service
        .cancel_queue_preview(
            f.manuscript,
            f.conversation,
            Uuid::new_v4(),
            2,
            id,
            record.request_id,
        )
        .await
        .unwrap();
    assert!(
        service
            .validate_start_for_queue(
                &prepared,
                &confirmation(record, f.manuscript),
                &profile(record.input.model.clone()),
                Some((id, 2))
            )
            .await
            .is_err()
    );
    assert_eq!(
        service
            .read_input_queue(f.manuscript, f.conversation)
            .await
            .unwrap()
            .inputs()
            .len(),
        1
    );
}
