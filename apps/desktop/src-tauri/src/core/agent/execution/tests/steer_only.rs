use super::*;
use research_agent_core::steering::{SteeringInput, SteeringScope, SteeringStore};

struct HeldAnswer {
    inner: Arc<Transport>,
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
    first: std::sync::atomic::AtomicBool,
}
impl ModelTransport for HeldAnswer {
    fn capabilities(&self) -> ModelCapabilities {
        self.inner.capabilities()
    }
    fn start(&self, request: ModelRequest) -> ModelFuture<'_, Box<dyn ModelStream>> {
        Box::pin(async move {
            if self.first.swap(false, Ordering::SeqCst) {
                self.entered.notify_one();
                self.release.notified().await;
            }
            self.inner.start(request).await
        })
    }
}

#[tokio::test]
async fn native_steer_only_continuation_compacts_before_consumption_without_empty_followup() {
    let root = tempfile::tempdir().unwrap();
    let mut db = Connection::open_in_memory().unwrap();
    crate::storage::migrate(&mut db).unwrap();
    let app = crate::storage::AppStore::for_test(db, root.path().join("objects"));
    let service = Arc::new(AgentSessionService::from_app_store(&app).unwrap());
    let id = Uuid::new_v4();
    service.create(ConversationId::from_uuid(id)).await.unwrap();
    let submissions = Arc::new(SqliteSubmissionStore::new(app.shared_agent_connection()));
    let steering = Arc::new(
        crate::agent::repository::steering::SqliteSteeringStore::new(app.shared_agent_connection()),
    );
    let (_, _, transport, tools, mut execution, _) = setup().await;
    let mut large_answer = response(false);
    large_answer[1] = ModelStreamEvent::TextDelta("x".repeat(8000));
    *transport.responses.lock().unwrap() =
        VecDeque::from([large_answer, response(false), response(false)]);
    let answer = Arc::new(HeldAnswer {
        inner: transport.clone(),
        entered: tokio::sync::Notify::new(),
        release: tokio::sync::Notify::new(),
        first: std::sync::atomic::AtomicBool::new(true),
    });
    let compact = Arc::new(HeldStrategy {
        inner: text_strategy(transport.clone()),
        entered: tokio::sync::Notify::new(),
        release: tokio::sync::Notify::new(),
    });
    execution.transport = token_compaction(answer.clone(), 1000);
    execution.compaction_strategy = Some(compact.clone());
    execution.max_input_bytes = usize::MAX;
    let submission = submissions
        .put(id, Uuid::new_v4(), input("frozen"))
        .await
        .unwrap();
    let admission = Arc::new(execution)
        .start_with_steering(
            service.clone(),
            submissions,
            ConversationId::from_uuid(id),
            submission.request_id,
            Some(steering.clone()),
        )
        .await
        .unwrap();
    let SubmissionAdmission::Started(owned) = admission else {
        panic!()
    };
    tokio::time::timeout(std::time::Duration::from_secs(3), answer.entered.notified())
        .await
        .unwrap();
    let scope = SteeringScope {
        conversation_id: id,
        turn_id: submission.turn_id,
    };
    let make_input = |text: &str| SteeringInput {
        scope,
        input_id: Uuid::new_v4(),
        text: text.into(),
        user_input: vec![],
        skill_instructions: vec![],
    };
    let first = make_input("guide before compact");
    steering.enqueue(first.clone()).await.unwrap();
    answer.release.notify_one();
    tokio::time::timeout(
        std::time::Duration::from_secs(3),
        compact.entered.notified(),
    )
    .await
    .unwrap();
    let history = service.open(ConversationId::from_uuid(id)).await.unwrap();
    assert!(
        !history
            .turn(scope.turn_id)
            .unwrap()
            .items
            .contains_key(&first.input_id)
    );
    let second = make_input("guide during compact");
    steering.enqueue(second.clone()).await.unwrap();
    compact.release.notify_one();
    assert_eq!(
        tokio::time::timeout(std::time::Duration::from_secs(3), owned.task.wait())
            .await
            .unwrap(),
        TaskCompletion::Completed
    );
    {
        let requests = transport.requests.lock().unwrap();
        assert_eq!(requests.len(), 3); // answer, summary, answer with both guides
        for guide in [&first, &second] {
            let contains = |request: &ModelRequest| {
                request.messages.iter().any(|message| {
                    matches!(message, ModelMessage::User { content: parts, .. } if parts.iter().any(|part|
                    matches!(part, ContentPart::Text(text) if text == &guide.text)))
                })
            };
            assert!(!contains(&requests[1]));
            assert!(contains(&requests[2]));
        }
        assert!(requests[1].messages.iter().any(|message| matches!(message,
            ModelMessage::Assistant { content, .. } if content.iter().any(|part|
                matches!(part, ContentPart::Text(text) if text.len() == 8000)))));
    }
    assert_eq!(tools.0.load(Ordering::SeqCst), 0);
    let history = service.open(ConversationId::from_uuid(id)).await.unwrap();
    for guide in [first, second] {
        assert!(history.turn(scope.turn_id).unwrap().items[&guide.input_id].completed);
    }
}
