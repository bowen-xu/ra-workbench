//! Model-facing bounded child tool for page-note and reading-note conversations.

use super::{
    note_child_commands::{self, ReadingRequest, Request},
    note_submission::source::Resource,
};
use research_agent_core::{
    model::{ToolCall, ToolDefinition},
    runner::{RunError, RunFuture, ToolExecutor, ToolOutput},
};
use serde::Deserialize;
use serde_json::json;
use std::{collections::HashSet, sync::Arc};
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
    pub resource: Resource,
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
        description: "Delegate a bounded task to a separate child conversation for the current frozen note resource. The child cannot expand the parent scope or budget.".into(),
        parameters: json!({"type":"object","properties":{"prompt":{"type":"string","minLength":1,"maxLength":16384}},"required":["prompt"],"additionalProperties":false}),
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
        let kind = match self.context.resource {
            Resource::Page(_) => research_agent_core::conversation::BindingKind::Note,
            Resource::Reading(_) => research_agent_core::conversation::BindingKind::ReadingNote,
        };
        Some(super::child_creation::intent(
            super::child_creation::note_tool_child_id(self.context.parent_turn_id, &call.call_id),
            research_agent_core::conversation::ConversationBinding {
                kind,
                resource_id: self.context.resource.id().to_string(),
            },
            super::child_creation::Host::Note,
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
            let child_id = super::child_creation::note_tool_child_id(
                self.context.parent_turn_id,
                &call.call_id,
            );
            let result = match self.context.resource {
                Resource::Page(page_id) => {
                    note_child_commands::spawn_note_child_with_id(
                        self.context.store.clone(),
                        self.context.catalog.clone(),
                        self.context.models.clone(),
                        self.context.sessions.clone(),
                        Request {
                            parent_conversation_id: self.context.parent_conversation_id,
                            parent_turn_id: self.context.parent_turn_id,
                            page_id,
                            prompt: args.prompt,
                            approved_servers: self.context.approved_servers.clone(),
                        },
                        child_id,
                    )
                    .await
                }
                Resource::Reading(reading_note_id) => {
                    note_child_commands::spawn_reading_note_child_with_id(
                        self.context.store.clone(),
                        self.context.catalog.clone(),
                        self.context.models.clone(),
                        self.context.sessions.clone(),
                        ReadingRequest {
                            parent_conversation_id: self.context.parent_conversation_id,
                            parent_turn_id: self.context.parent_turn_id,
                            reading_note_id,
                            prompt: args.prompt,
                            approved_servers: self.context.approved_servers.clone(),
                        },
                        child_id,
                    )
                    .await
                }
            };
            match result {
                Ok(response) => Ok(ToolOutput {
                    content: serde_json::to_string(&response)
                        .map_err(|_| RunError::ToolUnavailable)?,
                    is_error: false,
                }),
                Err(error) => {
                    if let note_child_commands::Error::ArchiveMutation(failure) = &error {
                        return Err(super::child_archive::unresolved_tool_error(
                            failure, child_id,
                        ));
                    }
                    if let note_child_commands::Error::ChildSubmissionUnconfirmed(failure) = &error
                    {
                        return Err(super::child_submission::tool_error(
                            failure,
                            child_id,
                            self.context.parent_conversation_id,
                            self.context.parent_turn_id,
                            self.context.context_snapshot_id,
                        ));
                    }
                    if let note_child_commands::Error::ChildTrackingUnconfirmed(failure) = &error {
                        return Err(failure.tool_error(
                            child_id,
                            self.context.parent_conversation_id,
                            self.context.parent_turn_id,
                            self.context.context_snapshot_id,
                        ));
                    }
                    if let note_child_commands::Error::CatalogMutation(failure) = &error {
                        let kind = match self.context.resource {
                            Resource::Page(_) => {
                                research_agent_core::conversation::BindingKind::Note
                            }
                            Resource::Reading(_) => {
                                research_agent_core::conversation::BindingKind::ReadingNote
                            }
                        };
                        let expected = super::child_creation::intent(
                            child_id,
                            research_agent_core::conversation::ConversationBinding {
                                kind,
                                resource_id: self.context.resource.id().to_string(),
                            },
                            super::child_creation::Host::Note,
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

/// Host errors contain only closed codes, recovery identity and safe diagnostics.
pub(super) fn error_output(error: &note_child_commands::Error) -> Result<ToolOutput, RunError> {
    Ok(ToolOutput {
        content: serde_json::to_string(error).map_err(|_| RunError::ToolUnavailable)?,
        is_error: true,
    })
}
