use super::*;
use research_agent_core::protocol::{
    HookPromptFragment, ModelIdentity, TaskKind, TurnOutcome, TurnPhase,
};

#[test]
fn hook_prompt_admission_requires_committed_matching_audit_and_cannot_repeat() {
    let (projector, epoch) = fixture();
    let id = ConversationId::from_uuid(Uuid::new_v4());
    let turn = Uuid::new_v4();
    let invocation = Uuid::new_v4();
    let handler = Uuid::new_v4();
    let run = Uuid::new_v4();
    let model = ModelIdentity {
        provider_id: "fixture".into(),
        model_id: "model".into(),
        revision: None,
    };
    {
        let mut db = projector.raw.lock().unwrap();
        crate::agent::repository::create_conversation(&mut db, id, "fixture").unwrap();
        let started = event(
            id,
            1,
            EventPayload::TurnStarted {
                turn_id: turn,
                context_snapshot_id: Uuid::new_v4(),
                model: model.clone(),
                task_kind: TaskKind::Regular,
            },
        );
        crate::agent::repository::append_batch(&mut db, id, Uuid::new_v4(), 0, &[started]).unwrap();
        db.execute("insert into agent_stop_hook_admissions(invocation_id,conversation_id,turn_id,event_name,model_json,boundary_seq,admitted_at_ms) values(?1,?2,?3,'stop',?4,1,1)", params![invocation.to_string(),id.as_uuid().to_string(),turn.to_string(),serde_json::to_string(&model).unwrap()]).unwrap();
        db.execute("insert into agent_stop_hook_handlers(invocation_id,ordinal,handler_id,run_id) values(?1,0,?2,?3)", params![invocation.to_string(),handler.to_string(),run.to_string()]).unwrap();
    }
    let prompt = |seq, text: String| {
        event(
            id,
            seq,
            EventPayload::ItemAccepted {
                turn_id: turn,
                item_id: Uuid::new_v4(),
                kind: ItemKind::HookPrompt {
                    invocation_id: invocation,
                    fragments: vec![HookPromptFragment {
                        run_id: run,
                        input_text: text,
                    }],
                },
                text: String::new(),
                phase: None,
                source: None,
            },
        )
    };
    let xml = format!("<hook_prompt hook_run_id=\"{run}\">retry</hook_prompt>");
    let proposal = command(id, 1, vec![prompt(2, xml.clone())], None);
    assert_eq!(
        projector.validate(&proposal, &[]).err(),
        Some(JournalError::InvalidInput)
    );
    {
        let db = projector.raw.lock().unwrap();
        db.execute("insert into agent_stop_hook_results(invocation_id,ordinal,run_id,outcome_json) values(?1,0,?2,?3)", params![invocation.to_string(),run.to_string(),r#"{"state":"completed","status":{"kind":"blocked","feedback":"retry"},"system_message":null}"#]).unwrap();
        db.execute("insert into agent_stop_hook_finishes(invocation_id,canceled,finished_at_ms) values(?1,0,2)", [invocation.to_string()]).unwrap();
    }
    let wrong = command(id, 1, vec![prompt(2, "fabricated".into())], None);
    assert_eq!(
        projector.validate(&wrong, &[]).err(),
        Some(JournalError::InvalidInput)
    );
    let forged = accepted(epoch, 1, wrong);
    assert_eq!(
        projector.apply_batch(&[forged]).err(),
        Some(JournalError::InvalidInput)
    );
    assert_eq!(projector.state().unwrap().materialized_position, 0);
    assert!(matches!(
        projector.validate(&proposal, &[]),
        Ok(Admission::New(2))
    ));
    let terminal = command(
        id,
        1,
        vec![event(
            id,
            2,
            EventPayload::TurnFinished {
                turn_id: turn,
                outcome: TurnOutcome::Completed,
            },
        )],
        None,
    );
    let after_terminal = command(id, 2, vec![prompt(3, xml.clone())], None);
    assert_eq!(
        projector
            .validate(&after_terminal, &[accepted(epoch, 1, terminal)])
            .err(),
        Some(JournalError::InvalidInput)
    );
    let first = accepted(epoch, 1, proposal);
    let duplicate = command(id, 2, vec![prompt(3, xml.clone())], None);
    assert_eq!(
        projector
            .validate(&duplicate, std::slice::from_ref(&first))
            .err(),
        Some(JournalError::InvalidInput)
    );
    let draft = Uuid::new_v4();
    let streamed = accepted(
        epoch,
        1,
        command(
            id,
            1,
            vec![
                event(
                    id,
                    2,
                    EventPayload::ItemStarted {
                        turn_id: turn,
                        item_id: draft,
                        kind: ItemKind::AssistantMessage,
                    },
                ),
                event(
                    id,
                    3,
                    EventPayload::TextDelta {
                        turn_id: turn,
                        item_id: draft,
                        text: "streamed prefix".into(),
                    },
                ),
            ],
            None,
        ),
    );
    let followup = command(id, 3, vec![prompt(4, xml.clone())], None);
    assert!(matches!(
        projector.validate(&followup, std::slice::from_ref(&streamed)),
        Ok(Admission::New(4))
    ));
    let projected = [streamed, accepted(epoch, 2, followup)];
    projector.apply_batch(&projected).unwrap();
    projector.apply_batch(&projected).unwrap();
    let duplicate = command(id, 4, vec![prompt(5, xml)], None);
    assert_eq!(
        projector.validate(&duplicate, &[]).err(),
        Some(JournalError::InvalidInput)
    );

    // A different committed Stop result becomes stale as soon as a later
    // model request begins, whether that request is pending or in SQLite.
    let next_invocation = Uuid::new_v4();
    let next_run = Uuid::new_v4();
    {
        let db = projector.raw.lock().unwrap();
        db.execute("insert into agent_stop_hook_admissions(invocation_id,conversation_id,turn_id,event_name,model_json,boundary_seq,admitted_at_ms) values(?1,?2,?3,'stop',?4,4,3)", params![next_invocation.to_string(),id.as_uuid().to_string(),turn.to_string(),serde_json::to_string(&model).unwrap()]).unwrap();
        db.execute("insert into agent_stop_hook_handlers(invocation_id,ordinal,handler_id,run_id) values(?1,0,?2,?3)", params![next_invocation.to_string(),Uuid::new_v4().to_string(),next_run.to_string()]).unwrap();
        db.execute("insert into agent_stop_hook_results(invocation_id,ordinal,run_id,outcome_json) values(?1,0,?2,?3)", params![next_invocation.to_string(),next_run.to_string(),r#"{"state":"completed","status":{"kind":"blocked","feedback":"retry"},"system_message":null}"#]).unwrap();
        db.execute("insert into agent_stop_hook_finishes(invocation_id,canceled,finished_at_ms) values(?1,0,4)", [next_invocation.to_string()]).unwrap();
    }
    let phase = event(
        id,
        5,
        EventPayload::PhaseChanged {
            turn_id: turn,
            phase: TurnPhase::RunningModel,
        },
    );
    let attempt = Uuid::new_v4();
    let sample = event(
        id,
        6,
        EventPayload::ModelRequestStarted {
            turn_id: turn,
            attempt_id: attempt,
            model: model.clone(),
        },
    );
    let stale = event(
        id,
        7,
        EventPayload::ItemAccepted {
            turn_id: turn,
            item_id: Uuid::new_v4(),
            kind: ItemKind::HookPrompt {
                invocation_id: next_invocation,
                fragments: vec![HookPromptFragment {
                    run_id: next_run,
                    input_text: format!(
                        "<hook_prompt hook_run_id=\"{next_run}\">retry</hook_prompt>"
                    ),
                }],
            },
            text: String::new(),
            phase: None,
            source: None,
        },
    );
    let same_batch = command(
        id,
        4,
        vec![phase.clone(), sample.clone(), stale.clone()],
        None,
    );
    assert_eq!(
        projector.validate(&same_batch, &[]).err(),
        Some(JournalError::InvalidInput)
    );
    let later_sample = accepted(epoch, 3, command(id, 4, vec![phase, sample], None));
    let stale_command = command(id, 6, vec![stale], None);
    assert_eq!(
        projector
            .validate(&stale_command, std::slice::from_ref(&later_sample))
            .err(),
        Some(JournalError::InvalidInput)
    );
    projector.apply_batch(&[later_sample]).unwrap();
    assert_eq!(
        projector.validate(&stale_command, &[]).err(),
        Some(JournalError::InvalidInput)
    );
    assert_eq!(
        projector
            .apply_batch(&[accepted(epoch, 4, stale_command)])
            .err(),
        Some(JournalError::InvalidInput)
    );
    assert_eq!(projector.state().unwrap().materialized_position, 3);

    // A sample at or before the next audit's boundary is the response that
    // the hook evaluated; it must not invalidate that new feedback.
    let response = event(
        id,
        7,
        EventPayload::ModelResponseStarted {
            turn_id: turn,
            attempt_id: attempt,
            response_id: "response".into(),
            model: None,
        },
    );
    let finished = event(
        id,
        8,
        EventPayload::ModelRequestFinished {
            turn_id: turn,
            attempt_id: attempt,
            input_tokens: None,
            output_tokens: None,
        },
    );
    projector
        .apply_batch(&[accepted(
            epoch,
            4,
            command(id, 6, vec![response, finished], None),
        )])
        .unwrap();
    let current_invocation = Uuid::new_v4();
    let current_run = Uuid::new_v4();
    {
        let db = projector.raw.lock().unwrap();
        db.execute("insert into agent_stop_hook_admissions(invocation_id,conversation_id,turn_id,event_name,model_json,boundary_seq,admitted_at_ms) values(?1,?2,?3,'stop',?4,8,5)", params![current_invocation.to_string(),id.as_uuid().to_string(),turn.to_string(),serde_json::to_string(&model).unwrap()]).unwrap();
        db.execute("insert into agent_stop_hook_handlers(invocation_id,ordinal,handler_id,run_id) values(?1,0,?2,?3)", params![current_invocation.to_string(),Uuid::new_v4().to_string(),current_run.to_string()]).unwrap();
        db.execute("insert into agent_stop_hook_results(invocation_id,ordinal,run_id,outcome_json) values(?1,0,?2,?3)", params![current_invocation.to_string(),current_run.to_string(),r#"{"state":"completed","status":{"kind":"blocked","feedback":"retry"},"system_message":null}"#]).unwrap();
        db.execute("insert into agent_stop_hook_finishes(invocation_id,canceled,finished_at_ms) values(?1,0,6)", [current_invocation.to_string()]).unwrap();
    }
    let current = event(
        id,
        9,
        EventPayload::ItemAccepted {
            turn_id: turn,
            item_id: Uuid::new_v4(),
            kind: ItemKind::HookPrompt {
                invocation_id: current_invocation,
                fragments: vec![HookPromptFragment {
                    run_id: current_run,
                    input_text: format!(
                        "<hook_prompt hook_run_id=\"{current_run}\">retry</hook_prompt>"
                    ),
                }],
            },
            text: String::new(),
            phase: None,
            source: None,
        },
    );
    let current = command(id, 8, vec![current], None);
    assert!(matches!(
        projector.validate(&current, &[]),
        Ok(Admission::New(9))
    ));
    projector
        .apply_batch(&[accepted(epoch, 5, current)])
        .unwrap();
}
