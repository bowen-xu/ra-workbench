//! Native structured text edits for an approved writing execution copy.
//!
//! This keeps file mutation separate from shell execution. Paths are validated
//! as workspace-relative names, the expected text is checked before each batch,
//! and the resulting workspace inventory is returned for the existing diff UI.
use super::*;
use crate::agent::{
    repository::execution_workspaces,
    workspace::{TextPatchOperation, WorkspaceLease},
    writing_submission::{WritingSubmissionReceipt, WritingSubmissionService},
};
use research_agent_core::{
    model::{ToolCall, ToolDefinition},
    runner::{RunError, RunFuture, ToolExecutor, ToolOutput},
    workspace::FileChange,
};
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

pub(super) const APPLY_PATCH: &str = "research_apply_patch";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Operation {
    path: String,
    before: Option<String>,
    after: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    operations: Vec<Operation>,
}

pub(super) struct PatchTools {
    store: AppStore,
    receipt: WritingSubmissionReceipt,
    context_snapshot_id: Uuid,
    allowed_paths: Option<std::collections::HashSet<String>>,
    scope: Option<crate::agent::context_session::sources::Resource>,
}

impl PatchTools {
    pub(super) async fn for_submission(
        store: AppStore,
        record: &SubmissionRecord,
    ) -> Result<Option<Self>, WritingSubmissionError> {
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
        let lookup = store.clone();
        let conversation_id = record.conversation_id;
        let request_id = record.request_id;
        let turn_id = record.turn_id;
        let context_snapshot_id = record.context_snapshot_id;
        let approved =
            tokio::task::spawn_blocking(move || -> Result<bool, WritingSubmissionError> {
                let db = lookup
                    .connection()
                    .map_err(|_| WritingSubmissionError::Unavailable)?;
                let Some((binding, manifest)) =
                    execution_workspaces::read(&db, conversation_id, request_id)
                        .map_err(|_| WritingSubmissionError::Unavailable)?
                else {
                    return Ok(false);
                };
                if binding.turn_id != turn_id
                    || binding.context_snapshot_id != context_snapshot_id
                    || binding.manuscript_id != manuscript_id
                {
                    return Err(WritingSubmissionError::Input(SubmissionError::Conflict));
                }
                Ok(execution_workspaces::authorize(&db, &binding, &manifest).is_ok())
            })
            .await
            .map_err(|_| WritingSubmissionError::Unavailable)??;
        Ok(approved.then_some(Self {
            store,
            receipt: WritingSubmissionReceipt {
                conversation_id: record.conversation_id,
                request_id: record.request_id,
                manuscript_id,
            },
            context_snapshot_id: record.context_snapshot_id,
            allowed_paths: crate::agent::context_session::permissions::writing_paths(record),
            scope: crate::agent::context_session::permissions::writing_scope(record),
        }))
    }

