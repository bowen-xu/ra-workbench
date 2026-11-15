//! Read-only Git context for the writing agent.
//!
//! The repository/process boundary is shared with the native Git adapter. This
//! module only binds that adapter to the current manuscript source directory;
//! it never accepts a path from the model.

use super::*;
use crate::agent::git::GitWorkspaceService;
use research_agent_core::{
    git::GitRepository,
    model::{ToolCall, ToolDefinition},
    runner::{RunError, RunFuture, ToolExecutor, ToolOutput},
};
use serde::Deserialize;
use serde_json::json;
use std::path::Path;
use uuid::Uuid;

pub(super) const STATUS: &str = "research_git_status";
pub(super) const LOG: &str = "research_git_log";
pub(super) const DIFF: &str = "research_git_diff";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LogRequest {
    limit: Option<usize>,
}

pub(super) struct GitTools {
    service: GitWorkspaceService,
    repository: GitRepository,
}

impl GitTools {
    pub(super) async fn for_submission(
        store: &AppStore,
        record: &SubmissionRecord,
    ) -> Option<Self> {
        let manuscript_id = record.input.context.iter().find_map(|source| {
            source
                .anchor
                .as_deref()
                .and_then(|anchor| serde_json::from_str::<serde_json::Value>(anchor).ok())
                .and_then(|anchor| {
                    anchor
                        .get("manuscriptId")?
                        .as_str()
                        .and_then(|id| Uuid::parse_str(id).ok())
                })
        })?;
        let source_directory = store.writing_source_directory()?;
        let source =
            source_directory(store.manuscript_directory(), &manuscript_id.to_string()).ok()?;
        if !source.is_dir() {
            return None;
        }
        let service = GitWorkspaceService::default();
        let repository = service.inspect(Path::new(&source)).await.ok()?;
        Some(Self {
            service,
            repository,
        })
    }

    pub(super) fn definitions(&self) -> Vec<ToolDefinition> {
        vec![
            ToolDefinition {
                namespace: None,
                name: STATUS.into(),
                description: "Read the current Git branch and bounded working-tree status for this manuscript. Read-only.".into(),
                parameters: json!({"type":"object","properties":{},"additionalProperties":false}),
            },
            ToolDefinition {
                namespace: None,
                name: LOG.into(),
                description: "Read recent Git commit subjects for this manuscript. Read-only.".into(),
                parameters: json!({
                    "type":"object",
                    "properties":{"limit":{"type":"integer","minimum":1,"maximum":50}},
                    "additionalProperties":false
                }),
            },
            ToolDefinition {
                namespace: None,
                name: DIFF.into(),
                description: "Read the bounded unstaged Git diff for this manuscript. Read-only.".into(),
                parameters: json!({"type":"object","properties":{},"additionalProperties":false}),
            },
        ]
    }

    fn empty(call: &ToolCall, name: &str) -> Result<(), RunError> {
        if call.call_id.is_empty() || call.call_id.len() > 512 || call.name != name {
            return Err(RunError::ToolRejected);
        }
        if call.arguments != json!({}) {
            return Err(RunError::ToolArguments(
                "This Git tool takes an empty object and no other arguments.".into(),
            ));
        }
        Ok(())
    }

    fn parse_log(call: &ToolCall) -> Result<usize, RunError> {
        if call.call_id.is_empty() || call.call_id.len() > 512 || call.name != LOG {
            return Err(RunError::ToolRejected);
        }
        let request: LogRequest = crate::agent::tool_arguments::decode_object(
            &call.arguments,
            "Provide only an optional integer limit between 1 and 50.",
        )?;
        let limit = request.limit.unwrap_or(20);
        if !(1..=50).contains(&limit) {
            return Err(RunError::ToolArguments(
                "limit must be between 1 and 50.".into(),
            ));
        }
        Ok(limit)
    }

    async fn status(&self) -> Result<ToolOutput, RunError> {
        let (branch, entries) = self
            .service
            .status(&self.repository)
            .await
            .map_err(|_| RunError::ToolRejected)?;
        Ok(ToolOutput {
            content: json!({"branch": branch, "entries": entries}).to_string(),
            is_error: false,
        })
    }

    async fn log(&self, limit: usize) -> Result<ToolOutput, RunError> {
        let commits = self
            .service
            .log(&self.repository, limit)
            .await
            .map_err(|_| RunError::ToolRejected)?;
        Ok(ToolOutput {
            content: json!({"commits": commits}).to_string(),
            is_error: false,
        })
    }

    async fn diff(&self) -> Result<ToolOutput, RunError> {
        let bytes = self
            .service
            .diff(&self.repository)
            .await
            .map_err(|_| RunError::ToolRejected)?;
        let content = match String::from_utf8(bytes) {
            Ok(text) => json!({"kind":"text","diff":text}),
            Err(error) => json!({"kind":"binary","byteSize":error.as_bytes().len()}),
        };
        Ok(ToolOutput {
            content: content.to_string(),
            is_error: false,
        })
    }
}

