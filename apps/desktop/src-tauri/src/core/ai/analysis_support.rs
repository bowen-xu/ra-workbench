//! Shared-Agent facts, resource admission and original physical persistence for
//! Native analysis composition. Callers select the domain rules.
#[path = "analysis_support/resources.rs"]
mod resources;
#[path = "analysis_support/runner.rs"]
mod runner;
use super::{
    error::AiError,
    reading_contracts::{LlmModelSelection, LlmModelSelectionDraft},
};
use crate::storage::AppStore;
use research_agent_core::{
    conversation::ConversationId, protocol::TurnOutcome, submission::SubmissionInput,
};
pub(crate) use runner::{AnalysisHost, PreparedWorkflow, start_workflow};
use rusqlite::{Connection, OptionalExtension, params};
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub(crate) struct AnalysisSupportError(Box<AiError>);
impl AnalysisSupportError {
    pub(crate) fn into_ai_error(self) -> AiError {
        *self.0
    }
}
impl From<AiError> for AnalysisSupportError {
    fn from(e: AiError) -> Self {
        Self(Box::new(e))
    }
}
impl From<rusqlite::Error> for AnalysisSupportError {
    fn from(e: rusqlite::Error) -> Self {
        Self(Box::new(e.into()))
    }
}
pub(crate) type FrozenResources = (
    Vec<research_agent_core::skills::SubmissionSkill>,
    Vec<research_agent_core::skills::instructions::SkillInstructions>,
    Vec<research_agent_core::user_input::UserInput>,
);
/// Actual application resource authority. No directory, store or credential
/// handle is exposed to the domain policy.
pub(crate) struct AnalysisResources<'a> {
    store: &'a AppStore,
}
/// Borrowed shared execution facts. It cannot commit, open another connection,
/// execute caller SQL or expose the underlying database.
pub(crate) struct AnalysisJournal<'a> {
    db: &'a Connection,
}
impl<'a> AnalysisJournal<'a> {
    pub(crate) fn new(db: &'a Connection) -> Self {
        Self { db }
    }

    pub(crate) fn grants(
        &self,
        paper: &str,
        attachments: &[String],
        conversation: Uuid,
        kind: &str,
        snapshot: &str,
        writing: Option<crate::agent::writing_resources::WritingResourceSource>,
    ) -> Result<Vec<research_agent_core::submission::ContextText>, AnalysisSupportError> {
        super::structured_generation::paper_grants(
            self.db,
            paper,
            attachments,
            conversation,
            kind,
            Some(snapshot),
            writing,
        )
        .map_err(Into::into)
    }

    pub(crate) fn managed_templates(&self) -> Result<bool, AnalysisSupportError> {
        self.db.query_row("select exists(select 1 from sqlite_master where type='table' and name='structured_paper_template_owners')",[],|r|r.get(0)).map_err(Into::into)
    }

    pub(crate) fn ensure_completed(&self, attempt: &str) -> Result<(), AnalysisSupportError> {
        let Some((conversation,turn)):Option<(String,String)>=self.db.query_row("select conversation_id, turn_id from agent_paper_analysis_submissions where attempt_id = ?1",[attempt],|r|Ok((r.get(0)?,r.get(1)?))).optional()? else{return Ok(())};
        let conversation = ConversationId::from_uuid(
            Uuid::parse_str(&conversation).map_err(|_| AiError::TemporaryUnavailable)?,
        );
        let turn = Uuid::parse_str(&turn).map_err(|_| AiError::TemporaryUnavailable)?;
        match crate::agent::repository::projection::read_turn_outcome(self.db, conversation, turn)?
        {
            Some(TurnOutcome::Completed) => Ok(()),
            _ => Err(AiError::TemporaryUnavailable.into()),
        }
    }
    pub(crate) fn candidate_matches(
        &self,
        run: &str,
        candidate: &str,
    ) -> Result<bool, AnalysisSupportError> {
        self.db
            .query_row(
                "select exists(select 1 from ai_candidates where id = ?1 and run_id = ?2)",
                params![candidate, run],
                |r| r.get(0),
            )
            .map_err(Into::into)
    }

    pub(crate) fn candidate_outcome(
        &self,
        run: &str,
        kind: &str,
    ) -> Result<CandidateOutcome, AnalysisSupportError> {
        self.db.query_row("select candidate.id, run.scope_hash, revision.payload_hash, run.input_tokens, run.output_tokens from ai_candidates candidate join ai_candidate_revisions revision on revision.candidate_id = candidate.id and revision.revision = candidate.current_revision join ai_runs run on run.id = candidate.run_id where candidate.run_id = ?1 and candidate.candidate_kind = ?2",params![run,kind],|r|Ok(CandidateOutcome{candidate_id:r.get(0)?,input_hash:r.get(1)?,output_hash:r.get(2)?,input_tokens:r.get(3)?,output_tokens:r.get(4)?})).optional()?.ok_or_else(||AiError::InvalidInput("AI 运行没有生成预期候选".into()).into())
    }
}
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct CandidateOutcome {
    pub candidate_id: String,
    pub input_hash: String,
    pub output_hash: String,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
}

