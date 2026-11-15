//! Model-facing bounded child tool for paper reading.
//! The tool only admits a prompt; native command code derives the parent's
//! exact paper scope and performs all identity and budget checks.

use super::reading_child_commands::{self, Request};
use research_agent_core::{
    model::{ToolCall, ToolDefinition},
    runner::{RunError, RunFuture, ToolExecutor, ToolOutput},
};
use serde::Deserialize;
use serde_json::json;
use std::collections::HashSet;
use std::sync::Arc;
use uuid::Uuid;

pub(crate) const SPAWN_SUBTASK: &str = "research_spawn_subtask";

#[derive(Clone)]
pub(crate) struct Context {
    pub store: crate::storage::AppStore,
    pub catalog: crate::agent::repository::SqliteConversationCatalog,
    pub models: crate::agent::model::AgentModelResolver,
    pub sessions: Arc<crate::agent::service::AgentSessionService>,
    pub parent_conversation_id: Uuid,
    pub parent_turn_id: Uuid,
    pub context_snapshot_id: Uuid,
    pub paper_id: String,
    pub paper_context_snapshot_id: String,
    pub fallback: Arc<dyn ToolExecutor>,
    pub approved_servers: HashSet<String>,
}

pub(crate) struct Tool {
    context: Context,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Args {
    prompt: String,
}

pub(crate) fn definition() -> ToolDefinition {
    ToolDefinition {
        namespace: None,
        name: SPAWN_SUBTASK.into(),
        description: "Delegate a bounded, paper-scoped research task to a separate child conversation. The child inherits the exact frozen paper snapshot and cannot expand the parent budget or sources.".into(),
        parameters: json!({
            "type": "object",
            "properties": {"prompt": {"type": "string", "minLength": 1, "maxLength": 16384}},
            "required": ["prompt"],
            "additionalProperties": false
        }),
    }
}

impl Tool {
    pub(crate) fn new(context: Context) -> Self {
        Self { context }
    }

    fn args(&self, call: &ToolCall) -> Result<Args, RunError> {
        if call.name != SPAWN_SUBTASK || call.arguments.to_string().len() > 32 * 1024 {
            return Err(RunError::ToolRejected);
        }
        let args: Args = crate::agent::tool_arguments::decode_object(
            &call.arguments,
            "Provide the required prompt string; no extra fields are accepted.",
        )?;
        let request = research_agent_core::subtasks::SpawnRequest {
            prompt: args.prompt.clone(),
            context_snapshot_id: self.context.context_snapshot_id,
            model_calls: 1,
            tool_calls: 1,
            approved_servers: self.context.approved_servers.clone(),
        };
        request.validate().map_err(|_| RunError::ToolRejected)?;
        Ok(args)
    }
}

impl ToolExecutor for Tool {
    fn child_creation_intent(
        &self,
        call: &ToolCall,
    ) -> Option<research_agent_core::conversation::CreateBoundConversation> {
        if call.name != SPAWN_SUBTASK {
            return self.context.fallback.child_creation_intent(call);
        }
        Some(super::child_creation::intent(
            super::child_creation::paper_tool_child_id(self.context.parent_turn_id, &call.call_id),
            research_agent_core::conversation::ConversationBinding {
                kind: research_agent_core::conversation::BindingKind::Paper,
                resource_id: self.context.paper_id.clone(),
            },
            super::child_creation::Host::Paper,
        ))
    }
    fn validate(&self, call: &ToolCall) -> Result<(), RunError> {
        if call.name == SPAWN_SUBTASK {
            self.args(call).map(|_| ())
        } else {
            self.context.fallback.validate(call)
        }
    }

