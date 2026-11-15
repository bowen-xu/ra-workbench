//! Original application resource freezing and model selection retain their Core owner.
use super::*;
impl<'a> AnalysisResources<'a> {
    pub(crate) fn freeze_application_resources(
        &self,
        source_ids: &[String],
        references: &[research_agent_core::user_input::UserInput],
    ) -> Result<FrozenResources, AnalysisSupportError> {
        use research_agent_core::user_input::UserInput;
        if !references.is_empty() {
            if references
                .iter()
                .any(|input| matches!(input, UserInput::Text { .. }))
            {
                return Err(AiError::InvalidInput("资源选择不能包含额外文本".into()).into());
            }
            let mut input = vec![UserInput::Text {
                text: "Extract".into(),
                text_elements: vec![],
            }];
            input.extend_from_slice(references);
            let (skills, prompts) = self.freeze_references("Extract", &input)?;
            return Ok((skills, prompts, references.to_vec()));
        }
        self.freeze_sources(source_ids)
    }
    pub(crate) fn new(store: &'a AppStore) -> Self {
        Self { store }
    }

    pub(crate) fn selection(
        &self,
        db: &Connection,
        route: &str,
        selection: Option<&LlmModelSelectionDraft>,
    ) -> Result<LlmModelSelection, AnalysisSupportError> {
        crate::ai::llm_configuration::resolve_model_selection(db, route, selection)
            .map_err(Into::into)
    }

    pub(crate) fn freeze_references(
        &self,
        prompt: &str,
        input: &[research_agent_core::user_input::UserInput],
    ) -> Result<
        (
            Vec<research_agent_core::skills::SubmissionSkill>,
            Vec<research_agent_core::skills::instructions::SkillInstructions>,
        ),
        AnalysisSupportError,
    > {
        crate::agent::skills::injection::freeze_application(
            self.store.agent_configuration_directory(),
            prompt,
            input,
        )
        .map_err(|_| AiError::InvalidInput("技能引用已失效，请重新选择".into()).into())
    }
    pub(crate) fn freeze_sources(
        &self,
        ids: &[String],
    ) -> Result<FrozenResources, AnalysisSupportError> {
        let skills = crate::agent::skills::application::freeze_sources(
            self.store.agent_configuration_directory(),
            ids,
        )
        .map_err(|_| AiError::TemporaryUnavailable)?;
        let root = self
            .store
            .agent_configuration_directory()
            .canonicalize()
            .map_err(|_| AiError::TemporaryUnavailable)?;
        let prompts = crate::agent::skills::injection::load_prompts(&root, &skills, |_| Ok(()))
            .map_err(|_| AiError::TemporaryUnavailable)?;
        Ok((skills, prompts, vec![]))
    }
}
