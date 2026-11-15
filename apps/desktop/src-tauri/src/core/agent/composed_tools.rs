//! Shared composition for host tools plus optional external tools.
//!
//! This mirrors the small routing boundary used by Codex's tool executor:
//! registered names select one exact handler, while namespaced extension tools
//! are delegated to an explicitly registered fallback group. Keeping this in
//! one adapter prevents each host from inventing subtly different validation
//! and acknowledgement behavior.
use research_agent_core::{
    model::{ToolCall, ToolDefinition},
    runner::{RunError, RunFuture, ToolExecutor, ToolNameResolution, ToolOutput},
};
use std::{collections::HashSet, sync::Arc};

pub struct ComposedToolExecutor {
    pub primary: Arc<dyn ToolExecutor>,
    primary_names: HashSet<(Option<String>, String)>,
    pub fallback: Option<Arc<dyn ToolExecutor>>,
    fallback_names: HashSet<(Option<String>, String)>,
}

impl ComposedToolExecutor {
    pub fn new(
        primary: Arc<dyn ToolExecutor>,
        primary_definitions: impl IntoIterator<Item = ToolDefinition>,
        fallback: Option<Arc<dyn ToolExecutor>>,
        fallback_definitions: impl IntoIterator<Item = ToolDefinition>,
    ) -> Self {
        Self {
            primary,
            primary_names: primary_definitions
                .into_iter()
                .map(|tool| (tool.namespace, tool.name))
                .collect(),
            fallback,
            fallback_names: fallback_definitions
                .into_iter()
                .map(|tool| (tool.namespace, tool.name))
                .collect(),
        }
    }

    fn select(&self, call: &ToolCall) -> Result<&dyn ToolExecutor, RunError> {
        let key = (call.namespace.clone(), call.name.clone());
        let owner = if self.primary_names.contains(&key) {
            self.primary.as_ref()
        } else if self.fallback_names.contains(&key) {
            self.fallback.as_deref().ok_or(RunError::ToolRejected)?
        } else {
            return Err(RunError::ToolNotFound);
        };
        if call.namespace.is_some()
            && owner.resolve_qualified_name(call.namespace.as_deref(), &call.name)
                != ToolNameResolution::Registered
        {
            return Err(RunError::ToolNotFound);
        }
        Ok(owner)
    }
}

impl ToolExecutor for ComposedToolExecutor {
    fn child_creation_intent(
        &self,
        call: &ToolCall,
    ) -> Option<research_agent_core::conversation::CreateBoundConversation> {
        self.select(call).ok()?.child_creation_intent(call)
    }
    fn resolve_name(&self, name: &str) -> ToolNameResolution {
        self.resolve_qualified_name(None, name)
    }
    fn resolve_qualified_name(&self, namespace: Option<&str>, name: &str) -> ToolNameResolution {
        let key = (namespace.map(str::to_owned), name.to_owned());
        let owner = if self.primary_names.contains(&key) {
            Some(self.primary.as_ref())
        } else if self.fallback_names.contains(&key) {
            self.fallback.as_deref()
        } else {
            return ToolNameResolution::NotFound;
        };
        match owner.map(|owner| owner.resolve_qualified_name(namespace, name)) {
            Some(ToolNameResolution::Unspecified) if namespace.is_none() => {
                ToolNameResolution::Registered
            }
            Some(status) => status,
            None => ToolNameResolution::Unspecified,
        }
    }
    fn cancel_pending(&self, call: ToolCall) -> RunFuture<'_, ()> {
        match self.select(&call) {
            Ok(tool) => tool.cancel_pending(call),
            Err(error) => Box::pin(async move { Err(error) }),
        }
    }
    fn supports_parallel(&self, call: &ToolCall) -> bool {
        self.select(call)
            .is_ok_and(|tool| tool.supports_parallel(call))
    }
    fn cancellation_safe_to_drop(&self, call: &ToolCall) -> bool {
        self.select(call)
            .is_ok_and(|tool| tool.cancellation_safe_to_drop(call))
    }
    fn requires_result_receipt(&self, call: &ToolCall) -> bool {
        self.select(call)
            .map_or(true, |tool| tool.requires_result_receipt(call))
    }
    fn finalization_call(&self) -> RunFuture<'_, Option<ToolCall>> {
        Box::pin(async {
            if let Some(call) = self.primary.finalization_call().await? {
                return Ok(Some(call));
            }
            if let Some(fallback) = &self.fallback {
                return fallback.finalization_call().await;
            }
            Ok(None)
        })
    }
    fn release_for_finalization(&self) -> RunFuture<'_, bool> {
        Box::pin(async {
            let primary = self.primary.release_for_finalization().await?;
            if let Some(fallback) = &self.fallback {
                return Ok(fallback.release_for_finalization().await? || primary);
            }
            Ok(primary)
        })
    }
    fn validate(&self, call: &ToolCall) -> Result<(), RunError> {
        self.select(call)?.validate(call)
    }

    fn authorize_and_execute(&self, call: ToolCall) -> RunFuture<'_, ToolOutput> {
        match self.select(&call) {
            Ok(tool) => tool.authorize_and_execute(call),
            Err(error) => Box::pin(async move { Err(error) }),
        }
    }

    fn result_committed(&self, call: ToolCall) -> RunFuture<'_, ()> {
        match self.select(&call) {
            Ok(tool) => tool.result_committed(call),
            Err(error) => Box::pin(async move { Err(error) }),
        }
    }
}

