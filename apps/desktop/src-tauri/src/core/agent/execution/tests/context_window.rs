use super::*;
use research_agent_core::context::{
    checkpoint::{CanonicalContext, ContextCheckpoint, store_messages},
    compaction_strategy::{CompactionBinding, CompactionKind, CompactionOutput, CompactionPhase},
    text_compaction::TextCompactionStrategy,
};

fn binding() -> CompactionBinding {
    CompactionBinding {
        model: model(),
        strategy_id: "research.text.v1".into(),
        state_schema: 1,
        kind: CompactionKind::TextSummary,
    }
}
struct InstallingTransport {
    inner: Arc<Transport>,
    db: crate::data_interface::SharedDatabase,
    conversation: Uuid,
    turn: Uuid,
    installed: std::sync::atomic::AtomicBool,
    canonical: bool,
    image: bool,
}
impl ModelTransport for InstallingTransport {
    fn capabilities(&self) -> ModelCapabilities {
        let mut capabilities = self.inner.capabilities();
        capabilities.vision = self.image;
        capabilities
    }
    fn start(&self, request: ModelRequest) -> ModelFuture<'_, Box<dyn ModelStream>> {
        Box::pin(async move {
            if !self.installed.swap(true, Ordering::SeqCst) {
                let mut db = self.db.lock().unwrap();
                let watermark = db
                    .query_row(
                        "select durable_count from agent_conversations where id=?1",
                        [self.conversation.to_string()],
                        |row| row.get(0),
                    )
                    .unwrap();
                crate::agent::repository::context_windows::install(
                    &mut db,
                    &ContextCheckpoint {
                        schema_version: if self.canonical { 2 } else { 1 },
                        window_id: Uuid::new_v4(),
                        conversation_id: self.conversation,
                        turn_id: self.turn,
                        source_watermark: watermark,
                        previous_window_id: None,
                        binding: binding(),
                        credential_scope_id: None,
                        phase: if self.canonical {
                            CompactionPhase::MidTurn
                        } else {
                            CompactionPhase::PreTurn
                        },
                        output: CompactionOutput::TextSummary(
                            "checkpoint: source S1; decisions retained".into(),
                        ),
                        retained: if self.canonical {
                            let mut parts = vec![ContentPart::Text("last retained user".into())];
                            if self.image {
                                parts.push(ContentPart::Image {
                                    media_type: "image/png".into(),
                                    bytes: Arc::from(retained_image().bytes),
                                });
                            }
                            store_messages(&[ModelMessage::User {
                                content: parts,
                                source_item_id: None,
                            }])
                            .unwrap()
                        } else {
                            vec![]
                        },
                        canonical_context: self.canonical.then(|| CanonicalContext {
                            before_retained: 0,
                            messages: store_messages(&[ModelMessage::User {
                                content: vec![ContentPart::Text(
                                    "frozen rules and loaded skill body".into(),
                                )],
                                source_item_id: None,
                            }])
                            .unwrap(),
                        }),
                    },
                )
                .unwrap();
            }
            self.inner.start(request).await
        })
    }
}

struct CoveredImagesUnavailable;
impl research_agent_core::submission::ImageMaterializer for CoveredImagesUnavailable {
    fn materialize_history(
        &self,
        _: Uuid,
        _: Uuid,
    ) -> research_agent_core::submission::SubmissionFuture<
        '_,
        Vec<research_agent_core::submission::SubmissionImage>,
    > {
        Box::pin(async { Err(research_agent_core::submission::SubmissionError::Unavailable) })
    }
    fn materialize(
        &self,
        _: Uuid,
        _: Uuid,
    ) -> research_agent_core::submission::SubmissionFuture<
        '_,
        Vec<research_agent_core::submission::SubmissionImage>,
    > {
        Box::pin(async { Ok(vec![]) })
    }
}

#[tokio::test]
async fn next_native_request_restores_checkpoint_and_only_appends_journal_tail() {
    restore_native_window(false, false).await;
}

#[tokio::test]
async fn next_native_request_restores_mid_turn_canonical_placement_and_current_authority() {
    restore_native_window(true, false).await;
}

