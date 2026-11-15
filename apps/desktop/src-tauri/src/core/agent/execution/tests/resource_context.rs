use super::*;
use serde_json::json;

struct InstallingScope {
    inner: Arc<Transport>,
    db: crate::data_interface::SharedDatabase,
    checkpoint: Mutex<Option<research_agent_core::context::checkpoint::ContextCheckpoint>>,
}
impl ModelTransport for InstallingScope {
    fn capabilities(&self) -> ModelCapabilities {
        self.inner.capabilities()
    }
    fn start(&self, request: ModelRequest) -> ModelFuture<'_, Box<dyn ModelStream>> {
        Box::pin(async move {
            {
                if let Some(mut checkpoint) = self.checkpoint.lock().unwrap().take() {
                    let mut db = self.db.lock().unwrap();
                    checkpoint.source_watermark = db
                        .query_row(
                            "select durable_count from agent_conversations where id=?1",
                            [checkpoint.conversation_id.to_string()],
                            |row| row.get(0),
                        )
                        .unwrap();
                    crate::agent::repository::context_windows::install(&mut db, &checkpoint)
                        .unwrap();
                }
            }
            self.inner.start(request).await
        })
    }
}

fn grant(excluded: &[&str]) -> ContextText {
    use sha2::{Digest, Sha256};
    let resource = json!({"kind":"paperNotes","resourceId":"paper","excludedNoteIds":excluded});
    let identity = format!("{:x}", Sha256::digest(resource.to_string().as_bytes()));
    ContextText {
        source_id: format!("resource:{identity}"),
        kind: ContextSourceKind::Paper,
        resource_id: "paper".into(),
        revision: format!("grant:{identity}"),
        anchor: Some(
            json!({"content":"resourceGrant","resource":resource,"contextAccess":"readOnly"})
                .to_string(),
        ),
        text: "Reading notes; body not loaded".into(),
    }
}

