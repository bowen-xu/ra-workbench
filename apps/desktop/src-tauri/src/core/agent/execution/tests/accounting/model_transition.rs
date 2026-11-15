use super::*;

#[tokio::test]
async fn native_text_checkpoint_continues_under_a_new_provider_without_rebinding() {
    text_checkpoint_model_transition(true).await;
}

#[tokio::test]
async fn native_text_checkpoint_rejects_an_unrecognized_codec_before_sampling() {
    text_checkpoint_model_transition(false).await;
}

async fn text_checkpoint_model_transition(compatible: bool) {
    use research_agent_core::context::{
        compaction_strategy::{CompactionBinding, CompactionKind},
        text_compaction::TextCompactionStrategy,
    };
    let (service, store, transport, tools, execution, id) = setup().await;
    *transport.responses.lock().unwrap() = VecDeque::from(vec![response(false), response(false)]);
    let old = store
        .put(id, Uuid::new_v4(), input("covered-old-source"))
        .await
        .unwrap();
    let execution = Arc::new(execution);
    assert_eq!(
        run(
            execution.clone(),
            service.clone(),
            store.clone(),
            id,
            old.request_id
        )
        .await,
        TaskCompletion::Completed
    );
    let mut command = input("compact");
    command.text = "/compact".into();
    let record = store.put(id, Uuid::new_v4(), command).await.unwrap();
    let admission = execution
        .clone()
        .start_compaction(
            service.clone(),
            store.clone(),
            ConversationId::from_uuid(id),
            record.request_id,
            None,
        )
        .await
        .unwrap();
    let SubmissionAdmission::Started(owned) = admission else {
        panic!("expected compact task")
    };
    assert_eq!(owned.task.wait().await, TaskCompletion::Completed);
    let previous = service
        .active_context_checkpoint(id)
        .await
        .unwrap()
        .unwrap();
    let mut execution = Arc::try_unwrap(execution).ok().unwrap();
    let next_model = ModelIdentity {
        provider_id: "next-provider".into(),
        model_id: "next-model".into(),
        revision: Some("v2".into()),
    };
    execution.model = next_model.clone();
    execution.system_instructions = "Current model authoritative rules".into();
    execution.compaction_strategy = Some(Arc::new(
        TextCompactionStrategy::new(
            transport.clone(),
            CompactionBinding {
                model: next_model.clone(),
                strategy_id: if compatible {
                    previous.binding.strategy_id.clone()
                } else {
                    "unrecognized-codec".into()
                },
                state_schema: previous.binding.state_schema,
                kind: CompactionKind::TextSummary,
            },
        )
        .unwrap(),
    ));
    execution.transport = Arc::new(Accounted {
        inner: transport.clone(),
        policy: Arc::new(ContextAccounting {
            model: next_model.clone(),
            budget: Some(CompactionBudget {
                context_window_tokens: 10000,
                effective_percent: 95,
                configured_trigger_tokens: None,
                output_reserve_tokens: 100,
            }),
            unknown_window_trigger_tokens: None,
            estimator: Arc::new(Utf8JsonTokenEstimator),
        }),
    });
    let mut events = response(false);
    if let ModelStreamEvent::Started { actual_model, .. } = &mut events[0] {
        *actual_model = next_model.clone();
    }
    transport.responses.lock().unwrap().push_back(events);
    let mut next = input("new-source");
    next.model = next_model.clone();
    let next = store.put(id, Uuid::new_v4(), next).await.unwrap();
    assert_eq!(
        run(
            Arc::new(execution),
            service.clone(),
            store,
            id,
            next.request_id
        )
        .await,
        if compatible {
            TaskCompletion::Completed
        } else {
            TaskCompletion::Failed
        }
    );
    let restored = service
        .active_context_checkpoint(id)
        .await
        .unwrap()
        .unwrap();
    assert!(restored == previous);
    assert_eq!(restored.binding.model, model());
    let requests = transport.requests.lock().unwrap();
    assert_eq!(requests.len(), if compatible { 3 } else { 2 });
    if !compatible {
        assert_eq!(tools.0.load(Ordering::SeqCst), 0);
        return;
    }
    assert_eq!(requests[2].model, next_model);
    assert!(
        requests[2].messages[0] == ModelMessage::System("Current model authoritative rules".into())
    );
    let encoded = requests[2]
        .messages
        .iter()
        .flat_map(|message| match message {
            ModelMessage::CanonicalOutput(item) => match &item.normalized {
                Some(research_agent_core::model::ModelOutputItem::Message(message)) => {
                    vec![message.text.as_str()]
                }
                _ => vec![],
            },
            ModelMessage::System(text)
            | ModelMessage::Developer(text)
            | ModelMessage::ToolResult { content: text, .. } => {
                vec![text.as_str()]
            }
            ModelMessage::User { content: parts, .. }
            | ModelMessage::Assistant { content: parts, .. } => parts
                .iter()
                .filter_map(|part| match part {
                    ContentPart::Text(text) => Some(text.as_str()),
                    _ => None,
                })
                .collect(),
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(encoded.contains("Context checkpoint handoff"));
    assert!(!encoded.contains("covered-old-source"));
    assert!(encoded.contains("new-source"));
    assert_eq!(tools.0.load(Ordering::SeqCst), 0);
}
