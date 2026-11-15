//! Original definition/revision reads. No database guard crosses a Native wait.
use super::*;
use crate::{
    ai::error::AiError,
    runtime_plugins::literature_knowledge_promotion::{KnowledgeSql, PromotionError, manual},
    storage::AppStore,
};
use research_workbench_native_sdk::{
    analysis_workflow::wire::{WorkflowDecisionV1, WorkflowInputV1},
    knowledge_promotion::{
        KnowledgeValidationInputV1 as Input, manual::ResearchObjectRevision,
        models::DefinitionClaim,
    },
};
impl LiteratureTraceService {
    pub(crate) async fn list_definitions(
        &self,
        store: AppStore,
        key: String,
        concept_id: String,
    ) -> Result<Vec<DefinitionClaim>, AiError> {
        self.available()
            .map_err(super::concept_capture::technical)?;
        let _lifetime = self.doi_operations.read().await;
        self.available()
            .map_err(super::concept_capture::technical)?;
        let sql = self.manual_knowledge_sql(key).await?;
        tokio::task::spawn_blocking(move || {
            let db = store.connection()?;
            manual::read_definitions::<std::convert::Infallible>(&db, &sql, &concept_id).map_err(|error| match error {
                PromotionError::Failed(error) => error,
                crate::runtime_plugins::literature_knowledge_promotion::PromotionError::Need(never) => match never {},
            })
        }).await.map_err(|_| AiError::TemporaryUnavailable)?
    }
    pub(crate) async fn list_knowledge_revisions(
        &self,
        store: AppStore,
        key: String,
        object_kind: String,
        object_id: String,
    ) -> Result<Vec<ResearchObjectRevision>, AiError> {
        self.available()
            .map_err(super::concept_capture::technical)?;
        let _lifetime = self.doi_operations.read().await;
        let sql = self.manual_knowledge_sql(key.clone()).await?;
        let (decision, origin) = self
            .analysis_workflow_decision(
                &key,
                WorkflowInputV1::KnowledgeValidation {
                    ordinal: 0,
                    input: Box::new(Input::RevisionKind {
                        object_kind: object_kind.clone(),
                    }),
                },
            )
            .await
            .map_err(super::concept_capture::technical)?;
        let WorkflowDecisionV1::KnowledgeValidation { decision } = decision else {
            return Err(super::concept_capture::identified(
                "plugin_host_response_mismatch",
                "研究对象版本校验种类不匹配。",
                &origin,
            ));
        };
        if let Some(message) =
            crate::runtime_plugins::literature_knowledge_promotion::validation_rejection(&decision)
        {
            return Err(super::concept_capture::identified(
                "invalid_input",
                &message,
                &origin,
            ));
        }
        tokio::task::spawn_blocking(move || {
            let db = store.connection()?;
            manual::read_revisions(&db, &sql, &object_kind, &object_id).map_err(AiError::Knowledge)
        })
        .await
        .map_err(|_| AiError::TemporaryUnavailable)?
    }
    async fn manual_knowledge_sql(&self, key: String) -> Result<KnowledgeSql, AiError> {
        let catalog = Arc::clone(&self.catalog);
        tokio::task::spawn_blocking(move || {
            let package = package::accept_analysis_workflow(&catalog, &key)
                .map_err(super::concept_capture::technical)?;
            package::accept_concepts(&catalog, &key).map_err(super::concept_capture::technical)?;
            KnowledgeSql::load_manual(&catalog, &package).map_err(|error| {
                super::concept_capture::technical(
                    Self::error(error.code, error.message).package(&key),
                )
            })
        })
        .await
        .map_err(|_| AiError::TemporaryUnavailable)?
    }
}