pub(crate) use super::error::AiCommandError;
impl AnalysisSupportError {
    pub(crate) fn paper_context(
        error: crate::ai::paper_reading_contracts::PaperContextError,
    ) -> Self {
        use crate::ai::paper_reading_contracts::PaperContextError;
        match error {
            PaperContextError::InvalidInput
            | PaperContextError::Conflict
            | PaperContextError::Budget => {
                AiError::InvalidInput("论文分析共享上下文无效或超过预算".into())
            }
            PaperContextError::Unavailable | PaperContextError::Corrupt => {
                AiError::TemporaryUnavailable
            }
        }
        .into()
    }
    pub(crate) fn canceled() -> Self {
        Self::from(AiError::Canceled)
    }
    pub(crate) fn into_command_error(self) -> AiCommandError {
        self.into_ai_error().into()
    }
}
/// Existing source preparation and overview runner, with the same Core owners.
/// This capability delegates execution; it contains no workflow orchestration.
#[derive(Clone)]
pub(crate) struct AnalysisExecution {
    store: AppStore,
    credentials: super::credential::JsonCredentialStore,
    models: crate::agent::model::AgentModelResolver,
    sessions: std::sync::Arc<crate::agent::service::AgentSessionService>,
}
impl AnalysisExecution {
    pub(crate) fn new(
        store: AppStore,
        credentials: super::credential::JsonCredentialStore,
        models: crate::agent::model::AgentModelResolver,
        sessions: std::sync::Arc<crate::agent::service::AgentSessionService>,
    ) -> Self {
        Self {
            store,
            credentials,
            models,
            sessions,
        }
    }
    pub(crate) async fn prepare_sources(
        &self,
        paper: &str,
        attachments: &[String],
    ) -> Result<(), AiCommandError> {
        super::paper_generation::prepare_sources(&self.store, paper, attachments)
            .await
            .map_err(Into::into)
    }
    pub(crate) async fn overview(
        &self,
        draft: crate::ai::paper_analysis_contracts::DoclingPaperOverviewDraft,
        signal: super::CancellationSignal,
    ) -> Result<String, AiCommandError> {
        super::paper_overview_docling::run_docling_paper_overview(
            self.store.clone(),
            self.credentials.clone(),
            self.models.clone(),
            draft,
            Some(signal),
            Some(self.sessions.clone()),
        )
        .await
        .map(|run| run.id)
    }
}

/// Fixed shared writes borrow the caller's original transaction; no commit escapes.
pub(crate) struct AnalysisAdmission<'tx, 'db> {
    transaction: &'tx rusqlite::Transaction<'db>,
}
pub(crate) struct AnalysisSubmission<'a> {
    pub attempt: &'a str,
    pub conversation: Uuid,
    pub request: Uuid,
    pub input: SubmissionInput,
    pub paper: &'a str,
    pub snapshot: &'a str,
    pub task_kind: &'a str,
}
impl<'tx, 'db> AnalysisAdmission<'tx, 'db> {
    pub(crate) fn new(transaction: &'tx rusqlite::Transaction<'db>) -> Self {
        Self { transaction }
    }
    pub(crate) fn bind_workflow(
        &self,
        workflow: &str,
        conversation: Uuid,
        paper: &str,
        snapshot: &str,
    ) -> Result<(), AnalysisSupportError> {
        self.transaction.execute("insert into agent_paper_analysis_bindings (workflow_id, conversation_id, paper_id, context_snapshot_id, created_at) values (?1, ?2, ?3, ?4, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",params![workflow,conversation.to_string(),paper,snapshot])?;
        Ok(())
    }
    pub(crate) fn admit(
        &self,
        resources: Option<&AnalysisResources<'_>>,
        submission: AnalysisSubmission<'_>,
    ) -> Result<(), AnalysisSupportError> {
        let AnalysisSubmission {
            attempt,
            conversation,
            request,
            mut input,
            paper,
            snapshot,
            task_kind,
        } = submission;
        if let Some(resources) = resources {
            super::structured_generation::bind_task_skill(
                resources.store.agent_configuration_directory(),
                task_kind,
                &mut input,
            )?;
        }
        let record = crate::agent::repository::submissions::insert_in(
            self.transaction,
            conversation,
            request,
            input,
        )
        .map_err(|e| match e {
            research_agent_core::submission::SubmissionError::InvalidInput => {
                AiError::InvalidInput("论文分析输入超出共享提交容量或格式无效".into())
            }
            _ => AiError::TemporaryUnavailable,
        })?;
        if let Some(resources) = resources {
            crate::agent::skills::injection::validate_mentions(
                resources.store,
                self.transaction,
                &record.input.user_input,
                &record.input.context,
            )
            .map_err(|_| AiError::InvalidInput("资源引用已失效或尚未授权".into()))?;
            crate::agent::skills::steering::freeze_grants(
                self.transaction,
                resources.store,
                &record,
            )
            .map_err(|_| AiError::TemporaryUnavailable)?;
        }
        self.transaction.execute("insert into agent_paper_analysis_submissions (attempt_id, conversation_id, request_id, turn_id, context_snapshot_id, paper_id, created_at) values (?1, ?2, ?3, ?4, ?5, ?6, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",params![attempt,conversation.to_string(),request.to_string(),record.turn_id.to_string(),snapshot,paper])?;
        Ok(())
    }
    pub(crate) fn complete(
        &self,
        attempt: &str,
        run: &str,
        candidate: &str,
    ) -> Result<(), AnalysisSupportError> {
        self.transaction.execute("update agent_paper_analysis_submissions set ai_run_id = ?2, candidate_id = ?3, terminal_status = 'succeeded', updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') where attempt_id = ?1",params![attempt,run,candidate])?;
        Ok(())
    }
    pub(crate) fn fail(&self, attempt: &str, status: &str) -> Result<(), AnalysisSupportError> {
        self.transaction.execute("update agent_paper_analysis_submissions set terminal_status = ?2, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') where attempt_id = ?1",params![attempt,status])?;
        Ok(())
    }
}