impl ToolExecutor for GitTools {
    fn validate(&self, call: &ToolCall) -> Result<(), RunError> {
        match call.name.as_str() {
            STATUS | DIFF => Self::empty(call, &call.name),
            LOG => Self::parse_log(call).map(|_| ()),
            _ => Err(RunError::ToolRejected),
        }
    }

    fn authorize_and_execute(&self, call: ToolCall) -> RunFuture<'_, ToolOutput> {
        Box::pin(async move {
            match call.name.as_str() {
                STATUS => {
                    Self::empty(&call, STATUS)?;
                    self.status().await
                }
                LOG => self.log(Self::parse_log(&call)?).await,
                DIFF => {
                    Self::empty(&call, DIFF)?;
                    self.diff().await
                }
                _ => Err(RunError::ToolRejected),
            }
        })
    }

    fn result_committed(&self, call: ToolCall) -> RunFuture<'_, ()> {
        Box::pin(async move { self.validate(&call) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use research_agent_core::git::GitRepository;
    use std::collections::BTreeMap;

    fn tools() -> GitTools {
        GitTools {
            service: GitWorkspaceService::default(),
            repository: GitRepository {
                root: "/tmp/research-agent-test".into(),
                git_dir: "/tmp/research-agent-test/.git".into(),
                worktree: None,
                head: None,
                branch: None,
                remotes: BTreeMap::new(),
            },
        }
    }

    #[test]
    fn definitions_are_strict_and_validate_arguments() {
        let tools = tools();
        assert_eq!(tools.definitions().len(), 3);
        let valid = ToolCall {
            namespace: None,
            raw_arguments_json: None,
            call_id: "call-1".into(),
            name: STATUS.into(),
            arguments: json!({}),
        };
        assert!(tools.validate(&valid).is_ok());
        let invalid = ToolCall {
            namespace: None,
            raw_arguments_json: None,
            call_id: "call-1".into(),
            name: STATUS.into(),
            arguments: json!({"path":"/tmp"}),
        };
        assert!(tools.validate(&invalid).is_err());
        let log = ToolCall {
            namespace: None,
            raw_arguments_json: None,
            call_id: "call-2".into(),
            name: LOG.into(),
            arguments: json!({"limit": 20}),
        };
        assert!(tools.validate(&log).is_ok());
        let oversized = ToolCall {
            namespace: None,
            raw_arguments_json: None,
            call_id: "call-3".into(),
            name: LOG.into(),
            arguments: json!({"limit": 51}),
        };
        assert!(tools.validate(&oversized).is_err());
    }

    #[tokio::test]
    async fn submission_uses_the_real_source_directory_for_status_without_new_active_checks() {
        for native in [false, true] {
            let mut f = super::super::super::tests::fixture(true);
            let record = WritingSubmissionService::from_app_store(&f.store)
                .prepare(
                    &f.models,
                    super::super::super::tests::request(&f, Uuid::new_v4()),
                )
                .await
                .unwrap()
                .record;
            if native {
                use crate::runtime_plugins::{
                    RuntimePluginCatalog, native::NativePreviewService,
                    writing_projects::context::Binding,
                };
                let catalog = std::sync::Arc::new(RuntimePluginCatalog::new(None));
                let binding = Binding {
                    service: std::sync::Arc::new(NativePreviewService::new(std::sync::Arc::clone(
                        &catalog,
                    ))),
                    catalog,
                    package_key: "a".repeat(64),
                };
                f.store = f
                    .store
                    .without_static_writing_context_sources()
                    .with_native_writing_context(binding);
                assert!(f.store.writing_resource_source().is_none());
            }
            let source = f.store.writing_source_directory().unwrap()(
                f.store.manuscript_directory(),
                &f.manuscript.to_string(),
            )
            .unwrap();
            assert!(source.is_dir());
            assert!(GitTools::for_submission(&f.store, &record).await.is_none());
            GitWorkspaceService::default()
                .initialize(&source)
                .await
                .unwrap();
            f.store
                .connection()
                .unwrap()
                .execute(
                    "update manuscripts set status='archived' where id=?1",
                    [f.manuscript.to_string()],
                )
                .unwrap();
            f.store
                .connection()
                .unwrap()
                .execute(
                    "delete from agent_conversation_bindings where conversation_id=?1",
                    [f.conversation.to_string()],
                )
                .unwrap();
            let tools = GitTools::for_submission(&f.store, &record).await.unwrap();
            let output = tools
                .authorize_and_execute(ToolCall {
                    namespace: None,
                    raw_arguments_json: None,
                    call_id: "source-status".into(),
                    name: STATUS.into(),
                    arguments: json!({}),
                })
                .await
                .unwrap();
            assert!(!output.is_error);
            let value: serde_json::Value = serde_json::from_str(&output.content).unwrap();
            assert_eq!(value["branch"], "main");
            assert!(value["entries"].is_array());
            let retained = source.with_extension("retained");
            std::fs::rename(&source, &retained).unwrap();
            assert!(GitTools::for_submission(&f.store, &record).await.is_none());
            std::fs::rename(&retained, &source).unwrap();
        }
    }
}
