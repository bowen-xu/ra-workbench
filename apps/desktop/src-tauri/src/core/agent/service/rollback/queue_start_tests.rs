use super::tests::{finish, setup, start};
use super::*;
use crate::agent::{
    repository::{SqliteSubmissionStore, input_queue},
    tasks::TaskCompletion,
};
use research_agent_core::{
    input_queue::{QueueEdit, QueuedInput},
    submission::SubmissionInput,
};
use std::sync::atomic::{AtomicUsize, Ordering};

#[tokio::test]
async fn rollback_paused_admission_cannot_execute_and_explicit_resume_starts_once() {
    let (db, service, id) = setup();
    crate::storage::migrate(&mut db.lock().unwrap()).unwrap();
    service.create(id).await.unwrap();
    let turn = start(&service, id).await;
    finish(&service, id, turn).await;
    let scope = *id.as_uuid();
    let request_id = Uuid::new_v4();
    let input_id = Uuid::new_v4();
    let record = {
        let mut db = db.lock().unwrap();
        input_queue::apply_checked(
            &mut db,
            scope,
            Uuid::new_v4(),
            0,
            QueueEdit::Add(QueuedInput {
                id: input_id,
                text: "pending confirmation".into(),
                user_input: vec![],
                origin: None,
            }),
            |_| Ok(()),
        )
        .unwrap();
        input_queue::apply_checked(
            &mut db,
            scope,
            Uuid::new_v4(),
            1,
            QueueEdit::Prepare(input_id),
            |_| Ok(()),
        )
        .unwrap();
        input_queue::admit_checked(
            &mut db,
            scope,
            request_id,
            2,
            input_id,
            SubmissionInput {
                remote_compaction_v2: None,
                text: "pending confirmation".into(),
                model: super::tests::model(),
                user_input: vec![],
                skill_instructions: vec![],
                instructions: vec![],
                instruction_configs: vec![],
                skills: vec![],
                reasoning_effort: None,
                service_tier: None,
                context: vec![],
            },
            |_| Ok(()),
        )
        .unwrap()
        .0
    };
    service
        .rollback_conversation(ConversationRollbackRequest {
            conversation_id: id,
            request_id: Uuid::new_v4(),
            expected_watermark: 5,
            expected_last_turn: turn,
            user_turns: 1,
        })
        .await
        .unwrap();
    let submissions = SqliteSubmissionStore::new(db.clone());
    let calls = Arc::new(AtomicUsize::new(0));
    let rejected_calls = calls.clone();
    assert!(
        service
            .start_submission(&submissions, id, request_id, move |_, _, _| async move {
                rejected_calls.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
            .await
            .is_err()
    );
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while service.tasks.has_active(id).unwrap() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(service.open(id).await.unwrap().seq(), 6);
    {
        let mut db = db.lock().unwrap();
        let queue = input_queue::read_checked(&mut db, scope, |_| Ok(())).unwrap();
        input_queue::apply_checked(
            &mut db,
            scope,
            Uuid::new_v4(),
            queue.revision(),
            QueueEdit::Resume,
            |_| Ok(()),
        )
        .unwrap();
    }
    let worker = service.clone();
    let run_calls = calls.clone();
    let admission = service
        .start_submission(
            &submissions,
            id,
            request_id,
            move |record, history, _| async move {
                assert_eq!(history.seq(), 6);
                assert!(history.turn_order().is_empty());
                run_calls.fetch_add(1, Ordering::SeqCst);
                worker
                    .commit_payload(
                        id,
                        record.request_id,
                        EventPayload::TurnFinished {
                            turn_id: record.turn_id,
                            outcome: research_agent_core::protocol::TurnOutcome::Completed,
                        },
                    )
                    .await
                    .unwrap();
                Ok(())
            },
        )
        .await
        .unwrap();
    let SubmissionAdmission::Started(owned) = admission else {
        panic!()
    };
    assert_eq!(owned.task.wait().await, TaskCompletion::Completed);
    assert!(
        matches!(service.start_submission(&submissions, id, request_id, |_, _, _| async { panic!("exact retry executed") }).await.unwrap(),
        SubmissionAdmission::AlreadyStarted { turn_id, .. } if turn_id == record.turn_id)
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}
