//! Read-only inspection of the approved writing execution copy.
//!
//! This is deliberately separate from the file/shell tools: it never grants a
//! path, never reads arbitrary content, and never publishes a change to the
//! live manuscript. The workspace review service revalidates the durable
//! binding and refuses to inspect a copy while a process still owns it.
use super::*;
use crate::agent::{
    repository::execution_workspaces,
    workspace::{ChangeContent, ReviewContent},
    writing_submission::{WritingSubmissionReceipt, WritingSubmissionService},
};
use research_agent_core::{
    model::{ToolCall, ToolDefinition},
    runner::{RunError, RunFuture, ToolExecutor, ToolOutput},
    workspace::{FileChange, FileVersion},
};
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

pub(super) const REVIEW_CHANGES: &str = "research_review_workspace_changes";
pub(super) const READ_CHANGE: &str = "research_read_workspace_change";

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReadRequest {
    path: String,
    before: Option<FileVersion>,
    after: Option<FileVersion>,
}

pub(super) struct WorkspaceTools {
    store: AppStore,
    receipt: WritingSubmissionReceipt,
    context_snapshot_id: Uuid,
    read_enabled: bool,
    selected_paths: Option<HashSet<String>>,
    scope: Option<crate::agent::context_session::sources::Resource>,
    #[cfg(target_os = "macos")]
    shell_host: Option<Arc<super::shell::Host>>,
}

