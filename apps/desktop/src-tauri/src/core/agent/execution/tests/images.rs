use super::*;
use research_agent_core::submission::{
    ImageMaterializer, SubmissionError, SubmissionFuture, SubmissionImage,
};
use sha2::Digest;

struct EmptyImages;
struct EvidenceSubmissionStore {
    inner: Arc<dyn SubmissionStore>,
    bytes: Vec<u8>,
}
impl SubmissionStore for EvidenceSubmissionStore {
    fn get_by_turn(&self, c: Uuid, t: Uuid) -> SubmissionFuture<'_, SubmissionRecord> {
        self.inner.get_by_turn(c, t)
    }
    fn put(
        &self,
        c: Uuid,
        r: Uuid,
        input: research_agent_core::submission::SubmissionInput,
    ) -> SubmissionFuture<'_, SubmissionRecord> {
        self.inner.put(c, r, input)
    }
    fn get(&self, c: Uuid, r: Uuid) -> SubmissionFuture<'_, SubmissionRecord> {
        self.inner.get(c, r)
    }
    fn evidence_content<'a>(
        &'a self,
        input: &'a [research_agent_core::user_input::UserInput],
    ) -> SubmissionFuture<'a, Vec<ContentPart>> {
        Box::pin(async move {
            research_agent_core::user_input::evidence::content(input, |_| Ok(self.bytes.clone()))
        })
    }
}
struct EvidenceVisionTransport(Arc<Transport>);
impl ModelTransport for EvidenceVisionTransport {
    fn capabilities(&self) -> ModelCapabilities {
        let mut caps = self.0.capabilities();
        caps.vision = true;
        caps
    }
    fn start(&self, request: ModelRequest) -> ModelFuture<'_, Box<dyn ModelStream>> {
        self.0.start(request)
    }
}
#[tokio::test]
async fn captured_annotation_images_are_sent_and_restored_as_image_parts() {
    use research_agent_core::user_input::{UserInput, evidence::EvidenceImage};
    let (service, store, transport, _, mut execution, id) = setup().await;
    execution.transport = Arc::new(EvidenceVisionTransport(transport.clone()));
    let mut selected = input("v1");
    let bytes = b"\x89PNG\r\n\x1a\nfixture".to_vec();
    selected.user_input = vec![
        UserInput::Text {
            text: selected.text.clone(),
            text_elements: vec![],
        },
        UserInput::Annotation {
            source: Some("source://note:#block{image}".into()),
            title: "Figure".into(),
            text: "caption".into(),
            comment: "Explain this diagram".into(),
            image: Some(EvidenceImage {
                media_type: "image/png".into(),
                asset_id: format!("{:x}", sha2::Sha256::digest(&bytes)),
                capture_id: Uuid::from_u128(1),
                byte_size: bytes.len(),
            }),
            display_text: None,
        },
    ];
    let first = store.put(id, Uuid::new_v4(), selected).await.unwrap();
    let store: Arc<dyn SubmissionStore> = Arc::new(EvidenceSubmissionStore {
        inner: store,
        bytes: bytes.clone(),
    });
    let execution = Arc::new(execution);
    let admission = execution
        .clone()
        .start(
            service.clone(),
            store.clone(),
            ConversationId::from_uuid(id),
            first.request_id,
        )
        .await
        .unwrap();
    let SubmissionAdmission::Started(owned) = admission else {
        panic!()
    };
    assert_eq!(owned.task.wait().await, TaskCompletion::Completed);
    let mut plain = input("v2");
    plain.text = "Next question".into();
    let second = store.put(id, Uuid::new_v4(), plain).await.unwrap();
    let admission = execution
        .start(
            service,
            store,
            ConversationId::from_uuid(id),
            second.request_id,
        )
        .await
        .unwrap();
    let SubmissionAdmission::Started(owned) = admission else {
        panic!()
    };
    assert_eq!(owned.task.wait().await, TaskCompletion::Completed);
    for request in transport.requests.lock().unwrap().iter() {
        let images: Vec<_> = request
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
                if let ContentPart::Image { bytes, .. } = part {
                    Some(bytes)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(images.len(), 1);
        assert_eq!(images[0].as_ref(), bytes.as_slice());
    }
}
#[tokio::test]
async fn selected_evidence_stays_with_its_original_user_item_in_history() {
    use research_agent_core::user_input::UserInput;
    let (service, store, transport, _, execution, id) = setup().await;
    let mut selected = input("v1");
    selected.user_input = vec![
        UserInput::Text {
            text: selected.text.clone(),
            text_elements: vec![],
        },
        UserInput::Annotation {
            source: Some("source://note:#block{b}".into()),
            title: "quote".into(),
            text: "captured evidence".into(),
            comment: String::new(),
            image: None,
            display_text: None,
        },
        UserInput::Annotation {
            source: None,
            title: "对话选段".into(),
            text: "prior chat quotation".into(),
            comment: "explain this".into(),
            image: None,
            display_text: None,
        },
    ];
    let first = store.put(id, Uuid::new_v4(), selected).await.unwrap();
    let execution = Arc::new(execution);
    let admission = execution
        .clone()
        .start(
            service.clone(),
            store.clone(),
            ConversationId::from_uuid(id),
            first.request_id,
        )
        .await
        .unwrap();
    let SubmissionAdmission::Started(owned) = admission else {
        panic!()
    };
    assert_eq!(owned.task.wait().await, TaskCompletion::Completed);
    let mut plain = input("v2");
    plain.text = "Second question".into();
    let second = store.put(id, Uuid::new_v4(), plain).await.unwrap();
    let admission = execution
        .start(
            service,
            store,
            ConversationId::from_uuid(id),
            second.request_id,
        )
        .await
        .unwrap();
    let SubmissionAdmission::Started(owned) = admission else {
        panic!()
    };
    assert_eq!(owned.task.wait().await, TaskCompletion::Completed);
    let requests = transport.requests.lock().unwrap();
    for request in requests.iter() {
        let referenced: Vec<_> = request.messages.iter().filter_map(|message| match message {ModelMessage::User { content: parts, .. } if parts.iter().any(|part|matches!(part,ContentPart::Text(text) if text.contains("<research_reference>"))) => Some(parts),_=>None}).collect();
        assert_eq!(referenced.len(), 1);
        assert!(
            referenced[0]
                .iter()
                .any(|part| matches!(part,ContentPart::Text(text) if text == "Explain"))
        );
        assert!(
            !referenced[0]
                .iter()
                .any(|part| matches!(part,ContentPart::Text(text) if text == "Second question"))
        );
        let quote = referenced[0]
            .iter()
            .find_map(|part| match part {
                ContentPart::Text(text) if text.contains("prior chat quotation") => Some(text),
                _ => None,
            })
            .expect("chat quote remains in the original model message");
        let envelope = quote
            .split("<research_reference>\n")
            .nth(1)
            .unwrap()
            .split("\n</research_reference>")
            .next()
            .unwrap();
        let evidence: serde_json::Value = serde_json::from_str(envelope).unwrap();
        assert_eq!(evidence["selectedText"], "prior chat quotation");
        assert_eq!(evidence["userComment"], "explain this");
        assert!(evidence.get("source").is_none());
    }
}
impl ImageMaterializer for EmptyImages {
    fn materialize_history(&self, _: Uuid, _: Uuid) -> SubmissionFuture<'_, Vec<SubmissionImage>> {
        Box::pin(async { Ok(vec![]) })
    }
    fn materialize(&self, _: Uuid, _: Uuid) -> SubmissionFuture<'_, Vec<SubmissionImage>> {
        Box::pin(async { Ok(vec![]) })
    }
}

