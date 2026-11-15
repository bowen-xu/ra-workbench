//! Fixed policy values selected by App for the existing generation consumers.
//! Connection arguments are internal repository composition, not plugin SQL APIs.
use std::collections::HashSet;

use rusqlite::{Connection, OptionalExtension};
use serde_json::Value;

use super::{
    candidate_contracts::CandidatePayload,
    error::AiError,
    model::{AiEvidenceBundle, AiMatrixSynthesisContext, AiOverviewProfileContext},
};
use crate::ai::paper_analysis_contracts::PaperTemplateRef;

pub(crate) struct PreparedCandidate {
    pub kind: &'static str,
    pub canonical_json: String,
}

pub(crate) type GeneratedCandidatePolicy =
    fn(&Connection, &str, &AiEvidenceBundle, &str) -> Result<PreparedCandidate, AiError>;
pub(crate) type RevisedCandidatePolicy = fn(
    &Connection,
    &str,
    &str,
    &str,
    &AiEvidenceBundle,
    Value,
) -> Result<PreparedCandidate, AiError>;
pub(crate) type TemplateResolver =
    fn(&Connection, &str, &PaperTemplateRef) -> Result<AiOverviewProfileContext, AiError>;
pub(crate) type OverviewDispatch =
    for<'a> fn(
        &'a tauri::AppHandle,
        &'a crate::storage::AppStore,
        &'a super::credential::JsonCredentialStore,
        &'a super::model::AiRun,
        &'a super::CancellationSignal,
        &'a std::sync::atomic::AtomicU64,
    ) -> std::pin::Pin<Box<dyn Future<Output = Result<(), AiError>> + Send + 'a>>;
pub(crate) type DoclingDispatch = fn(
    crate::storage::AppStore,
    super::credential::JsonCredentialStore,
    crate::agent::model::AgentModelResolver,
    crate::ai::paper_analysis_contracts::DoclingPaperOverviewDraft,
    Option<super::CancellationSignal>,
    Option<std::sync::Arc<crate::agent::service::AgentSessionService>>,
) -> std::pin::Pin<
    Box<dyn Future<Output = Result<super::model::AiRun, super::error::AiCommandError>> + Send>,
>;

pub(crate) type TaskEvidencePreviewPolicy =
    for<'a> fn(
        &Connection,
        super::task_evidence_support::TaskEvidencePreview<'a>,
    ) -> Result<Vec<super::task_evidence_support::AiEvidencePreview>, AiError>;
pub(crate) type TaskEvidencePersistPolicy = fn(
    &mut Connection,
    super::task_evidence_support::TaskEvidenceKind,
    &str,
) -> Result<AiEvidenceBundle, AiError>;
pub(crate) type MatrixContextPolicy = fn(
    &Connection,
    Option<&super::task_evidence_support::AiMatrixSynthesisContextDraft>,
    &[super::task_evidence_support::AiScopeItem],
) -> Result<Option<AiMatrixSynthesisContext>, AiError>;
pub(crate) type UnreviewedFieldPolicy = fn(&[String], &[String], &[String]) -> Vec<String>;
/// No default, lookup, serialization or runtime registration is introduced.
#[derive(Clone, Copy)]
pub(crate) struct GenerationPolicySource {
    pub execute_suggestions: OverviewDispatch,
    pub execute_research_task: OverviewDispatch,
    pub task_shape: fn(&super::model::AiTaskSpecDraft) -> Result<(), AiError>,
    pub task_scope: fn(&str, &[super::model::AiScopeItem]) -> Result<(), AiError>,
    pub matrix_context: MatrixContextPolicy,
    pub evidence_preview: TaskEvidencePreviewPolicy,
    pub evidence_persist: TaskEvidencePersistPolicy,
    pub execute_overview: OverviewDispatch,
    pub run_docling: DoclingDispatch,

    pub prepare_submission: super::generation_hooks::PreparePaperSubmission,

    pub generated: GeneratedCandidatePolicy,
    pub revised: RevisedCandidatePolicy,
    pub structured: fn(&str, Value, &AiEvidenceBundle) -> Result<Value, AiError>,

    pub decode_value: fn(Value, &HashSet<String>) -> Result<CandidatePayload, AiError>,

    pub template_resolve: TemplateResolver,
    pub unreviewed_fields: UnreviewedFieldPolicy,
    pub editable_template: fn(Value) -> Result<Value, String>,
}

/// App borrows its original connection. The domain can read only these fixed
/// shared facts, without obtaining the connection, Store or transaction owner.
pub(crate) struct CandidateValidationFacts<'db> {
    db: &'db Connection,
}

impl<'db> CandidateValidationFacts<'db> {
    pub(crate) fn new(db: &'db Connection) -> Self {
        Self { db }
    }

    pub(crate) fn run_output(&self, run_id: &str) -> Result<(String, Option<String>), AiError> {
        self.db
            .query_row(
                "select r.task_kind, s.candidate_kind
             from ai_runs r left join ai_suggestion_run_contexts s on s.run_id = r.id
             where r.id = ?1",
                [run_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
            )
            .optional()?
            .ok_or_else(|| AiError::InvalidInput("找不到候选来源运行".to_owned()))
    }

    pub(crate) fn overview_profile(
        &self,
        run: &str,
    ) -> Result<Option<AiOverviewProfileContext>, AiError> {
        super::overview_profile_repository::load_overview_profile(self.db, run)
    }

    pub(crate) fn matrix_context(
        &self,
        run: &str,
    ) -> Result<Option<AiMatrixSynthesisContext>, AiError> {
        super::matrix_synthesis_context_repository::load_matrix_synthesis_context(self.db, run)
    }

    pub(crate) fn active_confirmed_fields(
        &self,
        candidate: &str,
    ) -> Result<Vec<(String, String)>, AiError> {
        // Decode the original complete ordered history before filtering active.
        Ok(
            super::overview_confirmation_repository::list_overview_field_confirmations(
                self.db, candidate,
            )?
            .into_iter()
            .filter(|field| field.active)
            .map(|field| (field.field_kind, field.field_payload_hash))
            .collect(),
        )
    }
}
