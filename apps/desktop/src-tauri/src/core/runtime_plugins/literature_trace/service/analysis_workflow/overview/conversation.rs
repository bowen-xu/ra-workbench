//! Reuse the original two conversation decisions, then recapture inside run admission.
use super::*;
use crate::{
    ai::structured_resource_validation::{FailureCause, Flow},
    runtime_plugins::literature_analysis::{
        commands::original_error,
        conversation_store::{existing_generation_conversation, resolve_generation_conversation},
        creation_policy::{CreationNeed, CreationPolicy},
        creation_resources::ConversationResources,
        error::AnalysisError,
    },
};
use rusqlite::Connection;
use uuid::Uuid;

pub(super) struct PreparedConversation {
    paper: String,
    key: String,
    resources: ConversationResources,
    policy: CreationPolicy,
}
impl PreparedConversation {
    pub(super) fn resolve(
        &self,
        db: &Connection,
        paper: &str,
        candidate: Uuid,
    ) -> Result<Uuid, AiError> {
        if paper != self.paper {
            return Err(self.changed());
        }
        resolve_generation_conversation(
            db,
            self.resources.statements(),
            &self.policy,
            paper,
            candidate,
        )
        .map_err(|error| self.error(error))
    }
    fn changed(&self) -> AiError {
        match self.policy.latest_origin() {
            Some(origin) => template::identified(
                "plugin_host_source_changed",
                "文献生成会话在准备后变化，请重新载入。",
                origin,
            ),
            None => template::technical(
                LiteratureTraceService::error(
                    "plugin_host_source_changed",
                    "文献生成会话在准备后变化，请重新载入。",
                )
                .package(&self.key),
            ),
        }
    }
    fn error(&self, error: Flow<AnalysisError, CreationNeed>) -> AiError {
        match error {
            Flow::Need(_) => self.changed(),
            Flow::Fail(failure) => {
                let error = original_error(failure.error);
                match failure.cause {
                    FailureCause::NativeRejection(origin) => template::rejected(error, &origin),
                    _ => error,
                }
            }
        }
    }
}

impl LiteratureTraceService {
    pub(super) async fn prepare_portrait_conversation(
        &self,
        store: AppStore,
        key: String,
        paper: String,
    ) -> Result<PreparedConversation, AiError> {
        let catalog = Arc::clone(&self.catalog);
        let selected = key.clone();
        let resources = tokio::task::spawn_blocking(move || {
            let package = package::accept_analysis_workflow(&catalog, &selected)?;
            ConversationResources::load(&catalog, &package)
                .map_err(|error| Self::error(error.code, error.message).package(&selected))
        })
        .await
        .map_err(|_| AiError::TemporaryUnavailable)?
        .map_err(template::technical)?;
        let mut prepared = PreparedConversation {
            paper,
            key,
            resources,
            policy: CreationPolicy::default(),
        };
        // Availability, existing identity, then the original final read. No writes here.
        for _ in 0..3 {
            let source = store.clone();
            let (next, result) = tokio::task::spawn_blocking(move || {
                let result = (|| {
                    let db = source.connection().map_err(|error| {
                        Flow::original(AnalysisError::Core(AiError::from(error).into()))
                    })?;
                    existing_generation_conversation(
                        &db,
                        prepared.resources.statements(),
                        &prepared.policy,
                        &prepared.paper,
                    )
                })();
                (prepared, result)
            })
            .await
            .map_err(|_| AiError::TemporaryUnavailable)?;
            prepared = next;
            match result {
                Ok(_) => return Ok(prepared),
                Err(Flow::Need(need)) => {
                    let Some(input) = need.input().cloned() else {
                        return Err(prepared.changed());
                    };
                    let (decision, origin) = self
                        .analysis_workflow_decision(&prepared.key, input)
                        .await
                        .map_err(template::technical)?;
                    if !prepared.policy.resolve(need, decision, origin) {
                        return Err(template::technical(
                            Self::error(
                                "plugin_host_response_mismatch",
                                "文献生成会话决定种类不匹配。",
                            )
                            .package(&prepared.key),
                        ));
                    }
                }
                Err(error) => return Err(prepared.error(error)),
            }
        }
        Err(prepared.changed())
    }
}