fn retained_image() -> research_agent_core::submission::SubmissionImage {
    use sha2::Digest;
    let bytes = b"\x89PNG\r\n\x1a\nretained".to_vec();
    research_agent_core::submission::SubmissionImage {
        source_id: "image-source".into(),
        media_type: "image/png".into(),
        sha256: format!("{:x}", sha2::Sha256::digest(&bytes)),
        bytes,
    }
}
struct OnlyRetainedImages;
impl research_agent_core::submission::ImageMaterializer for OnlyRetainedImages {
    fn materialize_history(
        &self,
        _: Uuid,
        _: Uuid,
    ) -> research_agent_core::submission::SubmissionFuture<
        '_,
        Vec<research_agent_core::submission::SubmissionImage>,
    > {
        Box::pin(async { panic!("covered image objects must not be read in full") })
    }
    fn materialize_history_images(
        &self,
        _: Uuid,
        _: Uuid,
        hashes: Vec<String>,
    ) -> research_agent_core::submission::SubmissionFuture<
        '_,
        Vec<research_agent_core::submission::SubmissionImage>,
    > {
        Box::pin(async move {
            let image = retained_image();
            assert_eq!(hashes.as_slice(), std::slice::from_ref(&image.sha256));
            Ok(vec![image])
        })
    }
    fn materialize(
        &self,
        _: Uuid,
        _: Uuid,
    ) -> research_agent_core::submission::SubmissionFuture<
        '_,
        Vec<research_agent_core::submission::SubmissionImage>,
    > {
        Box::pin(async { Ok(vec![]) })
    }
}

#[tokio::test]
async fn next_native_request_reads_only_checkpoint_referenced_historical_images() {
    restore_native_window(true, true).await;
}

async fn restore_native_window(canonical: bool, image: bool) {
    let root = tempfile::tempdir().unwrap();
    let mut db = Connection::open_in_memory().unwrap();
    crate::storage::migrate(&mut db).unwrap();
    let app = crate::storage::AppStore::for_test(db, root.path().join("objects"));
    let service = Arc::new(AgentSessionService::from_app_store(&app).unwrap());
    let id = Uuid::new_v4();
    service.create(ConversationId::from_uuid(id)).await.unwrap();
    let store = Arc::new(SqliteSubmissionStore::new(app.shared_agent_connection()));
    let first = store
        .put(id, Uuid::new_v4(), input("old-evidence-never-repeat"))
        .await
        .unwrap();
    let inner = Arc::new(Transport {
        requests: Mutex::new(vec![]),
        responses: Mutex::new(VecDeque::from(vec![response(false), response(false)])),
    });
    let transport = Arc::new(InstallingTransport {
        inner: inner.clone(),
        db: app.shared_agent_connection(),
        conversation: id,
        turn: first.turn_id,
        installed: std::sync::atomic::AtomicBool::new(false),
        canonical,
        image,
    });
    let (_, _, _, _, mut execution, _) = setup().await;
    execution.transport = transport.clone();
    execution.compaction_strategy = Some(Arc::new(
        TextCompactionStrategy::new(transport, binding()).unwrap(),
    ));
    let execution = Arc::new(execution);
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
    let second = store
        .put(id, Uuid::new_v4(), input("new-evidence"))
        .await
        .unwrap();
    let admission = execution
        .start_with_images(
            service,
            store,
            if image {
                Arc::new(OnlyRetainedImages)
            } else {
                Arc::new(CoveredImagesUnavailable)
            },
            ConversationId::from_uuid(id),
            second.request_id,
        )
        .await
        .unwrap();
    let SubmissionAdmission::Started(owned) = admission else {
        panic!()
    };
    assert_eq!(owned.task.wait().await, TaskCompletion::Completed);

    let requests = inner.requests.lock().unwrap();
    if image {
        assert_eq!(
            requests[1]
                .messages
                .iter()
                .flat_map(|message| match message {
                    ModelMessage::User { content: parts, .. } => parts.as_slice(),
                    _ => &[],
                })
                .filter(|part| matches!(part, ContentPart::Image { .. }))
                .count(),
            1
        );
    }
    let texts = requests[1]
        .messages
        .iter()
        .flat_map(|message| match message {
            ModelMessage::System(text) => vec![text.as_str()],
            ModelMessage::CanonicalOutput(item) => match &item.normalized {
                Some(ModelOutputItem::Message(message)) => vec![message.text.as_str()],
                _ => vec![],
            },
            ModelMessage::User { content: parts, .. }
            | ModelMessage::Assistant { content: parts, .. } => parts
                .iter()
                .filter_map(|part| match part {
                    ContentPart::Text(text) => Some(text.as_str()),
                    _ => None,
                })
                .collect(),
            _ => vec![],
        })
        .collect::<Vec<_>>();
    assert!(
        texts
            .iter()
            .any(|text| text.contains("checkpoint: source S1"))
    );
    assert!(texts.iter().any(|text| text.contains("new-evidence")));
    assert!(
        !texts
            .iter()
            .any(|text| text.contains("old-evidence-never-repeat"))
    );
    assert_eq!(texts.iter().filter(|text| **text == "answer").count(), 1);
    if canonical {
        let index = |needle| texts.iter().position(|text| *text == needle).unwrap();
        assert_eq!(
            index("frozen rules and loaded skill body") + 1,
            index("last retained user")
        );
        assert!(index("last retained user") < index("checkpoint: source S1; decisions retained"));
        assert_eq!(
            texts
                .iter()
                .filter(|text| **text == "frozen rules and loaded skill body")
                .count(),
            1
        );
    }
    assert!(
        matches!(&requests[1].messages[0], ModelMessage::System(text) if text == "Research assistant")
    );
}

