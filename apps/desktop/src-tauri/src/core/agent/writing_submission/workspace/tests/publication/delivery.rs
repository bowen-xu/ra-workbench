use super::*;
use crate::agent::{
    repository::change_sets, writing_delivery_commands as delivery,
    writing_workspace_commands::Scope,
};
use research_agent_core::{
    conversation::ConversationId,
    protocol::{AgentEvent, EventPayload, ItemKind, ModelIdentity, TurnOutcome},
};

#[test]
fn delivery_proof_commit_failure_fences_the_managed_owner() {
    let f = fixture_with_objects();
    f.store.connection().unwrap().execute_batch("pragma foreign_keys=on;
        create table delivery_proof_parent(id integer primary key);
        create table delivery_proof_child(parent_id integer references delivery_proof_parent(id) deferrable initially deferred);").unwrap();
    let result = delivery::proof_transaction(&f.store, |tx| {
        tx.execute("insert into delivery_proof_child(parent_id) values(1)", [])
            .map_err(|_| crate::agent::writing_workspace_commands::Error::Unavailable)?;
        Ok(())
    });
    assert!(matches!(
        result,
        Err(crate::agent::writing_workspace_commands::Error::RecoveryRequired)
    ));
    assert!(f.store.connection().is_err());
}

#[test]
fn delivery_proof_rollback_failure_fences_the_managed_owner() {
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization, TransactionOperation};
    let f = fixture_with_objects();
    f.store
        .connection()
        .unwrap()
        .authorizer(Some(|context: AuthContext<'_>| match context.action {
            AuthAction::Transaction {
                operation: TransactionOperation::Rollback,
            } => Authorization::Deny,
            _ => Authorization::Allow,
        }));
    let result: Result<(), _> = delivery::proof_transaction(&f.store, |_| {
        Err(crate::agent::writing_workspace_commands::Error::Conflict)
    });
    assert!(matches!(
        result,
        Err(crate::agent::writing_workspace_commands::Error::RecoveryRequired)
    ));
    assert!(f.store.connection().is_err());
}

