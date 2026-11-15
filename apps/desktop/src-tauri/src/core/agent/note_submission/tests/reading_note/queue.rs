use super::super::start::{no_items, pending_transport, sessions};
use super::*;
use crate::agent::{
    execution_profile::AgentExecutionMode, note_queue as queue, note_queue_preview as preview,
    note_queue_recovery as recovery, note_queue_start as start, note_start,
    service::SubmissionAdmission, tasks::TaskCompletion,
};
use research_agent_core::model::ModelTransport;
use std::sync::{Arc, atomic::Ordering};

fn scope(f: &Fixture, reading: bool) -> queue::Scope {
    if reading {
        queue::Scope::ReadingNote {
            resource_id: f.page,
        }
    } else {
        queue::Scope::Note {
            resource_id: f.page,
        }
    }
}
fn receipt(f: &Fixture, id: Uuid, reading: bool) -> SourceReceipt {
    SourceReceipt {
        conversation_id: f.conversation,
        request_id: id,
        resource: scope(f, reading).resource(),
    }
}
async fn enqueue(f: &Fixture, reading: bool, revision: u64) -> Uuid {
    let input = Uuid::new_v4();
    queue::edit(&f.store,serde_json::from_value(serde_json::json!({"conversationId":f.conversation,"scope":scope(f,reading),"requestId":Uuid::new_v4(),"expectedRevision":revision,"edit":{"type":"add","inputId":input,"text":"Explain retained evidence"}})).unwrap()).await.unwrap();
    input
}
fn reopen_managed(f: Fixture) -> Fixture {
    let Fixture {
        root,
        store,
        models,
        conversation,
        page,
    } = f;
    drop(models);
    drop(store);
    let db = crate::data_interface::Database::open(
        Connection::open(root.path().join("fixture.sqlite")).unwrap(),
        &root.path().join("log"),
    )
    .unwrap();
    let store = AppStore::for_test_database(db, root.path().join("data/objects"));
    let models = AgentModelResolver::from_app_store(
        &store,
        JsonCredentialStore::from_path(root.path().join("credentials.json")),
    );
    Fixture {
        root,
        store,
        models,
        conversation,
        page,
    }
}
fn preparation(
    f: &Fixture,
    reading: bool,
    input: Uuid,
    request_id: Uuid,
    revision: u64,
) -> preview::PrepareRequest {
    let model = Some(ModelSelection {
        provider_id: "deepseek".into(),
        model_id: "deepseek-v4-pro".into(),
        reasoning_effort: None,
        service_tier: None,
    });
    let submission = if reading {
        let mut request = reading_request(f, request_id);
        request.text = "Explain retained evidence".into();
        request.model = model;
        preview::Submission::ReadingNote(request)
    } else {
        let mut request = request(f, request_id);
        request.text = "Explain retained evidence".into();
        request.model = model;
        preview::Submission::Note(request)
    };
    preview::PrepareRequest {
        input_id: input,
        expected_revision: revision,
        submission,
    }
}
fn confirmation(
    f: &Fixture,
    record: &SubmissionRecord,
    reading: bool,
) -> note_start::SelectedStartRequest {
    note_start::SelectedStartRequest {
        receipt: receipt(f, record.request_id, reading),
        context_snapshot_id: record.context_snapshot_id,
        model: record.input.model.clone(),
        mode: AgentExecutionMode::DeepSeekNonThinking,
        call_limits: None,
    }
}
async fn pending(f: &Fixture, reading: bool) -> Result<Option<preview::Preview>, preview::Error> {
    preview::read_pending(
        &f.store,
        queue::ReadRequest {
            conversation_id: f.conversation,
            scope: scope(f, reading),
        },
    )
    .await
}
async fn state(f: &Fixture) -> serde_json::Value {
    serde_json::to_value(
        queue::read(
            &f.store,
            queue::ReadRequest {
                conversation_id: f.conversation,
                scope: scope(f, false),
            },
        )
        .await
        .unwrap(),
    )
    .unwrap()
}
fn cancellation(
    f: &Fixture,
    reading: bool,
    input_id: Uuid,
    request: Uuid,
    revision: u64,
    cancel_id: Uuid,
) -> preview::CancelRequest {
    preview::CancelRequest {
        conversation_id: f.conversation,
        scope: scope(f, reading),
        input_id,
        preview_request_id: request,
        expected_revision: revision,
        request_id: cancel_id,
    }
}
async fn candidates(f: &Fixture, reading: bool) -> recovery::Page {
    recovery::list(
        &f.store,
        recovery::Request {
            conversation_id: f.conversation,
            scope: scope(f, reading),
            after: None,
            limit: 64,
        },
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn frozen_queue_previews_retain_head_and_recover_original_note_content_at_current_queue_revision()
 {
    for reading in [false, true] {
        let f = managed_reading_fixture();
        let input = enqueue(&f, reading, 0).await;
        let id = Uuid::new_v4();
        let first = preview::prepare(&f.store, &f.models, preparation(&f, reading, input, id, 1))
            .await
            .unwrap();
        let again = preview::prepare(&f.store, &f.models, preparation(&f, reading, input, id, 1))
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(&first).unwrap(),
            serde_json::to_value(again).unwrap()
        );
        assert_eq!(state(&f).await["inputs"].as_array().unwrap().len(), 1);
        assert!(matches!(
            pending(&f, !reading).await,
            Err(preview::Error::Preparation(Error::Unbound))
        ));
        enqueue(&f, reading, 2).await;
        f.store
            .connection()
            .unwrap()
            .execute("update note_pages set title='Changed',revision=1", [])
            .unwrap();
        f.store
            .connection()
            .unwrap()
            .execute(
                "update note_pages set revision=1 where id in (select page_id from reading_note_sources)",
                [],
            )
            .unwrap();
        let restored = pending(&f, reading).await.unwrap().unwrap();
        assert_eq!(restored.record.input, first.record.input);
        assert_eq!(serde_json::to_value(restored.queue).unwrap()["revision"], 3);
        assert_eq!(count(&f), 1);
        no_items(&f);
    }
}

#[tokio::test]
async fn cancelled_preview_cannot_start_through_normal_or_queued_paths_and_is_not_normal_recovery()
{
    for reading in [false, true] {
        let f = managed_reading_fixture();
        let input = enqueue(&f, reading, 0).await;
        let id = Uuid::new_v4();
        preview::prepare(&f.store, &f.models, preparation(&f, reading, input, id, 1))
            .await
            .unwrap();
        let record = restore_selected(&f.store, receipt(&f, id, reading))
            .await
            .unwrap();
        let cancel = Uuid::new_v4();
        assert!(
            preview::cancel(&f.store, cancellation(&f, !reading, input, id, 2, cancel))
                .await
                .is_err()
        );
        let one = preview::cancel(&f.store, cancellation(&f, reading, input, id, 2, cancel))
            .await
            .unwrap();
        let two = preview::cancel(&f.store, cancellation(&f, reading, input, id, 2, cancel))
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(one).unwrap(),
            serde_json::to_value(two).unwrap()
        );
        assert!(pending(&f, reading).await.unwrap().is_none());
        assert_eq!(state(&f).await["inputs"].as_array().unwrap().len(), 1);
        let service = sessions(&f);
        assert!(matches!(
            note_start::start_selected_checked(
                &f.store,
                service.clone(),
                confirmation(&f, &record, reading),
                None,
                None,
                |_, _| async { panic!("queue origin must block normal start before auth") }
            )
            .await,
            Err(note_start::Error::ConfirmationMismatch)
        ));
        let transport = pending_transport();
        assert!(matches!(
            note_start::start_selected_checked(
                &f.store,
                service,
                confirmation(&f, &record, reading),
                Some(start::Confirmation {
                    input_id: input,
                    expected_revision: 2
                }),
                None,
                |_, _| async { Ok(transport.clone() as Arc<dyn ModelTransport>) }
            )
            .await,
            Err(note_start::Error::Queue(_))
        ));
        assert_eq!(transport.calls.load(Ordering::SeqCst), 0);
        assert!(
            super::super::super::recovery::list_selected(
                &f.store,
                super::super::super::recovery::SelectedRequest {
                    conversation_id: f.conversation,
                    resource: scope(&f, reading).resource(),
                    after: None,
                    limit: 64
                }
            )
            .await
            .unwrap()
            .items
            .is_empty()
        );
        assert!(candidates(&f, reading).await.items.is_empty());
        no_items(&f);
    }
}