    pub(super) fn definitions(&self) -> Vec<ToolDefinition> {
        vec![ToolDefinition {
            namespace: None,
            name: APPLY_PATCH.into(),
            description: "Apply a structured batch of text edits to the approved isolated writing copy. Use workspace-relative paths only; before must match the complete current file or one unique context fragment. Set after to null to delete a file, or before to null to create one. After a successful turn, the application delivers these changes to the manuscript through version checks and presents pending diff for review. Keep confirms the delivered text; it does not initiate delivery. Do not claim delivery or saving succeeded from this tool result alone.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "operations": {"type": "array", "minItems": 1, "maxItems": 128, "items": {
                        "type": "object",
                        "properties": {
                            "path": {"type": "string", "minLength": 1, "maxLength": 1024},
                            "before": {"type": ["string", "null"], "maxLength": 16777216},
                            "after": {"type": ["string", "null"], "maxLength": 16777216}
                        },
                        "required": ["path", "before", "after"],
                        "additionalProperties": false
                    }}
                },
                "required": ["operations"],
                "additionalProperties": false
            }),
        }]
    }

    fn parse(call: &ToolCall) -> Result<Request, RunError> {
        if call.call_id.is_empty() || call.call_id.len() > 512 || call.name != APPLY_PATCH {
            return Err(RunError::ToolRejected);
        }
        let request: Request = crate::agent::tool_arguments::decode_object(
            &call.arguments,
            "Provide operations using the declared patch schema.",
        )?;
        if request.operations.is_empty() || request.operations.len() > 128 {
            return Err(RunError::ToolRejected);
        }
        for operation in &request.operations {
            if operation.path.to_ascii_lowercase().ends_with(".bib") {
                return Err(RunError::ToolRejected);
            }
            research_agent_core::workspace::validate_path(&operation.path)
                .map_err(|_| RunError::ToolRejected)?;
            if operation
                .before
                .as_ref()
                .is_some_and(|text| text.len() > 16 * 1024 * 1024)
                || operation
                    .after
                    .as_ref()
                    .is_some_and(|text| text.len() > 16 * 1024 * 1024)
            {
                return Err(RunError::ToolRejected);
            }
        }
        Ok(request)
    }

    async fn apply(&self, request: Request) -> Result<ToolOutput, RunError> {
        self.check_paths(&request)?;
        let prepared = match WritingSubmissionService::from_app_store(&self.store)
            .restore_workspace(self.receipt, self.context_snapshot_id)
            .await
        {
            Ok(prepared) => prepared,
            Err(error) => {
                return Ok(rejected_output(
                    "workspace_unavailable",
                    format!(
                        "无法打开当前执行副本：{error}。请先停止正在运行的命令，然后重新读取文件再修改。"
                    ),
                ));
            }
        };
        let store = self.store.clone();
        let changes = tokio::task::spawn_blocking(move || {
            let db = store.connection().map_err(|_| RunError::EventSink)?;
            if let Err(error) = execution_workspaces::authorize(
                &db,
                &prepared.binding,
                prepared.lease.manifest(),
            ) {
                return Ok::<_, RunError>(Err(rejected_output(
                    "workspace_unauthorized",
                    format!("执行副本授权已失效：{error}。请重新准备可编辑副本。"),
                )));
            }
            let mut lease: WorkspaceLease = prepared.lease;
            let operations = request
                .operations
                .into_iter()
                .map(|operation| TextPatchOperation {
                    path: operation.path,
                    before: operation.before,
                    after: operation.after,
                })
                .collect::<Vec<_>>();
            match lease.apply_text_operations(&operations) {
                Ok(changes) => Ok(Ok(changes)),
                Err(error) => Ok(Err(rejected_output(
                    "patch_conflict",
                    format!("修改未应用：{error}。请重新读取目标文件，使用读取到的完整 before 文本或唯一上下文片段后再提交补丁。"),
                ))),
            }
        })
        .await
        .map_err(|_| RunError::EventSink)??;
        let changes = match changes {
            Ok(changes) => changes,
            Err(output) => return Ok(output),
        };
        Ok(ToolOutput {
            content: json!({"status":"applied", "changes": changes.iter().map(render_change).collect::<Vec<_>>()}).to_string(),
            is_error: false,
        })
    }

    fn check_paths(&self, request: &Request) -> Result<(), RunError> {
        if self.scope.as_ref().map_or_else(
            || {
                self.allowed_paths.as_ref().is_some_and(|allowed| {
                    request
                        .operations
                        .iter()
                        .any(|operation| !allowed.contains(&operation.path))
                })
            },
            |scope| {
                request.operations.iter().any(|operation| {
                    !crate::agent::context_session::group_scope::path_access(
                        scope,
                        &operation.path,
                        true,
                        self.store.writing_resource_source(),
                    )
                })
            },
        ) {
            return Err(RunError::ToolRejected);
        }
        Ok(())
    }
}

impl ToolExecutor for PatchTools {
    fn validate(&self, call: &ToolCall) -> Result<(), RunError> {
        self.check_paths(&Self::parse(call)?)
    }
    fn authorize_and_execute(&self, call: ToolCall) -> RunFuture<'_, ToolOutput> {
        Box::pin(async move { self.apply(Self::parse(&call)?).await })
    }
    fn result_committed(&self, call: ToolCall) -> RunFuture<'_, ()> {
        Box::pin(async move { Self::parse(&call).map(|_| ()) })
    }
}

fn render_change(change: &FileChange) -> serde_json::Value {
    json!({"path": change.path, "kind": change.kind})
}

fn rejected_output(code: &str, message: String) -> ToolOutput {
    ToolOutput {
        content: json!({"error": code, "message": message}).to_string(),
        is_error: true,
    }
}

#[cfg(test)]
#[path = "patch/permission_tests.rs"]
mod permission_tests;