#[tokio::test]
async fn delivery_result_commit_requires_exact_complete_success_in_original_history() {
    for mode in [
        "incomplete",
        "wrong",
        "foreign",
        "no_pending",
        "valid",
        "rejected",
        "empty",
    ] {
        let f = fixture_with_objects();
        let service = WritingSubmissionService::from_app_store(&f.store);
        let record = service
            .prepare(&f.models, request(&f, Uuid::new_v4()))
            .await
            .unwrap()
            .record;
        let delivery_id = Uuid::new_v4();
        let item_id = Uuid::new_v4();
        let call_id = Uuid::new_v4();
        let result_id = Uuid::new_v4();
        {
            let db = f.store.connection().unwrap();
            let kind = if mode == "rejected" {
                "rejected"
            } else if mode == "empty" {
                "empty"
            } else {
                "applied"
            };
            db.execute("insert into writing_agent_delivery_proofs(delivery_id,conversation_id,request_id,turn_id,context_snapshot_id,manuscript_id,lease_id,manifest_hash,proof_json) values(?1,?2,?3,?4,?5,?6,?7,?8,?9)", params![delivery_id.to_string(),record.conversation_id.to_string(),record.request_id.to_string(),record.turn_id.to_string(),record.context_snapshot_id.to_string(),f.manuscript.to_string(),Uuid::new_v4().to_string(),"a".repeat(64),serde_json::json!({"schemaVersion":1,"kind":kind}).to_string()]).unwrap();
            db.execute("insert into writing_agent_delivery_tool_calls(delivery_id,item_id,call_id) values(?1,?2,?3)", params![delivery_id.to_string(),item_id.to_string(),call_id.to_string()]).unwrap();
            let mut events = vec![
                EventPayload::TurnStarted { task_kind: research_agent_core::protocol::TaskKind::Regular, turn_id: record.turn_id, context_snapshot_id: record.context_snapshot_id, model: record.input.model.clone() },
                EventPayload::ItemAccepted { turn_id: record.turn_id, item_id, kind: ItemKind::ToolCall { call_id, name: "research_deliver_files".into(), provider_call_id: Some("delivery-call".into()), namespace: None }, text: "{}".into(), phase: None, source: None },
                EventPayload::ToolReceiptPending { turn_id: record.turn_id, call_id },
                EventPayload::ItemStarted { turn_id: record.turn_id, item_id: result_id, kind: ItemKind::ToolResult { call_id, is_error: Some(false) } },
                EventPayload::TextDelta { turn_id: record.turn_id, item_id: result_id, text: serde_json::json!({"status":match mode { "wrong" | "rejected" => "rejected", "empty" => "unchanged", _ => "complete" },"requestId":record.request_id}).to_string() },
            ];
            if mode == "no_pending" {
                events.remove(2);
            }
            if mode != "incomplete" {
                events.push(EventPayload::ItemCompleted {
                    turn_id: record.turn_id,
                    item_id: result_id,
                });
            }
            for (index, payload) in events.into_iter().enumerate() {
                let request_id =
                    if mode == "foreign" && matches!(payload, EventPayload::TextDelta { .. }) {
                        Uuid::new_v4()
                    } else {
                        record.request_id
                    };
                let event = AgentEvent::new(
                    ConversationId::from_uuid(record.conversation_id),
                    index as u64 + 1,
                    0,
                    request_id,
                    payload,
                )
                .unwrap();
                db.execute("insert into agent_items(conversation_id,ordinal,schema_version,payload_json) values(?1,?2,1,?3)", params![record.conversation_id.to_string(), index as u64, serde_json::to_string(&event).unwrap()]).unwrap();
            }
        }
        let result = delivery::proof_transaction(&f.store, |tx| {
            delivery::confirm_result_in(tx, &record, "delivery-call")
        });
        if matches!(mode, "valid" | "rejected" | "empty") {
            result.unwrap();
            delivery::proof_transaction(&f.store, |tx| {
                delivery::confirm_result_in(tx, &record, "delivery-call")
            })
            .unwrap();
            assert!(
                delivery::proof_transaction(&f.store, |tx| delivery::confirm_result_in(
                    tx,
                    &record,
                    "wrong-call"
                ))
                .is_err()
            );
        } else {
            assert!(result.is_err());
        }
        let stored: u64 = f
            .store
            .connection()
            .unwrap()
            .query_row(
                "select count(*) from writing_agent_delivery_results",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            stored,
            u64::from(matches!(mode, "valid" | "rejected" | "empty"))
        );
    }
}

fn completed(
    f: &crate::agent::writing_submission::tests::Fixture,
    binding: &crate::agent::repository::execution_workspaces::Binding,
) -> delivery::Acknowledge {
    let request = delivery::Acknowledge {
        scope: Scope {
            manuscript_id: binding.manuscript_id,
            conversation_id: binding.conversation_id,
            request_id: binding.request_id,
            context_snapshot_id: binding.context_snapshot_id,
        },
        turn_id: binding.turn_id,
    };
    for (index, payload) in [
        EventPayload::TurnStarted {
            task_kind: research_agent_core::protocol::TaskKind::Regular,
            turn_id: binding.turn_id,
            context_snapshot_id: binding.context_snapshot_id,
            model: ModelIdentity {
                provider_id: "fixture".into(),
                model_id: "fixture".into(),
                revision: None,
            },
        },
        EventPayload::TurnFinished {
            turn_id: binding.turn_id,
            outcome: TurnOutcome::Completed,
        },
    ]
    .into_iter()
    .enumerate()
    {
        let event = AgentEvent::new(
            ConversationId::from_uuid(binding.conversation_id),
            index as u64 + 1,
            0,
            binding.request_id,
            payload,
        )
        .unwrap();
        f.store.connection().unwrap().execute("insert into agent_items(conversation_id,ordinal,schema_version,payload_json) values(?1,?2,1,?3)", params![binding.conversation_id.to_string(),index as u64,serde_json::to_string(&event).unwrap()]).unwrap();
    }
    request
}
#[tokio::test]
async fn delivery_acknowledgement_requires_complete_native_application_receipts_and_never_replays_files()
 {
    let f = fixture_with_objects();
    let operation = apply_disk::existing(&f, false).await;
    let binding = change_sets::read_in(
        &f.store.connection().unwrap(),
        f.conversation,
        f.manuscript,
        operation.proposal,
    )
    .unwrap()
    .binding;
    let request = completed(&f, &binding);
    assert!(
        delivery::acknowledge(&f.store, request.clone())
            .await
            .is_err()
    );
    assert!(
        f.store
            .connection()
            .unwrap()
            .execute(
                "update agent_proposal_applications set status='applied'",
                [],
            )
            .is_err()
    );
    assert!(
        delivery::acknowledge(&f.store, request.clone())
            .await
            .is_err()
    );
    WritingSubmissionService::from_app_store(&f.store)
        .apply_disk_proposal_file(operation, "main.tex".into())
        .await
        .unwrap();
    let path = f
        .store
        .manuscript_directory()
        .join(f.manuscript.to_string())
        .join("source/main.tex");
    std::fs::write(&path, "later user text").unwrap();
    delivery::acknowledge(&f.store, request.clone())
        .await
        .unwrap();
    delivery::acknowledge(&f.store, request).await.unwrap();
    assert_eq!(std::fs::read_to_string(path).unwrap(), "later user text");
    assert!(
        delivery::list(
            &f.store,
            delivery::Request {
                manuscript_id: f.manuscript,
                after: None,
                limit: 64
            }
        )
        .await
        .unwrap()
        .items
        .is_empty()
    );
}

