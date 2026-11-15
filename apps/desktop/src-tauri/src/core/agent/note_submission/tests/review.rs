use super::*;
use crate::agent::{
    note_review::{self, Scope},
    note_tools,
    repository::append_batch,
};
use research_agent_core::{
    model::ToolCall,
    protocol::{AgentEvent, EventPayload as P, ItemKind},
};
use serde_json::{Value, json};

pub(super) async fn seeded(f: &Fixture, complete: bool) -> (Scope, String) {
    let record = prepare(&f.store, &f.models, request(f, Uuid::new_v4()))
        .await
        .unwrap();
    let call = ToolCall {
        namespace: None,
        raw_arguments_json: None,
        call_id: "rewrite-1".into(),
        name: note_tools::REWRITE.into(),
        arguments: json!({
            "sourceId":record.input.context[0].source_id,"edits":[{"blockId":"selected","content":{"kind":"inline","items":[{"type":"text","text":"Reviewed suggestion","styles":[]}]}}]
        }),
    };
    let result = note_tools::evaluate(&record, &call).unwrap();
    let call_id = Uuid::new_v4();
    let call_item = Uuid::new_v4();
    let result_item = Uuid::new_v4();
    let turn_id = record.turn_id;
    let mut payloads = vec![
        P::TurnStarted {
            task_kind: research_agent_core::protocol::TaskKind::Regular,
            turn_id,
            context_snapshot_id: record.context_snapshot_id,
            model: record.input.model,
        },
        P::ItemStarted {
            turn_id,
            item_id: call_item,
            kind: ItemKind::ToolCall {
                call_id,
                name: call.name,
                provider_call_id: Some(call.call_id),
                namespace: None,
            },
        },
        P::TextDelta {
            turn_id,
            item_id: call_item,
            text: call.arguments.to_string(),
        },
        P::ItemCompleted {
            turn_id,
            item_id: call_item,
        },
        P::ItemStarted {
            turn_id,
            item_id: result_item,
            kind: ItemKind::ToolResult {
                call_id,
                is_error: Some(false),
            },
        },
        P::TextDelta {
            turn_id,
            item_id: result_item,
            text: result.clone(),
        },
    ];
    if complete {
        payloads.push(P::ItemCompleted {
            turn_id,
            item_id: result_item,
        });
    }
    let events: Vec<_> = payloads
        .into_iter()
        .enumerate()
        .map(|(i, p)| {
            AgentEvent::new(
                ConversationId::from_uuid(f.conversation),
                i as u64 + 1,
                0,
                record.request_id,
                p,
            )
            .unwrap()
        })
        .collect();
    append_batch(
        &mut f.store.connection().unwrap(),
        ConversationId::from_uuid(f.conversation),
        Uuid::new_v4(),
        0,
        &events,
    )
    .unwrap();
    (
        Scope {
            conversation_id: f.conversation,
            page_id: f.page,
            turn_id,
            item_id: result_item,
        },
        result,
    )
}

#[tokio::test]
async fn native_review_recovers_exact_proposal_and_reports_revision_conflict_without_writes() {
    let f = fixture(true);
    let (scope, original) = seeded(&f, true).await;
    let reopened = AppStore::for_test(
        Connection::open(f.root.path().join("fixture.sqlite")).unwrap(),
        f.root.path().join("data/objects"),
    );
    let before = reopened.connection().unwrap().total_changes();
    let review = note_review::review(&reopened, scope).await.unwrap();
    assert!(review.baseline_matches);
    assert_eq!(review.current_revision, 0);
    assert_eq!(
        review.proposal,
        serde_json::from_str::<Value>(&original).unwrap()
    );
    assert_eq!(reopened.connection().unwrap().total_changes(), before);
    reopened
        .connection()
        .unwrap()
        .execute("update note_pages set revision=1", [])
        .unwrap();
    let conflict = note_review::review(&reopened, scope).await.unwrap();
    assert!(!conflict.baseline_matches);
    assert_eq!(conflict.current_revision, 1);
    assert_eq!(conflict.proposal, review.proposal);
    reopened
        .connection()
        .unwrap()
        .execute("delete from note_blocks where id='selected'", [])
        .unwrap();
    let removed = note_review::review(&reopened, scope).await.unwrap();
    assert!(!removed.baseline_matches);
    assert_eq!(removed.proposal, review.proposal);
}

