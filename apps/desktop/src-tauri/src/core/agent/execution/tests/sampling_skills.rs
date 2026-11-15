use super::*;
use research_agent_core::{
    protocol::{AgentEvent, EventPayload, TaskKind},
    skills::{SkillDescriptor, SkillIdentity, SubmissionSkill, instructions::SkillInstructions},
};

fn history(
    record: &SubmissionRecord,
    name: &str,
    text: String,
    error: bool,
) -> ConversationProjection {
    let id = ConversationId::from_uuid(record.conversation_id);
    let mut history = ConversationProjection::new(id);
    let mut append = |payload| {
        let event = AgentEvent::new(id, history.seq() + 1, 1, Uuid::new_v4(), payload).unwrap();
        history.apply(&event).unwrap();
    };
    append(EventPayload::TurnStarted {
        task_kind: TaskKind::Regular,
        turn_id: record.turn_id,
        context_snapshot_id: record.context_snapshot_id,
        model: model(),
    });
    let call_id = Uuid::new_v4();
    for (kind, text) in [
        (
            ItemKind::ToolCall {
                call_id,
                name: name.into(),
                provider_call_id: None,
                namespace: None,
            },
            serde_json::json!({"identity":record.input.skills[0].descriptor.identity}).to_string(),
        ),
        (
            ItemKind::ToolResult {
                call_id,
                is_error: Some(error),
            },
            text,
        ),
    ] {
        let item_id = Uuid::new_v4();
        append(EventPayload::ItemStarted {
            turn_id: record.turn_id,
            item_id,
            kind,
        });
        append(EventPayload::TextDelta {
            turn_id: record.turn_id,
            item_id,
            text,
        });
        append(EventPayload::ItemCompleted {
            turn_id: record.turn_id,
            item_id,
        });
    }
    history
}

#[tokio::test]
async fn successful_registered_load_restores_exact_body_and_read_only_package_once() {
    let (_, store, _, _, _, id) = setup().await;
    let identity = SkillIdentity {
        environment_id: "application".into(),
        source_id: "skills/review/SKILL.md".into(),
        revision: "frozen".into(),
    };
    let mut input = input("v1");
    input.skills.push(SubmissionSkill {
        descriptor: SkillDescriptor {
            identity: identity.clone(),
            name: "review".into(),
            description: "Review evidence".into(),
            allow_implicit_invocation: true,
        },
        explicit: false,
    });
    let mut record = store.put(id, Uuid::new_v4(), input).await.unwrap();
    let body = "Preserve original evidence, including Unicode 科研.";
    let payload = serde_json::json!({"skill": {
        "identity":identity, "name":"review", "body":body,
        "executionDirectory":crate::agent::skills::packages::relative(&identity),
    }})
    .to_string();
    let loaded = history(&record, "research_load_skill", payload.clone(), false);
    let expected = SkillInstructions {
        identity: identity.clone(),
        name: "review".into(),
        contents: crate::agent::skills::injection::frozen_contents(body, &identity),
    };
    let restore = super::super::sampling_skills::restore(store.as_ref(), &record, &loaded)
        .await
        .unwrap();
    assert!(matches!(&restore[..], [ContentPart::Text(text)] if text == &expected.render()));
    // Explicit injection already owns this exact frozen prompt; a later load does
    // not conflict with its package policy or add a duplicate body.
    record.input.skill_instructions.push(expected);
    assert!(
        super::super::sampling_skills::restore(store.as_ref(), &record, &loaded)
            .await
            .unwrap()
            .is_empty()
    );
    record.input.skill_instructions.clear();
    for (name, error) in [
        ("research_read_skill_resource", false),
        ("research_load_skill", true),
    ] {
        let ignored = history(&record, name, payload.clone(), error);
        assert!(
            super::super::sampling_skills::restore(store.as_ref(), &record, &ignored)
                .await
                .unwrap()
                .is_empty()
        );
    }
    let mut forged: serde_json::Value = serde_json::from_str(&payload).unwrap();
    forged["skill"]["executionDirectory"] = serde_json::json!("/unregistered/package");
    let invalid = history(&record, "research_load_skill", forged.to_string(), false);
    assert!(
        super::super::sampling_skills::restore(store.as_ref(), &record, &invalid)
            .await
            .is_err()
    );
}