#[tokio::test]
async fn mid_turn_builder_installs_the_same_complete_window_that_it_returns() {
    let root = tempfile::tempdir().unwrap();
    let mut db = Connection::open_in_memory().unwrap();
    crate::storage::migrate(&mut db).unwrap();
    let app = crate::storage::AppStore::for_test(db, root.path().join("objects"));
    let service = Arc::new(AgentSessionService::from_app_store(&app).unwrap());
    let id = Uuid::new_v4();
    service.create(ConversationId::from_uuid(id)).await.unwrap();
    let store = SqliteSubmissionStore::new(app.shared_agent_connection());
    let mut submitted = input("frozen sources");
    submitted.text = "latest real user".into();
    let record = store.put(id, Uuid::new_v4(), submitted).await.unwrap();
    let (_, _, transport, _, mut execution, _) = setup().await;
    *transport.responses.lock().unwrap() = VecDeque::from(vec![response(false)]);
    execution.compaction_strategy = Some(text_strategy(transport.clone()));
    let worker = service.clone();
    let admission = service.start_submission(
        &store, ConversationId::from_uuid(id), record.request_id,
        move |record, _, signal| async move {
            let history = worker.open(ConversationId::from_uuid(id)).await.unwrap();
            let canonical = vec![ModelMessage::User { content: context_parts(&record)?, source_item_id: None }];
            let source = vec![ModelMessage::System(execution.system_instructions.clone()),
                ModelMessage::User { content: vec![ContentPart::Text(record.input.text.clone())], source_item_id: None }];
            let replacement = execution.compact_history(
                &worker, &record, &history, None, CompactionPhase::MidTurn,
                &signal, &source, &canonical, &[], &HashMap::new(),
            ).await?;
            let saved = worker.active_context_checkpoint(id).await.unwrap().unwrap();
            assert_eq!(saved.schema_version, 2);
            assert!(saved.canonical_context.is_some());
            let mut restored = vec![ModelMessage::System(execution.system_instructions.clone())];
            restored.extend(saved.restore_text_window(&|_| None).unwrap());
            assert!(matches!(&replacement[0], ModelMessage::System(text) if text == &execution.system_instructions));
            assert!(store_messages(&replacement[1..]).unwrap() == store_messages(&restored[1..]).unwrap());
            let canonical_index = replacement.iter().position(|message| matches!(message,
                ModelMessage::User { content: parts, .. } if parts.iter().any(|part| matches!(part, ContentPart::Text(text) if text.contains("Frozen research sources"))))).unwrap();
            assert!(matches!(&replacement[canonical_index + 1], ModelMessage::User { content: parts, .. }
                if matches!(&parts[0], ContentPart::Text(text) if text == "latest real user")));
            ServiceTurnSink { service: &worker, conversation_id: ConversationId::from_uuid(id), request_id: record.request_id }
                .emit(EventPayload::TurnFinished { turn_id: record.turn_id, outcome: TurnOutcome::Completed }).await?;
            Ok(())
        },
    ).await.unwrap();
    let SubmissionAdmission::Started(owned) = admission else {
        panic!()
    };
    assert_eq!(owned.task.wait().await, TaskCompletion::Completed);
    assert_eq!(transport.requests.lock().unwrap().len(), 1);
}
