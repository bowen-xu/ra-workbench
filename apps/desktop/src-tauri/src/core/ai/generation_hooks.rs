//! Paper policy hooks inside the existing Core shared submission owner.
use super::{
    error::AiError,
    generation_support::{ProviderInput, VisionGenerationResponse},
};
use crate::storage::AppStore;
use research_agent_core::{
    submission::{ContextText, SubmissionInput},
    user_input::UserInput,
};
use uuid::Uuid;

pub(crate) use research_workbench_native_sdk::analysis_workflow::prompt::PaperTaskPrompt;
pub(crate) type PaperCompletionFuture<'a> =
    std::pin::Pin<Box<dyn Future<Output = Result<VisionGenerationResponse, AiError>> + Send + 'a>>;

/// Static App composition and selected Native policy share the original Core
/// submission owner. Native waits borrow no SQL guard or resource authority.
pub(crate) trait PreparedPaperWorkflow: Send + Sync {
    fn kind(&self) -> &str;
    fn grants(&self) -> &[ContextText];
    fn prompt_text(&self) -> &str;
    fn prompt_user_input(&self) -> &[UserInput];
    fn decorate(
        &self,
        input: &mut ProviderInput,
        submission: &SubmissionInput,
    ) -> Result<(), AiError>;
    fn complete<'a>(
        &'a self,
        store: &'a AppStore,
        conversation: Uuid,
        request: Uuid,
        response: VisionGenerationResponse,
    ) -> PaperCompletionFuture<'a>;
}

pub(crate) type PreparePaperSubmission =
    fn(
        &AppStore,
        Uuid,
        Uuid,
        Option<&SubmissionInput>,
    ) -> Result<Option<Box<dyn PreparedPaperWorkflow>>, AiError>;
