use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Owner(AtomicUsize);
impl ToolExecutor for Owner {
    fn resolve_qualified_name(&self, namespace: Option<&str>, _: &str) -> ToolNameResolution {
        if namespace == Some("group") {
            ToolNameResolution::Registered
        } else {
            ToolNameResolution::NotFound
        }
    }
    fn requires_result_receipt(&self, _: &ToolCall) -> bool {
        false
    }
    fn supports_parallel(&self, _: &ToolCall) -> bool {
        true
    }
    fn cancellation_safe_to_drop(&self, _: &ToolCall) -> bool {
        true
    }
    fn finalization_call(&self) -> RunFuture<'_, Option<ToolCall>> {
        Box::pin(async { Ok(Some(call("finalize"))) })
    }
    fn release_for_finalization(&self) -> RunFuture<'_, bool> {
        Box::pin(async {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(true)
        })
    }
    fn validate(&self, _: &ToolCall) -> Result<(), RunError> {
        Ok(())
    }
    fn authorize_and_execute(&self, _: ToolCall) -> RunFuture<'_, ToolOutput> {
        Box::pin(async {
            Ok(ToolOutput {
                content: "done".into(),
                is_error: false,
            })
        })
    }
}

#[tokio::test]
async fn namespaced_previous_tool_does_not_enter_same_named_text_handler() {
    let mut fixture = fixture(true);
    fixture.tools.previous = Arc::new(Owner(AtomicUsize::new(0)));
    fixture
        .tools
        .previous_qualified_names
        .insert(("group".into(), READ.into()));
    let mut qualified = call(READ);
    qualified.namespace = Some("group".into());
    assert_eq!(
        fixture.tools.resolve_qualified_name(Some("group"), READ),
        ToolNameResolution::Registered
    );
    assert!(fixture.tools.supports_parallel(&qualified));
    assert!(fixture.tools.cancellation_safe_to_drop(&qualified));
    assert!(!fixture.tools.requires_result_receipt(&qualified));
    fixture.tools.validate(&qualified).unwrap();
    assert_eq!(
        fixture
            .tools
            .authorize_and_execute(qualified.clone())
            .await
            .unwrap()
            .content,
        "done"
    );
    fixture.tools.result_committed(qualified).await.unwrap();
    let mut foreign = call(READ);
    foreign.namespace = Some("foreign".into());
    assert!(matches!(
        fixture.tools.authorize_and_execute(foreign).await,
        Err(RunError::ToolNotFound)
    ));
}
fn call(name: &str) -> ToolCall {
    ToolCall {
        namespace: None,
        raw_arguments_json: None,
        call_id: "call".into(),
        name: name.into(),
        arguments: json!({}),
    }
}

#[tokio::test]
async fn text_owner_preserves_release_and_bounds_its_delivery_finalizer() {
    let mut fixture = fixture(true);
    let owner = Arc::new(Owner(AtomicUsize::new(0)));
    fixture.tools.previous = owner.clone();
    fixture.tools.previous_names.insert("finalize".into());
    for attempted in [false, true] {
        fixture
            .tools
            .delivery_attempted
            .store(attempted, Ordering::SeqCst);
        // This note-only fixture has no file delivery receipt. In particular,
        // a repeated previous offer must not restart a completed delivery.
        assert!(fixture.tools.finalization_call().await.unwrap().is_none());
    }
    assert!(fixture.tools.release_for_finalization().await.unwrap());
    assert_eq!(owner.0.load(Ordering::SeqCst), 1);
}

#[test]
fn live_editor_tools_do_not_inherit_parallel_or_drop_safe_metadata() {
    let mut fixture = fixture(true);
    fixture.tools.previous = Arc::new(Owner(AtomicUsize::new(0)));
    for name in [
        READ,
        PATCH,
        "research_read_source",
        "research_search_sources",
        crate::agent::writing_delivery_tool::NAME,
        "frozen_read",
    ] {
        fixture.tools.previous_names.insert(name.into());
    }
    for name in [
        READ,
        PATCH,
        "research_read_source",
        "research_search_sources",
        crate::agent::writing_delivery_tool::NAME,
        "unknown",
    ] {
        assert!(!fixture.tools.supports_parallel(&call(name)), "{name}");
        assert!(
            !fixture.tools.cancellation_safe_to_drop(&call(name)),
            "{name}"
        );
    }
    assert!(fixture.tools.supports_parallel(&call("frozen_read")));
    assert!(!fixture.tools.requires_result_receipt(&call("frozen_read")));
    assert!(fixture.tools.requires_result_receipt(&call(READ)));
    let mut patch = call(PATCH);
    patch.arguments = json!({"patch": "synthetic"});
    assert!(fixture.tools.requires_result_receipt(&patch));
    assert!(!fixture.tools.requires_result_receipt(&call(PATCH)));
    assert!(
        fixture
            .tools
            .cancellation_safe_to_drop(&call("frozen_read"))
    );
}
