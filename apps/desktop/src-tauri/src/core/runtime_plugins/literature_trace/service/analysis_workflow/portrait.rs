//! Original portrait reads and evidence publication use existing Core owners.
use super::*;
use crate::{
    ai::{
        candidate_contracts::AiEvidenceBundle, error::AiError, generation_support::AiRun,
        run_history::RunHistoryReader,
    },
    storage::AppStore,
};

impl LiteratureTraceService {
    async fn validated_portrait_input(
        &self,
        key: &str,
        input: WorkflowInputV1,
    ) -> Result<(), AiError> {
        let (decision, origin) = self
            .analysis_workflow_decision(key, input)
            .await
            .map_err(template::technical)?;
        match decision {
            WorkflowDecisionV1::Validated { rejection: None } => Ok(()),
            WorkflowDecisionV1::Validated {
                rejection: Some(error),
            } => Err(template::rejected(error, &origin)),
            _ => Err(template::identified(
                "plugin_host_response_mismatch",
                "文献画像判断种类不匹配。",
                &origin,
            )),
        }
    }

    pub(crate) async fn validate_portrait_evidence(
        &self,
        key: &str,
        bundle: &AiEvidenceBundle,
    ) -> Result<(), AiError> {
        self.validated_portrait_input(
            key,
            WorkflowInputV1::PortraitEvidence {
                status: bundle.validation_status.clone(),
                items: bundle.items.len(),
            },
        )
        .await
    }

    pub(crate) async fn portrait_evidence(
        self: &Arc<Self>,
        store: AppStore,
        key: String,
        run: AiRun,
    ) -> Result<AiEvidenceBundle, AiError> {
        let paper = run
            .scope_items
            .first()
            .and_then(|scope| scope.parent_object_id.clone())
            .ok_or_else(|| AiError::InvalidInput("画像任务缺少所属文献".into()))?;
        let files = run
            .scope_items
            .iter()
            .map(|scope| scope.object_id.clone())
            .collect();
        let sources = self
            .prepare_portrait_sources(store.clone(), key.clone(), paper, files)
            .await?;
        self.validated_portrait_input(
            &key,
            WorkflowInputV1::PortraitSources {
                statuses: sources
                    .attachment_statuses
                    .iter()
                    .map(|source| source.parse_status.clone())
                    .collect(),
                context_items: sources.context_items.len(),
            },
        )
        .await?;
        self.persist_analysis_docling_evidence(
            store,
            key,
            run,
            sources.context_items,
            Default::default(),
        )
        .await
    }

    pub(crate) async fn latest_portrait_run(
        &self,
        store: AppStore,
        key: String,
        paper: String,
    ) -> Result<Option<AiRun>, crate::ai::error::AiCommandError> {
        self.validated_portrait_input(
            &key,
            WorkflowInputV1::PortraitPaper {
                paper_id: paper.clone(),
            },
        )
        .await?;
        tokio::task::spawn_blocking(move || {
            let db = store.connection()?;
            RunHistoryReader::new(&db).latest_for_parent("paper_profile_classification", &paper)
        })
        .await
        .map_err(|_| crate::ai::error::AiCommandError::from(AiError::TemporaryUnavailable))?
    }
}