    fn authorize_and_execute(&self, call: ToolCall) -> RunFuture<'_, ToolOutput> {
        Box::pin(async move {
            if call.name != SPAWN_SUBTASK {
                return self.context.fallback.authorize_and_execute(call).await;
            }
            let args = self.args(&call)?;
            let child_id = super::child_creation::paper_tool_child_id(
                self.context.parent_turn_id,
                &call.call_id,
            );
            let response = reading_child_commands::spawn_paper_child_with_id(
                self.context.store.clone(),
                self.context.catalog.clone(),
                self.context.models.clone(),
                self.context.sessions.clone(),
                Request {
                    parent_conversation_id: self.context.parent_conversation_id,
                    parent_turn_id: self.context.parent_turn_id,
                    paper_id: self.context.paper_id.clone(),
                    paper_context_snapshot_id: self.context.paper_context_snapshot_id.clone(),
                    expected_context_hash: None,
                    prompt: args.prompt,
                    approved_servers: self.context.approved_servers.clone(),
                },
                child_id,
            )
            .await;
            match response {
                Ok(response) => Ok(ToolOutput {
                    content: serde_json::to_string(&response)
                        .map_err(|_| RunError::ToolUnavailable)?,
                    is_error: false,
                }),
                Err(error) => {
                    if let reading_child_commands::Error::ChildPreparationUnconfirmed {
                        child_id: actual_child,
                        request_id,
                        reason,
                    } = &error
                    {
                        return Err(preparation_error(
                            *actual_child,
                            *request_id,
                            reason,
                            child_id,
                            (
                                self.context.parent_conversation_id,
                                self.context.parent_turn_id,
                                self.context.context_snapshot_id,
                            ),
                        ));
                    }
                    if let reading_child_commands::Error::ArchiveMutation(failure) = &error {
                        return Err(super::child_archive::unresolved_tool_error(
                            failure, child_id,
                        ));
                    }
                    if let reading_child_commands::Error::ChildTrackingUnconfirmed(failure) = &error
                    {
                        return Err(failure.tool_error(
                            child_id,
                            self.context.parent_conversation_id,
                            self.context.parent_turn_id,
                            self.context.context_snapshot_id,
                        ));
                    }
                    if let reading_child_commands::Error::CatalogMutation(failure) = &error {
                        let expected = super::child_creation::intent(
                            child_id,
                            research_agent_core::conversation::ConversationBinding {
                                kind: research_agent_core::conversation::BindingKind::Paper,
                                resource_id: self.context.paper_id.clone(),
                            },
                            super::child_creation::Host::Paper,
                        );
                        if let Some(error) = super::child_creation::unknown_tool_error(
                            failure,
                            &expected,
                            self.context.parent_conversation_id,
                            self.context.parent_turn_id,
                            self.context.context_snapshot_id,
                        ) {
                            return Err(error);
                        }
                    }
                    error_output(&error)
                }
            }
        })
    }

    fn result_committed(&self, call: ToolCall) -> RunFuture<'_, ()> {
        if call.name == SPAWN_SUBTASK {
            Box::pin(async { Ok(()) })
        } else {
            self.context.fallback.result_committed(call)
        }
    }
}

fn preparation_error(
    actual_child: Uuid,
    request: Uuid,
    reason: &reading_child_commands::ChildPreparationReason,
    expected_child: Uuid,
    parent_scope: (Uuid, Uuid, Uuid),
) -> RunError {
    use super::reading_submission::Error;
    use reading_child_commands::ChildPreparationReason as Reason;
    use research_agent_core::runner::ToolOutcomeDiagnostic;
    if actual_child != expected_child
        || request != research_agent_core::conversation::child_submission_request_id(expected_child)
    {
        return RunError::ToolOutcomeUnknown;
    }
    if let Reason::Preparation(Error::OutcomeUnknown(detail)) = reason {
        if detail.conversation_id != expected_child || detail.request_id != request {
            return RunError::ToolOutcomeUnknown;
        }
        return ToolOutcomeDiagnostic::try_from_value(json!({
            "code": "DATA_OPERATION_OUTCOME_UNKNOWN",
            "phase": detail.phase,
            "safeCause": detail.safe_cause,
            "conversationId": parent_scope.0,
            "turnId": parent_scope.1,
            "contextSnapshotId": parent_scope.2,
            "resultId": actual_child,
            "requestId": request,
            "sqliteCode": detail.sqlite_code,
            "sqliteExtendedCode": detail.sqlite_extended_code,
        }))
        .map(|detail| RunError::ToolOutcomeUnknownDiagnostic(Box::new(detail)))
        .unwrap_or(RunError::ToolOutcomeUnknown);
    }
    if let Reason::Preparation(Error::Storage(detail)) = reason {
        if detail.operation_id != request {
            return RunError::ToolOutcomeUnknown;
        }
        return ToolOutcomeDiagnostic::try_from_value(json!({
            "code": detail.code, "phase": detail.phase, "safeCause": detail.safe_cause,
            "conversationId": parent_scope.0, "turnId": parent_scope.1,
            "contextSnapshotId": parent_scope.2, "resultId": actual_child, "requestId": request,
            "journalRequestId": request, "sqliteCode": detail.sqlite_code,
            "sqliteExtendedCode": detail.sqlite_extended_code,
        }))
        .map(|detail| RunError::ToolOutcomeUnknownDiagnostic(Box::new(detail)))
        .unwrap_or(RunError::ToolOutcomeUnknown);
    }
    let cause = match reason {
        Reason::CreationReplayed => "paper_child_creation_replayed",
        Reason::Preparation(Error::InvalidInput) => "paper_preparation_invalid_input",
        Reason::Preparation(Error::Unbound) => "paper_preparation_unbound",
        Reason::Preparation(Error::Conflict) => "paper_preparation_conflict",
        Reason::Preparation(Error::ModelUnavailable) => "paper_preparation_model_unavailable",
        Reason::Preparation(Error::Budget) => "paper_preparation_budget",
        Reason::Preparation(Error::Unavailable) => "paper_preparation_unavailable",
        Reason::Preparation(Error::OutcomeUnknown(_)) => unreachable!(),
        Reason::Preparation(Error::Storage(_)) => unreachable!(),
        Reason::Preparation(Error::Source(_)) => return RunError::ToolOutcomeUnknown,
    };
    ToolOutcomeDiagnostic::try_from_value(json!({
        "code": "CHILD_SUBMISSION_UNCONFIRMED",
        "phase": "paper-child-preparation",
        "safeCause": cause,
        "conversationId": parent_scope.0,
        "turnId": parent_scope.1,
        "contextSnapshotId": parent_scope.2,
        "resultId": actual_child,
        "requestId": request,
    }))
    .map(|detail| RunError::ToolOutcomeUnknownDiagnostic(Box::new(detail)))
    .unwrap_or(RunError::ToolOutcomeUnknown)
}