impl WorkspaceTools {
    pub(super) fn finalization_call(&self) -> RunFuture<'_, Option<ToolCall>> {
        #[cfg(target_os = "macos")]
        if let Some(host) = &self.shell_host {
            return Box::pin(async move {
                if !host.requires_finalization() || !host.has_changes_for_finalization().await? {
                    return Ok(None);
                }
                Ok(Some(ToolCall {
                    namespace: None,
                    raw_arguments_json: None,
                    call_id: Uuid::new_v4().to_string(),
                    name: crate::agent::writing_delivery_tool::NAME.into(),
                    arguments: json!({}),
                }))
            });
        }
        Box::pin(async { Ok(None) })
    }
    pub(super) fn release_for_finalization(&self) -> RunFuture<'_, bool> {
        #[cfg(target_os = "macos")]
        if let Some(host) = &self.shell_host {
            return Box::pin(async move { host.release_for_finalization().await });
        }
        Box::pin(async { Ok(false) })
    }
    pub(super) fn hook_workspace(
        &self,
    ) -> crate::agent::compaction_hook_factory::HookWorkspaceHandle {
        #[cfg(target_os = "macos")]
        return self.shell_host.clone().map(|host| {
            host as Arc<dyn crate::agent::compaction_hook_config::native::HookWorkspaceSource>
        });
        #[cfg(not(target_os = "macos"))]
        None
    }
    #[cfg(target_os = "macos")]
    pub(super) fn file_workspace(
        &self,
    ) -> crate::agent::text_tools::file_workspace::FileWorkspaceHandle {
        self.shell_host.clone().map(|host| {
            host as Arc<dyn crate::agent::text_tools::file_workspace::FileWorkspaceSource>
        })
    }
    pub(super) fn new(
        store: AppStore,
        record: &SubmissionRecord,
    ) -> Result<Self, WritingSubmissionError> {
        let manuscript_id = record
            .input
            .context
            .iter()
            .find_map(|source| {
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
            })
            .ok_or(SubmissionError::Corrupt)?;
        let read_enabled = store
            .connection()
            .ok()
            .and_then(|db| {
                let saved =
                    execution_workspaces::read(&db, record.conversation_id, record.request_id)
                        .ok()
                        .flatten()?;
                let (binding, manifest) = saved;
                Some(
                    binding.turn_id == record.turn_id
                        && binding.context_snapshot_id == record.context_snapshot_id
                        && binding.manuscript_id == manuscript_id
                        && execution_workspaces::authorize(&db, &binding, &manifest).is_ok(),
                )
            })
            .unwrap_or(false);
        Ok(Self {
            store,
            receipt: WritingSubmissionReceipt {
                conversation_id: record.conversation_id,
                request_id: record.request_id,
                manuscript_id,
            },
            context_snapshot_id: record.context_snapshot_id,
            read_enabled,
            scope: crate::agent::context_session::permissions::writing_scope(record),
            selected_paths: crate::agent::context_session::permissions::selected_writing_paths(
                record,
            ),
            #[cfg(target_os = "macos")]
            shell_host: None,
        })
    }

    #[cfg(target_os = "macos")]
    pub(super) fn with_shell_host(mut self, host: Arc<super::shell::Host>) -> Self {
        self.shell_host = Some(host);
        self
    }

    async fn current_review(&self) -> Result<super::super::workspace::WorkspaceReview, RunError> {
        #[cfg(target_os = "macos")]
        if let Some(host) = &self.shell_host
            && let Some(review) = host.review().await?
        {
            return Ok(review);
        }
        WritingSubmissionService::from_app_store(&self.store)
            .review_workspace(self.receipt, self.context_snapshot_id)
            .await
            .map_err(|_| RunError::ToolRejected)
    }

    pub(super) fn definitions(&self) -> Vec<ToolDefinition> {
        let mut definitions = vec![
            ToolDefinition {
                namespace: None,
                name: REVIEW_CHANGES.into(),
                description: "List changes in the approved writing copy after processes stop. Read-only; use before describing edits.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {},
                    "additionalProperties": false
                }),
            },
            ToolDefinition {
                namespace: None,
                name: READ_CHANGE.into(),
                description: "Read bounded content for one exact reviewed change. Binary or oversized sides return only a kind.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "path": {"type": "string", "minLength": 1, "maxLength": 4096},
                        "before": {"type": "object"},
                        "after": {"type": "object"}
                    },
                    "required": ["path"],
                    "additionalProperties": false
                }),
            },
        ];
        if !self.read_enabled {
            definitions.pop();
        }
        definitions
    }

    fn parse(call: &ToolCall) -> Result<(), RunError> {
        if call.call_id.is_empty() || call.call_id.len() > 512 || call.name != REVIEW_CHANGES {
            return Err(RunError::ToolRejected);
        }
        if call.arguments != json!({}) {
            return Err(RunError::ToolArguments(
                "Review changes takes an empty object and no other arguments.".into(),
            ));
        }
        Ok(())
    }

    fn parse_read(call: &ToolCall) -> Result<ReadRequest, RunError> {
        if call.call_id.is_empty() || call.call_id.len() > 512 || call.name != READ_CHANGE {
            return Err(RunError::ToolRejected);
        }
        let request: ReadRequest = crate::agent::tool_arguments::decode_object(
            &call.arguments,
            "Provide path and at least one of before or after using the declared change read schema.",
        )?;
        if request.path.len() > 4096 {
            return Err(RunError::ToolRejected);
        }
        research_agent_core::workspace::validate_path(&request.path)
            .map_err(|_| RunError::ToolRejected)?;
        if request.before.is_none() && request.after.is_none() {
            return Err(RunError::ToolArguments(
                "Provide at least one of before or after.".into(),
            ));
        }
        Ok(request)
    }

    async fn review(&self) -> Result<ToolOutput, RunError> {
        let result = self.current_review().await;
        match result {
            Ok(review) => Ok(ToolOutput {
                content: json!({
                    "status": "ready",
                    "changes": review.changes.iter().map(render_change).collect::<Vec<_>>(),
                })
                .to_string(),
                is_error: false,
            }),
            Err(_) => Ok(ToolOutput {
                content: json!({
                    "error": "workspace_review_unavailable",
                    "message": "The bound execution copy is missing, revoked, changed, or still busy. Do not infer that edits were published.",
                })
                .to_string(),
                is_error: true,
            }),
        }
    }

    async fn read(&self, request: ReadRequest) -> Result<ToolOutput, RunError> {
        let native = match self
            .store
            .native_writing_context()
            .filter(|_| self.scope.is_some())
        {
            Some(binding) => Some(binding.scope_resources().await),
            None => None,
        };
        let source = native
            .as_ref()
            .map(|source| crate::agent::writing_resources::WritingScope::Native(source))
            .or_else(|| {
                self.store
                    .writing_resource_source()
                    .map(crate::agent::writing_resources::WritingScope::Static)
            });
        let allowed = self.scope.as_ref().map_or_else(
            || {
                Ok(self
                    .selected_paths
                    .as_ref()
                    .is_none_or(|paths| paths.contains(&request.path)))
            },
            |scope| {
                crate::agent::context_session::group_scope::path_access_with_scope(
                    scope,
                    &request.path,
                    false,
                    source,
                )
            },
        );
        match allowed {
            Ok(true) => {}
            Ok(false) => return Err(RunError::ToolRejected),
            Err(error) => {
                return Ok(ToolOutput {
                    content: json!({
                        "error": "writing_scope_source_unavailable",
                        "message": error.to_string(),
                        "conversationId": self.receipt.conversation_id,
                        "requestId": self.receipt.request_id,
                        "manuscriptId": self.receipt.manuscript_id,
                        "contextSnapshotId": self.context_snapshot_id,
                    })
                    .to_string(),
                    is_error: true,
                });
            }
        }
        let service = WritingSubmissionService::from_app_store(&self.store);
        let review = self.current_review().await?;
        if !review.changes.iter().any(|change| {
            change.path == request.path
                && change.before == request.before
                && change.after == request.after
        }) {
            return Err(RunError::ToolRejected);
        }
        #[cfg(target_os = "macos")]
        if let Some(host) = &self.shell_host
            && let Some(expected) = review
                .changes
                .iter()
                .find(|change| change.path == request.path)
            && let Some(content) = host.read(expected.clone()).await?
        {
            return Ok(ToolOutput {
                content: render_content(&content).to_string(),
                is_error: false,
            });
        }
        let content = service
            .review_workspace_change(review.binding, request.path, request.before, request.after)
            .await
            .map_err(|_| RunError::ToolRejected)?;
        Ok(ToolOutput {
            content: render_content(&content).to_string(),
            is_error: false,
        })
    }
}

