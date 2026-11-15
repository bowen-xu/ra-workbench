use super::*;

#[tokio::test]
async fn instruction_snapshot_uses_user_role_and_survives_tool_continuation() {
    use research_agent_core::instructions::*;
    let (service, store, transport, tools, execution, id) = setup().await;
    let mut draft = input("v1");
    draft.instructions = assemble_instructions(
        vec![InstructionDocument {
            environment_id: "paper".into(),
            scope_id: ".".into(),
            source_id: "AGENTS.md".into(),
            revision: "v1".into(),
            text: "Style rule".into(),
        }],
        1024,
    )
    .unwrap();
    let saved = store.put(id, Uuid::new_v4(), draft).await.unwrap();
    assert_eq!(
        run(Arc::new(execution), service, store, id, saved.request_id).await,
        TaskCompletion::Completed
    );
    let requests = transport.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    for request in requests.iter() {
        assert!(
            matches!(&request.messages[0], ModelMessage::System(text) if text == "Research assistant")
        );
        let ModelMessage::User { content: parts, .. } = &request.messages[1] else {
            panic!("instruction role");
        };
        assert!(
            matches!(&parts[0], ContentPart::Text(text) if text.contains("Style rule") && text.contains("Frozen project instructions") && text.contains(&saved.turn_id.to_string()))
        );
        assert!(
            matches!(&parts[1], ContentPart::Text(text) if text.contains("evidence v1") && text.contains("not instructions"))
        );
        assert_eq!(request.tools.len(), 4);
    }
    assert_eq!(tools.0.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn historical_instruction_version_stays_bound_to_its_original_turn() {
    use research_agent_core::instructions::*;
    let (service, store, transport, _, execution, id) = setup().await;
    let execution = Arc::new(execution);
    let mut draft = input("v1");
    draft.instructions = assemble_instructions(
        vec![InstructionDocument {
            environment_id: "paper".into(),
            scope_id: ".".into(),
            source_id: "AGENTS.md".into(),
            revision: "instruction-v1".into(),
            text: "Old convention".into(),
        }],
        1024,
    )
    .unwrap();
    let first = store.put(id, Uuid::new_v4(), draft.clone()).await.unwrap();
    assert_eq!(
        run(
            execution.clone(),
            service.clone(),
            store.clone(),
            id,
            first.request_id
        )
        .await,
        TaskCompletion::Completed
    );
    draft.instructions[0].revision = "instruction-v2".into();
    draft.instructions[0].text = "New convention".into();
    let second = store.put(id, Uuid::new_v4(), draft).await.unwrap();
    assert_eq!(
        run(execution, service, store, id, second.request_id).await,
        TaskCompletion::Completed
    );
    let requests = transport.requests.lock().unwrap();
    let parts: Vec<_> = requests
        .last()
        .unwrap()
        .messages
        .iter()
        .filter_map(|message| {
            if let ModelMessage::User { content: parts, .. } = message {
                Some(parts)
            } else {
                None
            }
        })
        .flatten()
        .filter_map(|part| {
            if let ContentPart::Text(text) = part {
                Some(text)
            } else {
                None
            }
        })
        .collect();
    assert!(parts.iter().any(|text| text.contains("instruction-v1")
        && text.contains(&first.turn_id.to_string())
        && text.contains("Old convention")));
    assert!(parts.iter().any(|text| text.contains("instruction-v2")
        && text.contains(&second.turn_id.to_string())
        && text.contains("New convention")));
}