#[tokio::test]
async fn empty_image_manifest_preserves_text_and_tool_execution() {
    let (service, store, transport, tools, execution, id) = setup().await;
    let record = store.put(id, Uuid::new_v4(), input("v1")).await.unwrap();
    let admission = Arc::new(execution)
        .start_with_images(
            service,
            store,
            Arc::new(EmptyImages),
            ConversationId::from_uuid(id),
            record.request_id,
        )
        .await
        .unwrap();
    let SubmissionAdmission::Started(owned) = admission else {
        panic!()
    };
    assert_eq!(owned.task.wait().await, TaskCompletion::Completed);
    assert_eq!(tools.0.load(Ordering::SeqCst), 1);
    let requests = transport.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(requests[0].messages.iter().any(|message| matches!(message,
        ModelMessage::User { content: parts, .. } if parts.iter().any(|part| matches!(part,
            ContentPart::Text(text) if text == "Explain")))));
}

struct FailingImages {
    conversation_id: Uuid,
    request_id: Uuid,
    unavailable: bool,
    calls: AtomicUsize,
}
impl ImageMaterializer for FailingImages {
    fn materialize_history(&self, _: Uuid, _: Uuid) -> SubmissionFuture<'_, Vec<SubmissionImage>> {
        panic!("first turn must not load history")
    }
    fn materialize(
        &self,
        conversation_id: Uuid,
        request_id: Uuid,
    ) -> SubmissionFuture<'_, Vec<SubmissionImage>> {
        assert_eq!(conversation_id, self.conversation_id);
        assert_eq!(request_id, self.request_id);
        self.calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            Err(if self.unavailable {
                SubmissionError::Unavailable
            } else {
                SubmissionError::InvalidInput
            })
        })
    }
}

#[tokio::test]
async fn image_read_failure_finishes_turn_without_model_or_duplicate_read() {
    for unavailable in [false, true] {
        let (service, store, transport, _, execution, id) = setup().await;
        let record = store.put(id, Uuid::new_v4(), input("v1")).await.unwrap();
        let images = Arc::new(FailingImages {
            conversation_id: id,
            request_id: record.request_id,
            unavailable,
            calls: AtomicUsize::new(0),
        });
        let execution = Arc::new(execution);
        let admission = execution
            .clone()
            .start_with_images(
                service.clone(),
                store.clone(),
                images.clone(),
                ConversationId::from_uuid(id),
                record.request_id,
            )
            .await
            .unwrap();
        let SubmissionAdmission::Started(owned) = admission else {
            panic!()
        };
        assert_eq!(owned.task.wait().await, TaskCompletion::Failed);
        let projection = service.open(ConversationId::from_uuid(id)).await.unwrap();
        assert_eq!(projection.active_turn(), None);
        assert_eq!(
            projection.turn(record.turn_id).unwrap().outcome,
            Some(TurnOutcome::Failed)
        );
        assert!(transport.requests.lock().unwrap().is_empty());
        let retry = execution
            .start_with_images(
                service,
                store,
                images.clone(),
                ConversationId::from_uuid(id),
                record.request_id,
            )
            .await
            .unwrap();
        assert!(matches!(
            retry,
            SubmissionAdmission::AlreadyStarted {
                outcome: Some(TurnOutcome::Failed),
                ..
            }
        ));
        assert_eq!(images.calls.load(Ordering::SeqCst), 1);
    }
}