#[tokio::test]
async fn applied_file_does_not_make_terminal_delivery_ack_available_during_running_turn() {
    let f = fixture_with_objects();
    let operation = apply_disk::existing(&f, false).await;
    let service = WritingSubmissionService::from_app_store(&f.store);
    service
        .apply_disk_proposal_file(operation, "main.tex".into())
        .await
        .unwrap();
    let binding = change_sets::read_in(
        &f.store.connection().unwrap(),
        f.conversation,
        f.manuscript,
        operation.proposal,
    )
    .unwrap()
    .binding;
    let request = delivery::Acknowledge {
        scope: Scope {
            manuscript_id: binding.manuscript_id,
            conversation_id: binding.conversation_id,
            request_id: binding.request_id,
            context_snapshot_id: binding.context_snapshot_id,
        },
        turn_id: binding.turn_id,
    };
    let started = AgentEvent::new(
        ConversationId::from_uuid(binding.conversation_id),
        1,
        0,
        binding.request_id,
        EventPayload::TurnStarted {
            task_kind: research_agent_core::protocol::TaskKind::Regular,
            turn_id: binding.turn_id,
            context_snapshot_id: binding.context_snapshot_id,
            model: ModelIdentity {
                provider_id: "fixture".into(),
                model_id: "fixture".into(),
                revision: None,
            },
        },
    )
    .unwrap();
    f.store.connection().unwrap().execute("insert into agent_items(conversation_id,ordinal,schema_version,payload_json) values(?1,0,1,?2)", params![binding.conversation_id.to_string(), serde_json::to_string(&started).unwrap()]).unwrap();
    let delivery_id = Uuid::new_v4();
    let item_id = Uuid::new_v4();
    let call_id = Uuid::new_v4();
    let accepted = AgentEvent::new(
        ConversationId::from_uuid(binding.conversation_id),
        2,
        0,
        binding.request_id,
        EventPayload::ItemAccepted {
            turn_id: binding.turn_id,
            item_id,
            kind: ItemKind::ToolCall {
                call_id,
                name: "research_deliver_files".into(),
                provider_call_id: Some("fixture-delivery".into()),
                namespace: None,
            },
            text: "{}".into(),
            phase: None,
            source: None,
        },
    )
    .unwrap();
    f.store.connection().unwrap().execute("insert into agent_items(conversation_id,ordinal,schema_version,payload_json) values(?1,1,1,?2)", params![binding.conversation_id.to_string(), serde_json::to_string(&accepted).unwrap()]).unwrap();
    assert!(
        delivery::verify_running_with_call(
            &f.store,
            request.clone(),
            delivery_id,
            "wrong-call",
            binding.clone()
        )
        .await
        .is_err()
    );
    assert_eq!(
        f.store
            .connection()
            .unwrap()
            .query_row(
                "select count(*) from writing_agent_delivery_proofs",
                [],
                |row| row.get::<_, u64>(0)
            )
            .unwrap(),
        0
    );
    assert_eq!(
        delivery::verify_running_with_call(
            &f.store,
            request.clone(),
            delivery_id,
            "fixture-delivery",
            binding.clone()
        )
        .await
        .unwrap(),
        delivery::RunningProof::Applied
    );
    assert_eq!(
        delivery::verify_running_with_call(
            &f.store,
            request.clone(),
            delivery_id,
            "fixture-delivery",
            binding.clone()
        )
        .await
        .unwrap(),
        delivery::RunningProof::Applied
    );
    assert!(
        delivery::verify_running_with_call(
            &f.store,
            request.clone(),
            Uuid::new_v4(),
            "fixture-delivery",
            binding.clone()
        )
        .await
        .is_err()
    );
    let review = service
        .review_workspace(request.scope.receipt(), request.scope.context_snapshot_id)
        .await
        .unwrap();
    let root = f.store.agent_workspace_directory().canonicalize().unwrap();
    let copies = crate::agent::workspace::WorkspaceStore::open(&root, |_| Ok(())).unwrap();
    let copy = copies.restore(&review.baseline.identity).unwrap();
    let copy_path = copy.working_directory().unwrap().join("main.tex");
    std::fs::write(&copy_path, b"later unproven change").unwrap();
    drop(copy);
    assert!(
        delivery::verify_running(&f.store, request.clone(), delivery_id)
            .await
            .is_err()
    );
    let copy = copies.restore(&review.baseline.identity).unwrap();
    std::fs::write(
        copy.working_directory().unwrap().join("main.tex"),
        b"replacement",
    )
    .unwrap();
    drop(copy);
    assert!(matches!(
        delivery::acknowledge(&f.store, request.clone()).await,
        Err(crate::agent::writing_workspace_commands::Error::Denied)
    ));
    let finished = AgentEvent::new(
        ConversationId::from_uuid(binding.conversation_id),
        3,
        0,
        binding.request_id,
        EventPayload::TurnFinished {
            turn_id: binding.turn_id,
            outcome: TurnOutcome::Completed,
        },
    )
    .unwrap();
    f.store.connection().unwrap().execute("insert into agent_items(conversation_id,ordinal,schema_version,payload_json) values(?1,2,1,?2)", params![binding.conversation_id.to_string(), serde_json::to_string(&finished).unwrap()]).unwrap();
    delivery::acknowledge(&f.store, request).await.unwrap();
}
#[tokio::test]
async fn delivery_acknowledgement_rechecks_the_native_copy_and_cannot_discard_unpublished_changes()
{
    let f = fixture_with_objects();
    let review = changed(&f).await;
    let request = completed(&f, &review.binding);
    let delivery_id = Uuid::new_v4();
    assert!(
        delivery::acknowledge(&f.store, request.clone())
            .await
            .is_err()
    );
    let service = WritingSubmissionService::from_app_store(&f.store);
    let proposal = service
        .publish_workspace_changes(review.binding, review.changes)
        .await
        .unwrap();
    crate::agent::writing_proposal_commands::reject(
        &f.store,
        crate::agent::writing_proposal_commands::ProposalRequest {
            conversation_id: f.conversation,
            manuscript_id: f.manuscript,
            proposal_id: proposal,
        },
    )
    .await
    .unwrap();
    assert_eq!(
        delivery::verify_running(&f.store, request.clone(), delivery_id)
            .await
            .unwrap(),
        delivery::RunningProof::Rejected
    );
    delivery::acknowledge(&f.store, request).await.unwrap();
    assert_eq!(
        std::fs::read_to_string(
            f.store
                .manuscript_directory()
                .join(f.manuscript.to_string())
                .join("source/main.tex")
        )
        .unwrap(),
        "Frozen manuscript"
    );
}
