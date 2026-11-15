//! Manual remote compaction advertises current schemas without dispatching tools.
use super::*;
use research_agent_core::model::{ToolCall, ToolDefinition};
use std::sync::atomic::{AtomicUsize, Ordering};

struct UncalledTools(AtomicUsize);
impl ToolExecutor for UncalledTools {
    fn validate(&self, _: &ToolCall) -> Result<(), RunError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(RunError::ToolRejected)
    }
    fn authorize_and_execute(&self, _: ToolCall) -> RunFuture<'_, ToolOutput> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Err(RunError::ToolRejected) })
    }
}

#[tokio::test]
async fn manual_v2_uses_current_tool_catalog_without_execution() {
    exercise(false).await;
}

#[tokio::test]
async fn manual_full_window_uses_current_tool_catalog_without_execution() {
    exercise(true).await;
}

async fn exercise(full_window: bool) {
    let fixture = Fixture::new().await;
    let call = json!({"type":"function_call","id":"ignored-tool-item",
        "call_id":"ignored-call","name":"read","arguments":"{}"});
    let compact_response = if full_window {
        let mut items = legacy::full_window("tool-aware window");
        items.push(call.clone());
        legacy::json_response(&items)
    } else {
        ok(&events(&[opaque(), call], true))
    };
    let (url, peer) = sequential_server(vec![ok(&body()), compact_response, ok(&body())]).await;
    let tools = Arc::new(UncalledTools(AtomicUsize::new(0)));
    for (text, compact, description) in [
        ("before", false, "old catalog"),
        ("/compact", true, "current frozen catalog"),
        ("after", false, "current frozen catalog"),
    ] {
        let record = fixture
            .record_with_context(
                text,
                vec![research_agent_core::submission::ContextText {
                    source_id: "frozen-source".into(),
                    kind: research_agent_core::submission::ContextSourceKind::Paper,
                    resource_id: "paper".into(),
                    revision: "version-1".into(),
                    anchor: None,
                    text: "authorized evidence".into(),
                }],
            )
            .await;
        let mut execution = if full_window {
            legacy::legacy_execution(&fixture, &record, &url)
        } else {
            fixture.execution(&record, fixture.transport(&record, &url))
        };
        execution.host_tools = OwnedToolRegistration::new(
            vec![ToolDefinition {
                namespace: None,
                name: "read".into(),
                description: description.into(),
                parameters: json!({"type":"object","properties":{"limit":{"type":"integer","minimum":1}},"required":["limit"],"additionalProperties":false}),
            }],
            tools.clone(),
        );
        assert_eq!(
            fixture
                .start(&record, execution, compact)
                .await
                .task
                .wait()
                .await,
            TaskCompletion::Completed
        );
    }
    let sent = peer.await.unwrap();
    assert_eq!(sent.len(), 3);
    assert_eq!(sent[1]["tools"], sent[2]["tools"]);
    assert_ne!(sent[0]["tools"], sent[1]["tools"]);
    let catalog = sent[1]["tools"].as_array().unwrap();
    assert_eq!(catalog.len(), 4); // host tool plus the frozen source catalog
    assert_eq!(catalog[0]["name"], "read");
    assert_eq!(catalog[0]["description"], "current frozen catalog");
    assert_eq!(
        catalog[0]["parameters"]["properties"]["limit"]["minimum"],
        1
    );
    assert!(
        catalog
            .iter()
            .any(|tool| tool["name"] == "research_read_source")
    );
    assert!(!sent[1].to_string().contains("/compact"));
    assert_eq!(tools.0.load(Ordering::SeqCst), 0);
    let history = fixture
        .service
        .open(ConversationId::from_uuid(fixture.id))
        .await
        .unwrap();
    assert!(
        history
            .turn_order()
            .iter()
            .all(|id| history
                .turn(*id)
                .unwrap()
                .items
                .values()
                .all(|item| !matches!(
                    item.kind,
                    research_agent_core::protocol::ItemKind::ToolCall { .. }
                        | research_agent_core::protocol::ItemKind::ToolResult { .. }
                )))
    );
}