/// Host errors contain only closed codes, recovery identity and safe diagnostics.
pub(super) fn error_output(error: &reading_child_commands::Error) -> Result<ToolOutput, RunError> {
    Ok(ToolOutput {
        content: serde_json::to_string(error).map_err(|_| RunError::ToolUnavailable)?,
        is_error: true,
    })
}

#[cfg(test)]
mod preparation_tests {
    use super::*;

    #[test]
    fn preparation_diagnostic_preserves_original_identity() {
        let child = Uuid::new_v4();
        let request = research_agent_core::conversation::child_submission_request_id(child);
        let error = preparation_error(
            child,
            request,
            &reading_child_commands::ChildPreparationReason::Preparation(
                super::super::reading_submission::Error::Unavailable,
            ),
            child,
            (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4()),
        );
        let RunError::ToolOutcomeUnknownDiagnostic(detail) = error else {
            panic!("paper preparation must stop the parent runner");
        };
        let diagnostic = serde_json::to_value(detail).unwrap();
        assert_eq!(diagnostic["resultId"], child.to_string());
        assert_eq!(diagnostic["requestId"], request.to_string());
        assert_eq!(diagnostic["safeCause"], "paper_preparation_unavailable");
        let unknown = preparation_error(
            child,
            request,
            &reading_child_commands::ChildPreparationReason::Preparation(
                super::super::reading_submission::Error::OutcomeUnknown(
                    super::super::reading_submission::transaction::Unknown {
                        conversation_id: child,
                        request_id: request,
                        phase: "commit",
                        safe_cause: "sql_commit_outcome_unknown",
                        sqlite_code: Some("ConstraintViolation".into()),
                        sqlite_extended_code: Some(787),
                    },
                ),
            ),
            child,
            (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4()),
        );
        let RunError::ToolOutcomeUnknownDiagnostic(detail) = unknown else {
            panic!("expected storage diagnostic");
        };
        let diagnostic = serde_json::to_value(detail).unwrap();
        assert_eq!(diagnostic["safeCause"], "sql_commit_outcome_unknown");
        assert_eq!(diagnostic["sqliteExtendedCode"], 787);
        let replay = preparation_error(
            child,
            request,
            &reading_child_commands::ChildPreparationReason::CreationReplayed,
            child,
            (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4()),
        );
        let RunError::ToolOutcomeUnknownDiagnostic(detail) = replay else {
            panic!("replayed creation must require reconciliation");
        };
        assert_eq!(
            serde_json::to_value(detail).unwrap()["safeCause"],
            "paper_child_creation_replayed"
        );
        assert!(matches!(
            preparation_error(
                child,
                Uuid::new_v4(),
                &reading_child_commands::ChildPreparationReason::Preparation(
                    super::super::reading_submission::Error::Unavailable
                ),
                child,
                (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4())
            ),
            RunError::ToolOutcomeUnknown
        ));
    }
}