#[tokio::test]
async fn review_checks_content_hash_even_if_the_live_revision_was_not_incremented() {
    let f = fixture(true);
    let (scope, _) = seeded(&f, true).await;
    let changed =
        json!({"kind":"inline","items":[{"type":"text","text":"New live evidence","styles":[]}]})
            .to_string();
    f.store
        .connection()
        .unwrap()
        .execute(
            "update note_blocks set content_json=?1 where id='selected'",
            [changed],
        )
        .unwrap();
    let review = note_review::review(&f.store, scope).await.unwrap();
    assert_eq!(review.current_revision, 0);
    assert!(!review.baseline_matches);
}

#[tokio::test]
async fn refuses_incomplete_wrong_scope_and_revoked_review_without_new_runtime_events() {
    let f = fixture(true);
    let (scope, _) = seeded(&f, false).await;
    let changes = f.store.connection().unwrap().total_changes();
    assert!(matches!(
        note_review::review(&f.store, scope).await,
        Err(note_review::Error::Incomplete)
    ));
    assert!(matches!(
        note_review::review(
            &f.store,
            Scope {
                page_id: Uuid::new_v4(),
                ..scope
            }
        )
        .await,
        Err(note_review::Error::Unbound)
    ));
    assert!(matches!(
        note_review::review(
            &f.store,
            Scope {
                item_id: Uuid::nil(),
                ..scope
            }
        )
        .await,
        Err(note_review::Error::InvalidInput)
    ));
    assert!(
        note_review::review(
            &f.store,
            Scope {
                turn_id: Uuid::new_v4(),
                ..scope
            }
        )
        .await
        .is_err()
    );
    assert_eq!(f.store.connection().unwrap().total_changes(), changes);
    f.store
        .connection()
        .unwrap()
        .execute("update agent_conversations set archived=1", [])
        .unwrap();
    assert!(matches!(
        note_review::review(&f.store, scope).await,
        Err(note_review::Error::Unbound)
    ));
}

#[tokio::test]
async fn rejects_forged_result_role_arguments_identity_and_broken_journal() {
    // Each mutation starts from a valid journal and saved source. A look-alike
    // assistant JSON or altered result must never become native edit authority.
    for mutation in [
        "result",
        "arguments",
        "provider",
        "ordinal",
        "schema",
        "source",
    ] {
        let f = fixture(true);
        let (scope, _) = seeded(&f, true).await;
        {
            let db = f.store.connection().unwrap();
            match mutation {
                "result" => {
                    db.execute("update agent_items set payload_json=json_set(payload_json,'$.payload.text','{}') where ordinal=5",[]).unwrap();
                }
                "arguments" => {
                    db.execute("update agent_items set payload_json=json_set(payload_json,'$.payload.text','{}') where ordinal=2",[]).unwrap();
                }
                "provider" => {
                    db.execute("update agent_items set payload_json=json_set(payload_json,'$.payload.kind.providerCallId','different') where ordinal=1",[]).unwrap();
                }
                "ordinal" => {
                    db.execute("delete from agent_items where ordinal=2", [])
                        .unwrap();
                }
                "schema" => {
                    db.execute("update agent_items set payload_json=json_set(payload_json,'$.schemaVersion',2) where ordinal=5",[]).unwrap();
                }
                "source" => {
                    db.execute("update agent_submissions set input_json=json_set(input_json,'$.context[0].text','{}')",[]).unwrap();
                }
                _ => unreachable!(),
            }
        }
        assert!(
            matches!(
                note_review::review(&f.store, scope).await,
                Err(note_review::Error::Corrupt)
            ),
            "{mutation}"
        );
    }
    let f = fixture(true);
    let (scope, _) = seeded(&f, true).await;
    f.store.connection().unwrap().execute("update agent_items set payload_json=json_set(payload_json,'$.payload.kind',json('{\"type\":\"assistantMessage\"}')) where ordinal=4",[]).unwrap();
    assert!(matches!(
        note_review::review(&f.store, scope).await,
        Err(note_review::Error::InvalidInput)
    ));
}

#[test]
fn review_scope_rejects_renderer_content_and_permission_overrides() {
    let scope = json!({"conversationId":Uuid::new_v4(),"pageId":Uuid::new_v4(),"turnId":Uuid::new_v4(),"itemId":Uuid::new_v4()});
    assert!(serde_json::from_value::<Scope>(scope.clone()).is_ok());
    for field in [
        "proposal",
        "before",
        "after",
        "baselineMatches",
        "allowWrite",
    ] {
        let mut invalid = scope.clone();
        invalid[field] = json!(true);
        assert!(serde_json::from_value::<Scope>(invalid).is_err());
    }
}