pub struct RejectingToolExecutor;
impl ToolExecutor for RejectingToolExecutor {
    fn resolve_name(&self, _: &str) -> ToolNameResolution {
        ToolNameResolution::NotFound
    }
    fn validate(&self, _: &ToolCall) -> Result<(), RunError> {
        Err(RunError::ToolRejected)
    }
    fn authorize_and_execute(&self, _: ToolCall) -> RunFuture<'_, ToolOutput> {
        Box::pin(async { Err(RunError::ToolRejected) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Fake {
        name: &'static str,
        calls: Arc<AtomicUsize>,
    }

    impl ToolExecutor for Fake {
        fn resolve_qualified_name(
            &self,
            namespace: Option<&str>,
            name: &str,
        ) -> ToolNameResolution {
            if namespace.is_none() {
                self.resolve_name(name)
            } else if name == self.name {
                ToolNameResolution::Registered
            } else {
                ToolNameResolution::NotFound
            }
        }
        fn validate(&self, call: &ToolCall) -> Result<(), RunError> {
            (call.name == self.name)
                .then_some(())
                .ok_or(RunError::ToolRejected)
        }
        fn authorize_and_execute(&self, call: ToolCall) -> RunFuture<'_, ToolOutput> {
            let accepted = call.name == self.name;
            if accepted {
                self.calls.fetch_add(1, Ordering::SeqCst);
            }
            Box::pin(async move {
                if accepted {
                    Ok(ToolOutput {
                        content: "ok".into(),
                        is_error: false,
                    })
                } else {
                    Err(RunError::ToolRejected)
                }
            })
        }
    }

    fn call(name: &str) -> ToolCall {
        ToolCall {
            namespace: None,
            raw_arguments_json: None,
            call_id: "c".into(),
            name: name.into(),
            arguments: json!({}),
        }
    }

    fn definition(name: &str) -> ToolDefinition {
        ToolDefinition {
            namespace: None,
            name: name.into(),
            description: name.into(),
            parameters: json!({"type":"object"}),
        }
    }

    #[tokio::test]
    async fn routes_primary_then_fallback_without_second_runner() {
        let primary_calls = Arc::new(AtomicUsize::new(0));
        let fallback_calls = Arc::new(AtomicUsize::new(0));
        let tools = ComposedToolExecutor::new(
            Arc::new(Fake {
                name: "write",
                calls: primary_calls.clone(),
            }),
            [definition("write")],
            Some(Arc::new(Fake {
                name: "mcp__search",
                calls: fallback_calls.clone(),
            })),
            [definition("mcp__search")],
        );
        tools.authorize_and_execute(call("write")).await.unwrap();
        tools
            .authorize_and_execute(call("mcp__search"))
            .await
            .unwrap();
        assert_eq!(primary_calls.load(Ordering::SeqCst), 1);
        assert_eq!(fallback_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn registered_primary_name_never_falls_through_after_validation_failure() {
        let primary_calls = Arc::new(AtomicUsize::new(0));
        let fallback_calls = Arc::new(AtomicUsize::new(0));
        let tools = ComposedToolExecutor::new(
            Arc::new(Fake {
                name: "different",
                calls: primary_calls.clone(),
            }),
            [definition("overlap")],
            Some(Arc::new(Fake {
                name: "overlap",
                calls: fallback_calls.clone(),
            })),
            [definition("overlap")],
        );

        assert!(tools.validate(&call("overlap")).is_err());
        assert!(tools.authorize_and_execute(call("overlap")).await.is_err());
        assert_eq!(primary_calls.load(Ordering::SeqCst), 0);
        assert_eq!(fallback_calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn registered_fallback_runs_its_own_validation() {
        let calls = Arc::new(AtomicUsize::new(0));
        let tools = ComposedToolExecutor::new(
            Arc::new(RejectingToolExecutor),
            [],
            Some(Arc::new(Fake {
                name: "different",
                calls: calls.clone(),
            })),
            [definition("reader")],
        );
        assert!(tools.validate(&call("reader")).is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn rejects_unknown_tools_when_no_fallback_exists() {
        let tools = ComposedToolExecutor::new(
            Arc::new(Fake {
                name: "write",
                calls: Arc::new(AtomicUsize::new(0)),
            }),
            [definition("write")],
            None,
            std::iter::empty(),
        );
        assert!(matches!(
            tools.validate(&call("unknown")),
            Err(RunError::ToolNotFound)
        ));
        assert_eq!(tools.resolve_name("unknown"), ToolNameResolution::NotFound);
        assert_eq!(tools.resolve_name("write"), ToolNameResolution::Registered);
        let mut foreign = call("write");
        foreign.namespace = Some("other".into());
        assert!(matches!(
            tools.validate(&foreign),
            Err(RunError::ToolNotFound)
        ));
    }

    #[test]
    fn name_lookup_preserves_selected_owner_rejection_and_missing_owner_uncertainty() {
        let tools = ComposedToolExecutor::new(
            Arc::new(Fake {
                name: "different",
                calls: Arc::new(AtomicUsize::new(0)),
            }),
            [definition("declared")],
            None,
            [definition("missing-owner")],
        );
        assert_eq!(
            tools.resolve_name("declared"),
            ToolNameResolution::Registered
        );
        assert!(matches!(
            tools.validate(&call("declared")),
            Err(RunError::ToolRejected)
        ));
        assert_eq!(
            tools.resolve_name("missing-owner"),
            ToolNameResolution::Unspecified
        );
        assert!(matches!(
            tools.validate(&call("missing-owner")),
            Err(RunError::ToolRejected)
        ));
    }

    #[tokio::test]
    async fn same_bare_name_routes_by_namespace_through_all_phases() {
        let primary_calls = Arc::new(AtomicUsize::new(0));
        let fallback_calls = Arc::new(AtomicUsize::new(0));
        let mut qualified = definition("read");
        qualified.namespace = Some("space".into());
        let tools = ComposedToolExecutor::new(
            Arc::new(Fake {
                name: "read",
                calls: primary_calls.clone(),
            }),
            [definition("read")],
            Some(Arc::new(Fake {
                name: "read",
                calls: fallback_calls.clone(),
            })),
            [qualified],
        );
        let mut qualified_call = call("read");
        qualified_call.namespace = Some("space".into());
        assert_eq!(
            tools.resolve_qualified_name(Some("space"), "read"),
            ToolNameResolution::Registered
        );
        tools.validate(&qualified_call).unwrap();
        tools
            .authorize_and_execute(qualified_call.clone())
            .await
            .unwrap();
        tools.result_committed(qualified_call).await.unwrap();
        tools.authorize_and_execute(call("read")).await.unwrap();
        assert_eq!(primary_calls.load(Ordering::SeqCst), 1);
        assert_eq!(fallback_calls.load(Ordering::SeqCst), 1);
        let mut foreign = call("read");
        foreign.namespace = Some("foreign".into());
        assert!(matches!(
            tools.authorize_and_execute(foreign).await,
            Err(RunError::ToolNotFound)
        ));
    }
}