#[tokio::test]
async fn admitted_but_unstarted_queue_recovers_once_and_duplicate_start_never_refreshes_transport()
{
    for (reading, interrupted_admission) in
        [(false, false), (true, false), (false, true), (true, true)]
    {
        let mut f = managed_reading_fixture();
        let input = enqueue(&f, reading, 0).await;
        let id = Uuid::new_v4();
        preview::prepare(&f.store, &f.models, preparation(&f, reading, input, id, 1))
            .await
            .unwrap();
        let record = restore_selected(&f.store, receipt(&f, id, reading))
            .await
            .unwrap();
        let admission = start::Confirmation {
            input_id: input,
            expected_revision: 2,
        };
        if interrupted_admission {
            crate::agent::note_queue_admission::admit(
                &f.store,
                receipt(&f, id, reading),
                &record,
                admission,
            )
            .await
            .unwrap();
            assert_eq!(state(&f).await["inputs"], serde_json::json!([]));
            f = reopen_managed(f);
            let recovered = recovery::list(
                &f.store,
                recovery::Request {
                    conversation_id: f.conversation,
                    scope: scope(&f, reading),
                    after: None,
                    limit: 64,
                },
            )
            .await
            .unwrap();
            assert_eq!(recovered.items.len(), 1);
            assert_eq!(
                recovered.items[0].queue.as_ref().unwrap().expected_revision,
                2
            );
        } else {
            assert_eq!(state(&f).await["inputs"].as_array().unwrap().len(), 1);
            assert!(candidates(&f, reading).await.items.is_empty());
        }
        assert!(candidates(&f, !reading).await.items.is_empty());
        let service = sessions(&f);
        let transport = pending_transport();
        let first = note_start::start_selected_checked(
            &f.store,
            service.clone(),
            confirmation(&f, &record, reading),
            Some(admission),
            None,
            |_, _| async { Ok(transport.clone() as Arc<dyn ModelTransport>) },
        )
        .await
        .unwrap();
        let SubmissionAdmission::Started(owned) = first else {
            panic!("expected new owner")
        };
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            transport.reached.notified(),
        )
        .await
        .unwrap();
        let again = note_start::start_selected_checked(
            &f.store,
            service.clone(),
            confirmation(&f, &record, reading),
            Some(admission),
            None,
            |_, _| async { panic!("must return durable admission without auth") },
        )
        .await
        .unwrap();
        assert!(matches!(again, SubmissionAdmission::AlreadyStarted { .. }));
        assert!(candidates(&f, reading).await.items.is_empty());
        assert!(
            service
                .tasks()
                .cancel_turn(ConversationId::from_uuid(f.conversation), owned.turn_id)
                .unwrap()
        );
        assert_eq!(owned.task.wait().await, TaskCompletion::Canceled);
        assert_eq!(transport.calls.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn failed_auth_or_binding_revocation_retains_head_without_admission() {
    let f = managed_reading_fixture();
    let input = enqueue(&f, true, 0).await;
    let id = Uuid::new_v4();
    preview::prepare(&f.store, &f.models, preparation(&f, true, input, id, 1))
        .await
        .unwrap();
    let record = restore_selected(&f.store, receipt(&f, id, true))
        .await
        .unwrap();
    let admission = start::Confirmation {
        input_id: input,
        expected_revision: 2,
    };
    let service = sessions(&f);
    assert!(matches!(
        note_start::start_selected_checked(
            &f.store,
            service.clone(),
            confirmation(&f, &record, true),
            Some(admission),
            None,
            |_, _| async { Err(note_start::Error::ModelUnavailable) }
        )
        .await,
        Err(note_start::Error::ModelUnavailable)
    ));
    let transport = pending_transport();
    assert!(
        note_start::start_selected_checked(
            &f.store,
            service,
            confirmation(&f, &record, true),
            Some(admission),
            None,
            |_, _| async {
                f.store
                    .connection()
                    .unwrap()
                    .execute(
                        "delete from agent_conversation_bindings where resource_kind='readingNote'",
                        [],
                    )
                    .unwrap();
                Ok(transport.clone() as Arc<dyn ModelTransport>)
            }
        )
        .await
        .is_err()
    );
    assert_eq!(state(&f).await["revision"], 2);
    assert_eq!(transport.calls.load(Ordering::SeqCst), 0);
    no_items(&f);
}

#[tokio::test]
async fn failed_preview_transaction_rolls_back_submission_and_queue_claim() {
    let f = managed_reading_fixture();
    let input = enqueue(&f, true, 0).await;
    let id = Uuid::new_v4();
    f.store.connection().unwrap().execute_batch("create trigger reject_queue_preview before insert on agent_input_queue_previews begin select raise(abort,'synthetic failure'); end;").unwrap();
    assert!(
        preview::prepare(&f.store, &f.models, preparation(&f, true, input, id, 1))
            .await
            .is_err()
    );
    assert_eq!(count(&f), 0);
    assert_eq!(state(&f).await["revision"], 1);
    f.store
        .connection()
        .unwrap()
        .execute_batch("drop trigger reject_queue_preview;")
        .unwrap();
    preview::prepare(&f.store, &f.models, preparation(&f, true, input, id, 1))
        .await
        .unwrap();
}

#[test]
fn queue_confirmation_and_cancellation_do_not_accept_model_or_resource_overrides() {
    let mut request = serde_json::json!({"conversationId":Uuid::new_v4(),"scope":{"kind":"readingNote","resourceId":Uuid::new_v4()},"requestId":Uuid::new_v4(),"expectedRevision":2,"inputId":Uuid::new_v4(),"previewRequestId":Uuid::new_v4()});
    assert!(serde_json::from_value::<preview::CancelRequest>(request.clone()).is_ok());
    request["execute"] = serde_json::json!(true);
    assert!(serde_json::from_value::<preview::CancelRequest>(request).is_err());
    assert!(
        serde_json::from_value::<start::Confirmation>(
            serde_json::json!({"inputId":Uuid::new_v4(),"expectedRevision":2,"text":"override"})
        )
        .is_err()
    );
}

#[tokio::test]
async fn cancelling_and_admitting_the_same_note_preview_have_one_durable_winner() {
    let f = managed_reading_fixture();
    let input = enqueue(&f, true, 0).await;
    let id = Uuid::new_v4();
    preview::prepare(&f.store, &f.models, preparation(&f, true, input, id, 1))
        .await
        .unwrap();
    let record = restore_selected(&f.store, receipt(&f, id, true))
        .await
        .unwrap();
    let (admitted, cancelled) = tokio::join!(
        crate::agent::note_queue_admission::admit(
            &f.store,
            receipt(&f, id, true),
            &record,
            start::Confirmation {
                input_id: input,
                expected_revision: 2
            }
        ),
        preview::cancel(
            &f.store,
            cancellation(&f, true, input, id, 2, Uuid::new_v4())
        )
    );
    assert_ne!(admitted.is_ok(), cancelled.is_ok());
    assert_eq!(
        state(&f).await["inputs"].as_array().unwrap().len(),
        usize::from(cancelled.is_ok())
    );
    assert_eq!(
        candidates(&f, true).await.items.len(),
        usize::from(admitted.is_ok())
    );
    assert!(pending(&f, true).await.unwrap().is_none());
    no_items(&f);
}