fn scope_texts(request: &ModelRequest) -> Vec<&str> {
    request
        .messages
        .iter()
        .filter_map(|message| match message {
            ModelMessage::User { content: parts, .. } => Some(parts),
            _ => None,
        })
        .flatten()
        .filter_map(|part| match part {
            ContentPart::Text(text) if text.starts_with("<research_resource_scope>") => {
                Some(text.as_str())
            }
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn pre_turn_compaction_reinjects_unchanged_scope_then_next_turn_reuses_it() {
    let (service, store, transport, _, mut execution, id) = setup().await;
    *transport.responses.lock().unwrap() = (0..4).map(|_| response(false)).collect();
    execution.max_input_bytes = usize::MAX;
    execution.transport = token_compaction(transport.clone(), 3000);
    let execution = Arc::new(execution);
    for version in ["first", "second", "third"] {
        let mut draft = input(version);
        if version != "third" {
            draft.context[0].text =
                format!("{version} evidence ").repeat(if version == "first" { 1000 } else { 500 });
        }
        draft.context.push(grant(&[]));
        draft.text = format!("{version} question");
        let record = store.put(id, Uuid::new_v4(), draft).await.unwrap();
        assert_eq!(
            run(
                execution.clone(),
                service.clone(),
                store.clone(),
                id,
                record.request_id
            )
            .await,
            TaskCompletion::Completed
        );
    }
    {
        let requests = transport.requests.lock().unwrap();
        assert_eq!(requests.len(), 4);
        assert!(
            requests[1].tools.is_empty(),
            "second request must summarize the old window"
        );
        for request in requests.iter() {
            assert_eq!(scope_texts(request).len(), 1);
        }
        assert!(requests[2].messages.iter().any(|message|matches!(message,ModelMessage::User { content: parts, .. } if parts.iter().any(|part|matches!(part,ContentPart::Text(text) if text.contains("Context checkpoint handoff"))))));
    }
    assert!(
        service
            .active_context_checkpoint(id)
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn native_replay_reuses_scope_updates_exclusions_and_clears_revoked_access_after_reopen() {
    let ((service, store, transport, _, execution, id), db) = setup_with_connection().await;
    let mut service = service;
    let execution = Arc::new(execution);
    *transport.responses.lock().unwrap() = (0..6).map(|_| response(false)).collect();
    let scopes = vec![
        vec![grant(&[])],
        vec![grant(&[])],
        vec![grant(&["excluded-note"])],
        vec![],
        vec![],
        vec![grant(&[])],
    ];
    for (index, scope) in scopes.into_iter().enumerate() {
        let mut draft = input("fixture");
        draft.context = scope;
        draft.text = format!("Question {index}");
        let record = store.put(id, Uuid::new_v4(), draft).await.unwrap();
        // Reconstruct from durable history; no new in-memory permission baseline is required.
        if index == 1 {
            service = Arc::new(AgentSessionService::new(Arc::new(
                SqliteConversationStore::new(db.clone(), "scope-resume".into()).unwrap(),
            )));
        }
        assert_eq!(
            run(
                execution.clone(),
                service.clone(),
                store.clone(),
                id,
                record.request_id
            )
            .await,
            TaskCompletion::Completed
        );
    }
    let requests = transport.requests.lock().unwrap();
    let counts: Vec<_> = requests
        .iter()
        .map(|request| scope_texts(request).len())
        .collect();
    assert_eq!(counts, vec![1, 1, 2, 3, 3, 4]);
    let fragments = scope_texts(&requests[2]);
    let current: serde_json::Value =
        serde_json::from_str(fragments.last().unwrap().lines().nth(1).unwrap()).unwrap();
    assert_eq!(
        current[0]["overrides"],
        json!([{"resource":{"kind":"note","resourceId":"excluded-note"},
            "access":"readOnly","selected":false}])
    );
    assert!(scope_texts(&requests[3]).last().unwrap().contains("\n[]\n"));
    // No document bodies were loaded, and a stable selection did not get a second prompt fragment.
    assert!(!requests[1].messages.iter().any(|message|matches!(message,ModelMessage::User { content: parts, .. } if parts.iter().any(|part|matches!(part,ContentPart::Text(text) if text.contains("evidence fixture"))))));
}

#[tokio::test]
async fn mid_turn_checkpoint_scope_is_reused_but_legacy_checkpoint_rebuilds_it() {
    use research_agent_core::context::{
        checkpoint::{CanonicalContext, ContextCheckpoint, store_messages},
        compaction_strategy::{
            CompactionBinding, CompactionKind, CompactionOutput, CompactionPhase,
        },
    };
    for canonical in [false, true] {
        let ((service, store, transport, _, mut execution, id), db) = setup_with_connection().await;
        *transport.responses.lock().unwrap() =
            vec![response(false), response(false), response(false)].into();
        let mut draft = input("fixture");
        draft.context = vec![grant(&[])];
        let first = store.put(id, Uuid::new_v4(), draft.clone()).await.unwrap();
        let host_context = canonical.then(|| CanonicalContext {
            before_retained: 0,
            messages: store_messages(&[ModelMessage::User {
                content: context_parts(&first).unwrap(),
                source_item_id: None,
            }])
            .unwrap(),
        });
        let checkpoint = ContextCheckpoint {
            schema_version: if canonical { 2 } else { 1 },
            window_id: Uuid::new_v4(),
            conversation_id: id,
            turn_id: first.turn_id,
            source_watermark: 0,
            previous_window_id: None,
            binding: CompactionBinding {
                model: model(),
                strategy_id: "research.text.v1".into(),
                state_schema: 1,
                kind: CompactionKind::TextSummary,
            },
            credential_scope_id: None,
            phase: if canonical {
                CompactionPhase::MidTurn
            } else {
                CompactionPhase::PreTurn
            },
            output: CompactionOutput::TextSummary("Earlier work summary".into()),
            retained: vec![],
            canonical_context: host_context,
        };
        execution.transport = Arc::new(InstallingScope {
            inner: transport.clone(),
            db,
            checkpoint: Mutex::new(Some(checkpoint)),
        });
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
        for _ in 0..2 {
            let record = store.put(id, Uuid::new_v4(), draft.clone()).await.unwrap();
            assert_eq!(
                run(
                    execution.clone(),
                    service.clone(),
                    store.clone(),
                    id,
                    record.request_id
                )
                .await,
                TaskCompletion::Completed
            );
        }
        let requests = transport.requests.lock().unwrap();
        assert_eq!(scope_texts(&requests[1]).len(), 1);
        assert_eq!(scope_texts(&requests[2]).len(), 1);
        assert!(requests[1].messages.iter().any(|message|matches!(message,ModelMessage::User { content: parts, .. } if parts.iter().any(|part|matches!(part,ContentPart::Text(text) if text=="Earlier work summary")))));
    }
}
