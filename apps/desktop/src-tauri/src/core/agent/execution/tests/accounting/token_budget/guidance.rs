use super::*;
#[cfg(unix)]
#[path = "guidance/initial.rs"]
mod initial;
#[cfg(unix)]
#[path = "guidance/resume.rs"]
mod resume;
const GUIDANCE: &str = "  Keep durable notes.\nPreserve source anchors.  ";
fn configure(execution: &mut AgentExecution, transport: Arc<Transport>, after: u64) {
    buffered(execution, transport.clone(), after, false);
    execution.transport = Arc::new(Buffered {
        inner: transport,
        accounting: Arc::new(ContextAccounting {
            model: model(),
            budget: None,
            unknown_window_trigger_tokens: Some(100),
            estimator: Arc::new(Tokens { before: 40, after }),
        }),
        policy: Arc::new(
            TokenBudgetPolicy::resolve(TokenBudgetSettings {
                guidance_message: Some(GUIDANCE.into()),
                ..Default::default()
            })
            .unwrap(),
        ),
    });
}
fn occurrences(messages: &[ModelMessage]) -> usize {
    messages
        .iter()
        .flat_map(|message| match message {
            ModelMessage::Developer(text) => vec![text.as_str()],
            ModelMessage::User { content: parts, .. } => parts
                .iter()
                .filter_map(|part| match part {
                    ContentPart::Text(text) => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>(),
            _ => vec![],
        })
        .filter(|text| text.contains("<context_window_guidance>"))
        .count()
}
fn metadata(messages: &[ModelMessage]) -> Vec<String> {
    messages
        .iter()
        .filter_map(|message| match message {
            ModelMessage::Developer(body) if body.starts_with("<context_window>\n") => {
                Some(body.clone())
            }
            _ => None,
        })
        .collect()
}
#[tokio::test]
async fn reset_metadata_advances_identity_and_restores_once_in_actual_request() {
    use research_agent_core::context::token_budget::TokenBudgetContext;
    let (service, store, transport, tools, mut execution, id) = setup().await;
    *transport.responses.lock().unwrap() = VecDeque::from(vec![response(false)]);
    buffered(&mut execution, transport.clone(), 40, false);
    #[cfg(unix)]
    let hint_calls = install_hint_runtime(&service, "text").await;
    let execution = Arc::new(execution);
    let mut root = None;
    let mut previous = None;
    let mut saved = String::new();
    for _ in 0..2 {
        let record = store
            .put(id, Uuid::new_v4(), input("/compact"))
            .await
            .unwrap();
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
            panic!("new owner expected")
        };
        assert_eq!(owned.task.wait().await, TaskCompletion::Completed);
        let checkpoint = service
            .active_context_checkpoint(id)
            .await
            .unwrap()
            .unwrap();
        let fragments = metadata(&checkpoint.restore_reset_window(&|_| None).unwrap());
        assert_eq!(fragments.len(), 1);
        saved = fragments[0].clone();
        let fragment = TokenBudgetContext::from_body(&saved).unwrap().unwrap();
        #[cfg(unix)]
        assert_eq!(fragment.thread_hint.as_deref(), Some(" saved notes "));
        assert_eq!(fragment.conversation_id, id);
        assert_eq!(fragment.window_id, checkpoint.window_id);
        let first = *root.get_or_insert(fragment.first_window_id);
        assert_eq!(fragment.first_window_id, first);
        assert_eq!(fragment.previous_window_id, Some(previous.unwrap_or(first)));
        previous = Some(fragment.window_id);
    }
    assert!(transport.requests.lock().unwrap().is_empty());
    service
        .release_idle(ConversationId::from_uuid(id))
        .await
        .unwrap();
    service.open(ConversationId::from_uuid(id)).await.unwrap();
    let record = store
        .put(id, Uuid::new_v4(), input("continue"))
        .await
        .unwrap();
    assert_eq!(
        run(execution, service.clone(), store, id, record.request_id).await,
        TaskCompletion::Completed
    );
    {
        let requests = transport.requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(metadata(&requests[0].messages), vec![saved]);
    }
    assert_eq!(tools.0.load(Ordering::SeqCst), 0);
    #[cfg(unix)]
    assert_eq!(
        std::fs::read_to_string(hint_calls.path().join("calls"))
            .unwrap()
            .lines()
            .count(),
        2
    );
}

#[cfg(unix)]
async fn install_hint_runtime(service: &AgentSessionService, mode: &str) -> tempfile::TempDir {
    use crate::agent::{
        mcp_config::{ConfiguredServer, Transport as McpTransport},
        mcp_runtime::McpRuntime,
    };
    use std::{
        collections::{BTreeMap, HashSet},
        ffi::OsString,
        path::PathBuf,
        time::Duration,
    };
    let directory = tempfile::tempdir().unwrap();
    let script = r#"while IFS= read line; do case "$line" in *initialize*) printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{}}' ;; *tools/list*) printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"thread_hint","inputSchema":{"type":"object"}}]}}' ;; *tools/call*) printf '%s\n' called >> "$1"; id=${line#*\"id\":}; id=${id%%,*}; if [ "$HINT_FIXTURE_MODE" = hang ]; then continue; fi; if [ "$HINT_FIXTURE_MODE" = error ]; then printf '{"jsonrpc":"2.0","id":%s,"error":{"code":-1,"message":"fixture failure"}}\n' "$id"; continue; fi; printf '{"jsonrpc":"2.0","id":%s,"result":{"content":[{"text":" saved notes "}]}}\n' "$id" ;; esac; done"#;
    let server = ConfiguredServer {
        server_id: "notes".into(),
        environment_id: "project".into(),
        revision: "r1".into(),
        enabled: true,
        transport: McpTransport::Stdio {
            program: PathBuf::from("/bin/sh"),
            arguments: vec![
                OsString::from("-c"),
                OsString::from(script),
                OsString::from("hint-fixture"),
                directory.path().join("calls").into_os_string(),
            ],
            directory: directory.path().into(),
        },
        environment: BTreeMap::from([(OsString::from("HINT_FIXTURE_MODE"), OsString::from(mode))]),
        credential_handle: None,
    };
    service
        .install_mcp_runtime(Arc::new(
            McpRuntime::build(
                vec![server],
                &HashSet::from(["notes".into()]),
                Duration::from_secs(2),
            )
            .await
            .unwrap(),
        ))
        .await;
    directory
}

#[tokio::test]
async fn guidance_is_once_in_initial_and_reset_requests_and_survives_owner_recovery() {
    let (service, store, transport, tools, mut execution, id) = setup().await;
    configure(&mut execution, transport.clone(), 120);
    *transport.responses.lock().unwrap() =
        VecDeque::from(vec![response(true), response(false), response(false)]);
    let execution = Arc::new(execution);
    let record = store.put(id, Uuid::new_v4(), input("first")).await.unwrap();
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
    {
        let requests = transport.requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(occurrences(&requests[0].messages), 1);
        assert_eq!(occurrences(&requests[1].messages), 1);
        assert!(metadata(&requests[1].messages).is_empty()); // Unknown capacity stays unknown.
        assert!(requests[1].messages.iter().any(|message| matches!(message, ModelMessage::Developer(text) if text == &format!("<context_window_guidance>\n{GUIDANCE}\n</context_window_guidance>"))));
    }
    assert_eq!(tools.0.load(Ordering::SeqCst), 1);
    let checkpoint = service
        .active_context_checkpoint(id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(checkpoint.schema_version, 3);
    assert_eq!(
        checkpoint.phase,
        research_agent_core::context::compaction_strategy::CompactionPhase::MidTurn
    );
    assert_eq!(
        occurrences(&checkpoint.restore_reset_window(&|_| None).unwrap()),
        1
    );
    // Rebuild the actual session owner from its durable prefix; no model/tool replay.
    service
        .release_idle(ConversationId::from_uuid(id))
        .await
        .unwrap();
    service.open(ConversationId::from_uuid(id)).await.unwrap();
    let next = store.put(id, Uuid::new_v4(), input("next")).await.unwrap();
    assert_eq!(
        run(execution, service.clone(), store, id, next.request_id).await,
        TaskCompletion::Completed
    );
    assert_eq!(
        occurrences(&transport.requests.lock().unwrap()[2].messages),
        1
    );
    let history = service.open(ConversationId::from_uuid(id)).await.unwrap();
    assert!(history.turn_order().iter().all(|turn| {
        history
            .turn(*turn)
            .unwrap()
            .items
            .values()
            .all(|item| !item.text.contains("<context_window_guidance>"))
    }));
}

#[cfg(unix)]
#[tokio::test]
async fn reset_omits_failed_hint_but_cancellation_installs_no_window() {
    for mode in ["error", "hang"] {
        let (service, store, transport, tools, mut execution, id) = setup().await;
        buffered(&mut execution, transport.clone(), 40, false);
        let calls = install_hint_runtime(&service, mode).await;
        let record = store
            .put(id, Uuid::new_v4(), input("/compact"))
            .await
            .unwrap();
        let admission = Arc::new(execution)
            .start_compaction(
                service.clone(),
                store,
                ConversationId::from_uuid(id),
                record.request_id,
                None,
            )
            .await
            .unwrap();
        let SubmissionAdmission::Started(owned) = admission else {
            panic!("new owner expected")
        };
        if mode == "hang" {
            tokio::time::timeout(std::time::Duration::from_secs(4), async {
                while !calls.path().join("calls").exists() {
                    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                }
            })
            .await
            .unwrap();
            assert!(
                service
                    .tasks()
                    .cancel_turn(ConversationId::from_uuid(id), record.turn_id)
                    .unwrap()
            );
        }
        let completion = tokio::time::timeout(std::time::Duration::from_secs(4), owned.task.wait())
            .await
            .unwrap();
        assert_eq!(
            completion,
            if mode == "hang" {
                TaskCompletion::Canceled
            } else {
                TaskCompletion::Completed
            }
        );
        let checkpoint = service.active_context_checkpoint(id).await.unwrap();
        if mode == "hang" {
            assert!(checkpoint.is_none());
        } else {
            let bodies = metadata(&checkpoint.unwrap().restore_reset_window(&|_| None).unwrap());
            assert_eq!(bodies.len(), 1);
            let decoded =
                research_agent_core::context::token_budget::TokenBudgetContext::from_body(
                    &bodies[0],
                )
                .unwrap()
                .unwrap();
            assert!(decoded.thread_hint.is_none());
        }
        assert!(transport.requests.lock().unwrap().is_empty());
        assert_eq!(tools.0.load(Ordering::SeqCst), 0);
    }
}
#[tokio::test]
async fn repeated_manual_resets_rebuild_guidance_without_accumulating_old_fragments() {
    let (service, store, transport, _, mut execution, id) = setup().await;
    configure(&mut execution, transport.clone(), 40);
    let execution = Arc::new(execution);
    for _ in 0..2 {
        let record = store
            .put(id, Uuid::new_v4(), input("/compact"))
            .await
            .unwrap();
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
            panic!("new task expected")
        };
        assert_eq!(owned.task.wait().await, TaskCompletion::Completed);
        let checkpoint = service
            .active_context_checkpoint(id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            occurrences(&checkpoint.restore_reset_window(&|_| None).unwrap()),
            1
        );
    }
    assert!(transport.requests.lock().unwrap().is_empty());
}