impl ToolExecutor for WorkspaceTools {
    fn validate(&self, call: &ToolCall) -> Result<(), RunError> {
        if call.name == READ_CHANGE {
            if !self.read_enabled {
                return Err(RunError::ToolRejected);
            }
            Self::parse_read(call).map(|_| ())
        } else {
            Self::parse(call)
        }
    }

    fn authorize_and_execute(&self, call: ToolCall) -> RunFuture<'_, ToolOutput> {
        Box::pin(async move {
            if call.name == READ_CHANGE {
                if !self.read_enabled {
                    return Err(RunError::ToolRejected);
                }
                return self.read(Self::parse_read(&call)?).await;
            }
            Self::parse(&call)?;
            self.review().await
        })
    }

    fn result_committed(&self, call: ToolCall) -> RunFuture<'_, ()> {
        Box::pin(async move {
            if call.name == READ_CHANGE {
                if !self.read_enabled {
                    return Err(RunError::ToolRejected);
                }
                Self::parse_read(&call).map(|_| ())
            } else {
                Self::parse(&call)
            }
        })
    }
}

fn render_content(content: &ChangeContent) -> serde_json::Value {
    fn side(value: Option<&ReviewContent>) -> serde_json::Value {
        match value {
            None => json!({"kind": "absent"}),
            Some(ReviewContent::Text(text)) => json!({"kind": "text", "text": text}),
            Some(ReviewContent::Binary) => json!({"kind": "binary"}),
            Some(ReviewContent::TooLarge) => json!({"kind": "too_large"}),
        }
    }
    json!({
        "status": "ready",
        "path": content.change.path,
        "kind": content.change.kind,
        "before": side(content.before.as_ref()),
        "after": side(content.after.as_ref()),
    })
}

fn render_change(change: &FileChange) -> serde_json::Value {
    json!({
        "path": change.path,
        "kind": change.kind,
        "before": change.before,
        "after": change.after,
        "hasEditorBuffer": change.source.as_ref().is_some_and(|source| source.editor.is_some()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use research_agent_core::workspace::{FileChangeKind, FileVersion};

    #[test]
    fn accepts_only_an_empty_review_request() {
        let call = ToolCall {
            namespace: None,
            raw_arguments_json: None,
            call_id: "review-1".into(),
            name: REVIEW_CHANGES.into(),
            arguments: json!({}),
        };
        assert!(WorkspaceTools::parse(&call).is_ok());
        let mut invalid = call.clone();
        invalid.arguments = json!({"path": "main.tex"});
        assert!(WorkspaceTools::parse(&invalid).is_err());
    }

    #[test]
    fn renders_versions_without_exposing_workspace_paths() {
        let change = FileChange {
            kind: FileChangeKind::Modified,
            path: "sections/body.tex".into(),
            before: Some(FileVersion {
                sha256: "a".repeat(64),
                byte_size: 4,
                executable: false,
            }),
            after: Some(FileVersion {
                sha256: "b".repeat(64),
                byte_size: 5,
                executable: false,
            }),
            source: None,
        };
        let value = render_change(&change);
        assert_eq!(value["path"], "sections/body.tex");
        assert_eq!(value["kind"], "modified");
        assert!(value.get("/private").is_none());
        assert_eq!(value["hasEditorBuffer"], false);
    }

    #[test]
    fn accepts_only_an_exact_versioned_read_request() {
        let version = json!({
            "sha256": "a".repeat(64),
            "byte_size": 4,
            "executable": false
        });
        let call = ToolCall {
            namespace: None,
            raw_arguments_json: None,
            call_id: "read-1".into(),
            name: READ_CHANGE.into(),
            arguments: json!({"path": "main.tex", "before": version}),
        };
        assert!(WorkspaceTools::parse_read(&call).is_ok());
        let mut invalid = call.clone();
        invalid.arguments = json!({"path": "../main.tex", "before": version});
        assert!(WorkspaceTools::parse_read(&invalid).is_err());
        invalid.arguments = json!({"path": "main.tex"});
        assert!(WorkspaceTools::parse_read(&invalid).is_err());
    }

    #[test]
    fn renders_bounded_content_kinds_without_paths() {
        let content = ChangeContent {
            change: FileChange {
                kind: FileChangeKind::Modified,
                path: "main.tex".into(),
                before: None,
                after: None,
                source: None,
            },
            before: Some(ReviewContent::Text("before".into())),
            after: Some(ReviewContent::Binary),
        };
        let value = render_content(&content);
        assert_eq!(value["before"]["kind"], "text");
        assert_eq!(value["after"]["kind"], "binary");
        assert!(value.to_string().contains("main.tex"));
        assert!(!value.to_string().contains("/private/"));
    }
}

#[cfg(test)]
#[path = "workspace/runtime_tests.rs"]
mod runtime_tests;
