//! Connect prepared writing input to the real native model transport and runner.
use super::*;
use crate::agent::{
    execution::AgentExecution,
    repository::SqliteSubmissionStore,
    service::{AgentSessionService, SubmissionAdmission, SubmissionRunError},
};
use research_agent_core::{
    conversation::ConversationId, model::ModelTransport, protocol::ModelIdentity,
};
use std::sync::Arc;

pub use crate::agent::execution_profile::AgentExecutionProfile as WritingExecutionProfile;
/// Names the exact saved input the host confirmed for model disclosure.
/// Constructing this native value alone is not a user-facing consent workflow.
pub struct WritingExecutionConfirmation {
    pub receipt: WritingSubmissionReceipt,
    pub context_snapshot_id: Uuid,
    pub model: ModelIdentity,
}
#[derive(Debug, thiserror::Error)]
pub enum WritingStartError {
    #[error("writing execution confirmation does not match the saved input")]
    Mismatch,
    #[error(transparent)]
    Preparation(#[from] WritingSubmissionError),
    #[error(transparent)]
    Model(#[from] AgentModelPreparationError),
    #[error(transparent)]
    Run(#[from] SubmissionRunError),
}
impl WritingSubmissionService {
    pub async fn start_confirmed(
        &self,
        sessions: Arc<AgentSessionService>,
        prepared: PreparedWritingSubmission,
        confirmation: WritingExecutionConfirmation,
        profile: WritingExecutionProfile,
    ) -> Result<SubmissionAdmission, WritingStartError> {
        self.validate_start(&prepared, &confirmation, &profile)
            .await?;
        self.run_prepared(
            sessions,
            prepared,
            profile,
            confirmation.receipt.manuscript_id,
        )
        .await
    }

    pub async fn start_queued_confirmed(
        &self,
        sessions: Arc<AgentSessionService>,
        prepared: PreparedWritingSubmission,
        confirmation: WritingExecutionConfirmation,
        profile: WritingExecutionProfile,
        queue_input: (Uuid, u64),
    ) -> Result<SubmissionAdmission, WritingStartError> {
        self.validate_start_for_queue(&prepared, &confirmation, &profile, Some(queue_input))
            .await?;
        self.run_prepared(
            sessions,
            prepared,
            profile,
            confirmation.receipt.manuscript_id,
        )
        .await
    }
    async fn run_prepared(
        &self,
        sessions: Arc<AgentSessionService>,
        prepared: PreparedWritingSubmission,
        profile: WritingExecutionProfile,
        manuscript_id: Uuid,
    ) -> Result<SubmissionAdmission, WritingStartError> {
        let submissions = SqliteSubmissionStore::for_app(&self.store);
        if let Some(admission) = sessions
            .existing_submission(&submissions, &prepared.record)
            .await?
        {
            return Ok(admission);
        }
        let conversation_id = prepared.record.conversation_id;
        let request_id = prepared.record.request_id;
        let store = self.store.clone();
        let transport = profile
            .transport_with_model_history(
                &self.store,
                &prepared.record,
                prepared.model,
                self.model_resolver.clone(),
                crate::agent::model::AgentHost::Writing,
            )
            .await?;
        #[cfg(all(debug_assertions, feature = "development-data"))]
        let transport = super::native_acceptance_transport::replace(
            &self.store,
            manuscript_id,
            profile.capabilities.clone(),
            transport,
        )
        .await
        .map_err(AgentModelPreparationError::Transport)?;
        let has_images = tokio::task::spawn_blocking(move || {
            let db = store
                .connection()
                .map_err(|_| WritingSubmissionError::Unavailable)?;
            let result = db.query_row(
                "select exists(select 1 from agent_submission_images where conversation_id=?1 and request_id=?2)",
                rusqlite::params![conversation_id.to_string(), request_id.to_string()],
                |row| row.get::<_, bool>(0),
            );
            match result {
                Ok(value) => Ok(value),
                Err(error) if error.to_string().contains("no such table") => Ok(false),
                Err(_) => Err(WritingSubmissionError::Unavailable),
            }
        })
        .await
        .map_err(|_| WritingSubmissionError::Unavailable)??;
        if has_images {
            self.start_transport_with_images(
                sessions,
                prepared.record,
                profile,
                transport,
                manuscript_id,
            )
            .await
        } else {
            self.start_transport_with_manuscript(
                sessions,
                prepared.record,
                profile,
                transport,
                manuscript_id,
            )
            .await
        }
    }
    async fn validate_start(
        &self,
        prepared: &PreparedWritingSubmission,
        confirmation: &WritingExecutionConfirmation,
        profile: &WritingExecutionProfile,
    ) -> Result<(), WritingStartError> {
        self.validate_start_for_queue(prepared, confirmation, profile, None)
            .await
    }
    async fn validate_start_for_queue(
        &self,
        prepared: &PreparedWritingSubmission,
        confirmation: &WritingExecutionConfirmation,
        profile: &WritingExecutionProfile,
        queue_input: Option<(Uuid, u64)>,
    ) -> Result<(), WritingStartError> {
        if self.store.native_writing_context().is_none()
            && (self.store.writing_resource_source().is_none()
                || self.store.writing_compile_feedback_source().is_none())
        {
            return Err(WritingSubmissionError::Unavailable.into());
        }
        let record = &prepared.record;
        if prepared.model.is_chatgpt()
            && !matches!(
                profile.request_policy,
                crate::agent::model::ChatRequestPolicy::ProviderDefault
            )
        {
            return Err(AgentModelPreparationError::Transport(
                research_agent_core::model::ModelError::UnsupportedCapability,
            )
            .into());
        }
        if confirmation.receipt.conversation_id != record.conversation_id
            || confirmation.receipt.request_id != record.request_id
            || confirmation.context_snapshot_id != record.context_snapshot_id
            || confirmation.model != record.input.model
            || profile.model != record.input.model
            || prepared.model.identity() != &record.input.model
            || prepared.model.reasoning_effort() != record.input.reasoning_effort.as_deref()
            || prepared.model.service_tier() != record.input.service_tier.as_deref()
            || record
                .input
                .reasoning_effort
                .as_ref()
                .is_some_and(|effort| !profile.capabilities.reasoning_efforts.contains(effort))
            || profile.max_output_tokens == 0
            || profile.max_input_bytes == 0
            || !profile.capabilities.tools
        {
            return Err(WritingStartError::Mismatch);
        }
        validate_scope(record, confirmation.receipt.manuscript_id)?;
        let store = self.store.clone();
        let record = record.clone();
        let manuscript = confirmation.receipt.manuscript_id;
        tokio::task::spawn_blocking(move || {
            let mut db = store
                .connection()
                .map_err(|_| WritingSubmissionError::Unavailable)?;
            check_binding(&db, record.conversation_id, manuscript)?;
            // A frozen compact preparation cannot become ordinary user input,
            // including through the queue's normal admission path.
            if super::compaction::read_in(&db, record.conversation_id, record.request_id)?.is_some() {
                return Err(SubmissionError::Conflict.into());
            }
            let queued: bool = db.query_row(
                "select exists(select 1 from agent_queue_submission_origins where conversation_id=?1 and request_id=?2)",
                params![record.conversation_id.to_string(),record.request_id.to_string()], |row|row.get(0))
                .map_err(|_|WritingSubmissionError::Unavailable)?;
            if queued != queue_input.is_some() { return Err(SubmissionError::Conflict.into()); }
            let saved = read(&db, record.conversation_id, record.request_id)?
                .ok_or(SubmissionError::NotFound)?;
            if saved != record {
                return Err(SubmissionError::Conflict.into());
            }
            if let Some((input_id, revision)) = queue_input {
                use crate::agent::repository::input_queue::{admit_and_resume_checked, Error};
                // This is an explicit confirmation of the current queue revision.
                // Receipt replay never resumes a pause committed after admission;
                // the journal still rechecks that barrier before TurnStarted.
                admit_and_resume_checked(&mut db, record.conversation_id, record.request_id, revision,
                    input_id, record.input.clone(), |db| {
                        check_binding(db,record.conversation_id,manuscript).map_err(|error| match error {
                            WritingSubmissionError::Unbound=>Error::Closed,_=>Error::Storage,
                        })
                    }).map_err(|error| match error {
                        Error::Storage=>WritingSubmissionError::Unavailable,
                        Error::Closed=>WritingSubmissionError::Unbound,
                        _=>SubmissionError::Conflict.into(),
                    })?;
            }
            Ok::<_, WritingSubmissionError>(())
        })
        .await
        .map_err(|_| WritingSubmissionError::Unavailable)??;
        Ok(())
    }
    #[allow(dead_code, reason = "kept as a fixture-friendly transport entry")]
    async fn start_transport(
        &self,
        sessions: Arc<AgentSessionService>,
        record: SubmissionRecord,
        profile: WritingExecutionProfile,
        transport: Arc<dyn ModelTransport>,
    ) -> Result<SubmissionAdmission, WritingStartError> {
        self.start_transport_with_manuscript(sessions, record, profile, transport, Uuid::nil())
            .await
    }
    async fn start_transport_with_manuscript(
        &self,
        sessions: Arc<AgentSessionService>,
        record: SubmissionRecord,
        profile: WritingExecutionProfile,
        transport: Arc<dyn ModelTransport>,
        manuscript_id: Uuid,
    ) -> Result<SubmissionAdmission, WritingStartError> {
        self.start_transport_inner(sessions, record, profile, transport, false, manuscript_id)
            .await
    }
    async fn start_transport_with_images(
        &self,
        sessions: Arc<AgentSessionService>,
        record: SubmissionRecord,
        profile: WritingExecutionProfile,
        transport: Arc<dyn ModelTransport>,
        manuscript_id: Uuid,
    ) -> Result<SubmissionAdmission, WritingStartError> {
        self.start_transport_inner(sessions, record, profile, transport, true, manuscript_id)
            .await
    }
    async fn start_transport_inner(
        &self,
        sessions: Arc<AgentSessionService>,
        record: SubmissionRecord,
        profile: WritingExecutionProfile,
        transport: Arc<dyn ModelTransport>,
        with_images: bool,
        manuscript_id: Uuid,
    ) -> Result<SubmissionAdmission, WritingStartError> {
        let submissions = SqliteSubmissionStore::for_app(&self.store);
        if let Some(admission) = sessions.existing_submission(&submissions, &record).await? {
            return Ok(admission);
        }
        let mcp_runtime = super::super::child_runtime::mcp_runtime_for_conversation(
            &self.store,
            &sessions,
            record.conversation_id,
        )
        .await
        .map(|runtime| {
            if transport.supports_namespace_tools() {
                runtime.namespaced_view()
            } else {
                runtime
            }
        });
        let approved_servers = mcp_runtime
            .as_ref()
            .map(|runtime| runtime.approved_servers())
            .unwrap_or_default();
        let child_context =
            self.model_resolver
                .clone()
                .map(|models| super::tools::ChildSpawnContext {
                    store: self.store.clone(),
                    catalog: crate::agent::repository::SqliteConversationCatalog::from_app_store(
                        &self.store,
                    ),
                    service: self.clone(),
                    models,
                    sessions: sessions.clone(),
                    parent_conversation_id: record.conversation_id,
                    parent_turn_id: record.turn_id,
                    context_snapshot_id: record.context_snapshot_id,
                    manuscript_id,
                    approved_servers,
                });
        let tools = super::tools::for_submission_with_mcp_and_child(
            self.store.clone(),
            record.clone(),
            sessions.clone(),
            mcp_runtime,
            child_context,
        )
        .await;
        let tools = match tools {
            Ok(tools) => tools,
            Err(error) => {
                // Another caller may have admitted and applied this submission
                // while this caller was preparing tools against its old snapshot.
                if let Some(admission) = sessions.existing_submission(&submissions, &record).await?
                {
                    return Ok(admission);
                }
                return Err(error.into());
            }
        };
        let tool_definitions = tools.definitions();
        let workspace = tools.hook_workspace();
        #[cfg(target_os = "macos")]
        let stop_hooks = match &self.model_resolver {
            Some(models) => {
                crate::agent::execution::NativeStopBinding::prepare(
                    models,
                    crate::agent::model::AgentHost::Writing,
                    &self.store,
                    &record,
                    workspace.clone(),
                )
                .await?
            }
            None => None,
        };
        #[cfg(target_os = "macos")]
        let file_workspace = tools.file_workspace();
        let compaction_hooks = match &self.model_resolver {
            Some(models) => {
                models
                    .prepare_compaction_hooks_for_profile(
                        crate::agent::model::AgentHost::Writing,
                        &record,
                        &profile,
                        workspace.clone(),
                    )
                    .await?
            }
            None => None,
        };
        let host_tools = research_agent_core::tool_registry::OwnedToolRegistration::new(
            tool_definitions,
            Arc::new(tools),
        );
        #[cfg(target_os = "macos")]
        let host_tools = crate::agent::compaction_hook_spill::tool::registration(
            self.store.clone(),
            record.clone(),
            &profile,
            workspace,
            host_tools,
        )
        .map_err(|_| SubmissionRunError::RecoveryRequired)?;
        #[cfg(target_os = "macos")]
        let host_tools = crate::agent::context_session::resource_tools::registration_with_workspace(
            self.store.clone(),
            record.clone(),
            host_tools,
            file_workspace,
        );
        #[cfg(not(target_os = "macos"))]
        let host_tools = crate::agent::context_session::resource_tools::registration(
            self.store.clone(),
            record.clone(),
            host_tools,
        );
        let execution = Arc::new(AgentExecution {
            #[cfg(target_os = "macos")]
            stop_hooks,
            compaction_hooks,
            compaction_strategy: Some(profile.compaction_strategy(transport.clone())?),
            model: record.input.model.clone(),
            transport,
            host_tools,
            system_instructions: crate::agent::host_instructions::WRITING.into(),
            limits: profile.limits,
            max_output_tokens: profile.max_output_tokens,
            max_input_bytes: profile.max_input_bytes,
        });
        let steering = Some(Arc::new(
            crate::agent::repository::steering::SqliteSteeringStore::new(
                self.store.shared_agent_connection(),
            ),
        )
            as Arc<dyn research_agent_core::steering::SteeringStore>);
        let materializer =
            Arc::new(crate::agent::repository::submissions::AppImageMaterializer::new(&self.store));
        if !with_images {
            let admission = execution
                .start_with_steering(
                    sessions,
                    Arc::new(SqliteSubmissionStore::for_app(&self.store)),
                    ConversationId::from_uuid(record.conversation_id),
                    record.request_id,
                    steering,
                )
                .await?;
            return Ok(admission);
        }
        let submissions = Arc::new(SqliteSubmissionStore::for_app(&self.store));
        let admission = execution
            .start_with_images_and_steering(
                sessions,
                submissions,
                materializer,
                ConversationId::from_uuid(record.conversation_id),
                record.request_id,
                steering,
            )
            .await?;
        Ok(admission)
    }
}
#[cfg(test)]
#[path = "execution/tests.rs"]
mod tests;
