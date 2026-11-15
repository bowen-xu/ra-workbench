use super::*;
use crate::agent::repository::{append_batch, create_conversation};
use research_agent_core::protocol::{
    AgentEvent, EventPayload, ModelIdentity, TaskKind, TurnOutcome, TurnPhase,
};

fn started(turn_id: Uuid) -> EventPayload {
    EventPayload::TurnStarted {
        task_kind: TaskKind::Regular,
        turn_id,
        context_snapshot_id: Uuid::new_v4(),
        model: ModelIdentity {
            provider_id: "fixture".into(),
            model_id: "fixture".into(),
            revision: None,
        },
    }
}
fn append(db: &mut Connection, id: ConversationId, payload: EventPayload) {
    let count: u64 = db
        .query_row(
            "select durable_count from agent_conversations where id=?1",
            [id.as_uuid().to_string()],
            |row| row.get(0),
        )
        .unwrap();
    let event = AgentEvent::new(id, count + 1, 0, Uuid::new_v4(), payload).unwrap();
    append_batch(db, id, Uuid::new_v4(), count, &[event]).unwrap();
}

#[tokio::test]
async fn activity_uses_the_last_committed_boundary_and_respects_binding_without_writes() {
    let (connection, catalog) = super::super::tests::setup();
    let id = super::super::tests::id;
    {
        let mut db = connection.lock().unwrap();
        for n in 1..=4 {
            create_conversation(&mut db, id(n), "Synthetic chat").unwrap();
            append(&mut db, id(n), started(Uuid::from_u128(n + 10)));
            append(
                &mut db,
                id(n),
                EventPayload::PhaseChanged {
                    turn_id: Uuid::from_u128(n + 10),
                    phase: TurnPhase::RunningModel,
                },
            );
        }
        db.execute("insert into agent_conversation_bindings(conversation_id,resource_kind,resource_id) values(?1,'paper','paper-a')", [id(1).as_uuid().to_string()]).unwrap();
        db.execute(
            "insert into ai_runs(id,task_kind,status,provider_id,model_id,pipeline_version,prompt_version,output_schema_version,user_goal,scope_hash,input_item_count,max_input_tokens,max_output_tokens,timeout_ms,created_at,updated_at)
             values(?1,'paper_overview','generating','fixture-generation','fixture-v1','test','test',1,'',?2,0,100,100,1000,'now','now')",
            params![Uuid::from_u128(10).to_string(), "a".repeat(64)],
        )
        .unwrap();
        db.execute("insert into agent_legacy_ai_run_bindings(ai_run_id,conversation_id,request_id,task_kind) values(?1,?2,?3,'fixture')",
            params![Uuid::from_u128(10).to_string(), id(2).as_uuid().to_string(), Uuid::from_u128(11).to_string()]).unwrap();
        db.execute(
            "update agent_conversations set archived=1 where id=?1",
            [id(4).as_uuid().to_string()],
        )
        .unwrap();
    }
    let changes = connection.lock().unwrap().total_changes();
    let running = catalog.activity(None).await.unwrap();
    assert_eq!(
        running.iter().map(|row| row.id).collect::<Vec<_>>(),
        (1..=3).map(|n| *id(n).as_uuid()).collect::<Vec<_>>()
    );
    assert!(running[1].internal);
    assert!(!running[0].internal);
    let bound = catalog
        .activity(Some(ConversationBinding {
            kind: BindingKind::Paper,
            resource_id: "paper-a".into(),
        }))
        .await
        .unwrap();
    assert_eq!(bound.len(), 1);
    assert_eq!(bound[0].id, *id(1).as_uuid());
    assert_eq!(connection.lock().unwrap().total_changes(), changes);
    for (index, outcome) in [
        TurnOutcome::Completed,
        TurnOutcome::Failed,
        TurnOutcome::Canceled,
        TurnOutcome::Interrupted,
    ]
    .into_iter()
    .enumerate()
    {
        {
            let mut db = connection.lock().unwrap();
            let turn_id = if index == 0 {
                Uuid::from_u128(11)
            } else {
                Uuid::new_v4()
            };
            if index > 0 {
                append(&mut db, id(1), started(turn_id));
            }
            append(
                &mut db,
                id(1),
                EventPayload::TurnFinished { turn_id, outcome },
            );
        }
        assert!(
            !catalog
                .activity(None)
                .await
                .unwrap()
                .iter()
                .any(|row| row.id == *id(1).as_uuid())
        );
    }
    assert!(
        catalog
            .activity(Some(ConversationBinding {
                kind: BindingKind::Paper,
                resource_id: "".into()
            }))
            .await
            .is_err()
    );
}
