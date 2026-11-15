use super::*;
use crate::agent::{instructions::InstructionRootIdentity, repository::instruction_trust};
use std::{fs, os::unix::fs::MetadataExt};

struct SkillTransport {
    calls: AtomicUsize,
    identity: research_agent_core::skills::SkillIdentity,
    expected: &'static str,
}
impl ModelTransport for SkillTransport {
    fn capabilities(&self) -> ModelCapabilities {
        Transport(AtomicUsize::new(0)).capabilities()
    }
    fn start(&self, request: ModelRequest) -> ModelFuture<'_, Box<dyn ModelStream>> {
        let index = self.calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            assert!(
                request
                    .tools
                    .iter()
                    .any(|tool| tool.name == "research_load_skill")
            );
            let mut events = vec![ModelStreamEvent::Started {
                response_id: "skill-fixture".into(),
                actual_model: request.model,
            }];
            if index == 0 {
                assert!(request.messages.iter().any(|message| matches!(message,
                    ModelMessage::User { content: parts, .. } if parts.iter().any(|part| matches!(part, ContentPart::Text(text) if text.contains("Frozen skill catalog") && text.contains(&self.identity.revision))))));
                // Explicit instructions belong to the frozen submission, even
                // if the source changes or trust is revoked before execution.
                assert!(request.messages.iter().any(|message| matches!(message,
                    ModelMessage::User { content: parts, .. } if parts.iter().any(|part| matches!(part, ContentPart::Text(text) if text.contains("Skill body evidence"))))));
                assert!(!request.messages.iter().any(|message| matches!(message,
                    ModelMessage::User { content: parts, .. } if parts.iter().any(|part| matches!(part, ContentPart::Text(text) if text.contains("\nChanged"))))));
                events.extend([
                    ModelStreamEvent::ToolCallStarted {
                        call_id: "skill-load".into(),
                        name: "research_load_skill".into(),
                    },
                    ModelStreamEvent::ToolArgumentsDelta {
                        call_id: "skill-load".into(),
                        delta: serde_json::json!({"identity":self.identity}).to_string(),
                    },
                    ModelStreamEvent::ToolCallCompleted {
                        call_id: "skill-load".into(),
                    },
                    ModelStreamEvent::Completed {
                        reason: FinishReason::ToolCalls,
                        end_turn: None,
                    },
                ]);
            } else {
                assert!(request.messages.iter().any(|message| matches!(message,
                    ModelMessage::ToolResult { content, .. } if content.contains(self.expected))));
                if index == 1 && self.expected == "Skill body evidence" {
                    assert!(
                        request
                            .tools
                            .iter()
                            .any(|tool| tool.name == "research_read_skill_resource")
                    );
                    events.extend([
                        ModelStreamEvent::ToolCallStarted { call_id: "resource-read".into(), name: "research_read_skill_resource".into() },
                        ModelStreamEvent::ToolArgumentsDelta { call_id: "resource-read".into(), delta: serde_json::json!({"identity":self.identity,"path":"references/rules.md","expected_revision":null}).to_string() },
                        ModelStreamEvent::ToolCallCompleted { call_id: "resource-read".into() },
                        ModelStreamEvent::Completed { reason: FinishReason::ToolCalls, end_turn: None },
                    ]);
                } else {
                    if index == 2 {
                        assert!(request.messages.iter().any(|message| matches!(message,
                            ModelMessage::ToolResult { content, .. } if content.contains("Resource evidence") && content.contains("revision"))));
                    }
                    events.extend([
                        ModelStreamEvent::TextDelta("Reviewed".into()),
                        ModelStreamEvent::Completed {
                            reason: FinishReason::Stop,
                            end_turn: None,
                        },
                    ]);
                }
            }
            Ok(Box::new(Stream(events.into())) as Box<dyn ModelStream>)
        })
    }
}
#[tokio::test]
async fn confirmed_skill_runs_through_shared_loop_and_records_success_stale_and_revoked() {
    for expected in ["Skill body evidence", "stale", "unauthorized"] {
        let f = fixture(true);
        let service = WritingSubmissionService::from_app_store(&f.store);
        let root = f
            .store
            .manuscript_directory()
            .join(f.manuscript.to_string())
            .join("source")
            .canonicalize()
            .unwrap();
        fs::create_dir_all(root.join(".agents/skills/review")).unwrap();
        let path = root.join(".agents/skills/review/SKILL.md");
        fs::write(&path, "---\ndescription: Review\n---\nSkill body evidence").unwrap();
        fs::create_dir(root.join(".agents/skills/review/agents")).unwrap();
        fs::write(
            root.join(".agents/skills/review/agents/openai.yaml"),
            "policy: {allow_implicit_invocation: false}",
        )
        .unwrap();
        fs::create_dir(root.join(".agents/skills/review/references")).unwrap();
        fs::write(
            root.join(".agents/skills/review/references/rules.md"),
            "Resource evidence",
        )
        .unwrap();
        let metadata = fs::metadata(&root).unwrap();
        let identity = InstructionRootIdentity {
            canonical_path: &root,
            device: metadata.dev(),
            inode: metadata.ino(),
        };
        instruction_trust::update(
            &mut f.store.connection().unwrap(),
            f.manuscript,
            0,
            &identity,
            true,
        )
        .unwrap();
        let mut input = request(&f, Uuid::new_v4());
        input.text = "Use $review to review this manuscript".into();
        let prepared = service.prepare(&f.models, input).await.unwrap();
        let record = prepared.record;
        assert!(record.input.skills[0].explicit);
        assert!(!record.input.skills[0].descriptor.allow_implicit_invocation);
        assert_eq!(record.input.skill_instructions.len(), 1);
        assert_eq!(
            record.input.skill_instructions[0].identity,
            record.input.skills[0].descriptor.identity
        );
        assert!(
            record.input.skill_instructions[0]
                .contents
                .starts_with("Skill body evidence")
        );
        if expected == "stale" {
            fs::write(&path, "---\ndescription: Review\n---\nChanged").unwrap();
        }
        if expected == "unauthorized" {
            instruction_trust::update(
                &mut f.store.connection().unwrap(),
                f.manuscript,
                1,
                &identity,
                false,
            )
            .unwrap();
        }
        let transport = Arc::new(SkillTransport {
            calls: AtomicUsize::new(0),
            identity: record.input.skills[0].descriptor.identity.clone(),
            expected,
        });
        let sessions = Arc::new(AgentSessionService::new(Arc::new(
            SqliteConversationStore::new(f.store.shared_agent_connection(), "fixture".into())
                .unwrap(),
        )));
        let mut execution_profile = profile(record.input.model.clone());
        // This tests skill routing, not initial compaction of a growing catalog.
        execution_profile.max_input_bytes = 50_000;
        let admission = service
            .start_transport(
                sessions.clone(),
                record.clone(),
                execution_profile,
                transport.clone(),
            )
            .await
            .unwrap();
        let SubmissionAdmission::Started(owned) = admission else {
            panic!()
        };
        assert_eq!(owned.task.wait().await, TaskCompletion::Completed);
        assert_eq!(
            transport.calls.load(Ordering::SeqCst),
            if expected == "Skill body evidence" {
                3
            } else {
                2
            }
        );
        let projection = sessions
            .open(ConversationId::from_uuid(record.conversation_id))
            .await
            .unwrap();
        let turn = projection.turn(record.turn_id).unwrap();
        if expected == "Skill body evidence" {
            assert!(turn.items.values().any(|item| item.completed
                && item.text.contains("Resource evidence")
                && item.text.contains("revision")));
            let history = sessions
                .read_since(ConversationId::from_uuid(record.conversation_id), 0, 100)
                .await
                .unwrap();
            assert!(!history.items.iter().any(|event| matches!(
                event.payload(),
                research_agent_core::protocol::EventPayload::ToolReceiptPending { .. }
                    | research_agent_core::protocol::EventPayload::ToolReceiptConfirmed { .. }
            )));
        }
        assert!(
            turn.items
                .values()
                .any(|item| item.completed && item.text.contains(expected))
        );
    }
}
