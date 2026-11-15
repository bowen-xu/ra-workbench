//! Original startup recovery before the shared Database owner is exposed.
use super::*;
use crate::{
    ai::candidate_recovery,
    runtime_plugins::literature_analysis::recovery_resources::RecoveryResources,
    storage::StorageError,
};
use rusqlite::Connection;

impl LiteratureTraceService {
    /// The startup caller owns this raw connection exclusively. No DatabaseGuard,
    /// statement or transaction spans a Native wait. The original per-candidate
    /// Deferred transaction and shared-journal eligibility remain with Core.
    pub(crate) async fn recover_startup_candidates(
        &self,
        connection: &mut Connection,
        key: &str,
    ) -> Result<RecoveryResources, StorageError> {
        self.available().map_err(StorageError::NativeStartup)?;
        let _lifetime = self.doi_operations.read().await;
        self.available().map_err(StorageError::NativeStartup)?;
        let catalog = Arc::clone(&self.catalog);
        let selected = key.to_owned();
        let resources = tokio::task::spawn_blocking(move || {
            let package = package::accept_analysis_workflow(&catalog, &selected)?;
            RecoveryResources::load(&catalog, &package)
                .map_err(|error| Self::error(error.code, error.message).package(&selected))
        })
        .await
        .map_err(|_| {
            StorageError::NativeStartup(
                Self::error(
                    "plugin_host_source_worker_unsettled",
                    "文献启动恢复资源准备未确认完成。",
                )
                .package(key),
            )
        })?
        .map_err(StorageError::NativeStartup)?;
        let statements = resources.statements();
        let candidates = candidate_recovery::stranded_candidates(connection, &statements)?;
        for candidate in candidates {
            let input = WorkflowInputV1::CandidateRecovery {
                input: candidate.input(),
            };
            let (decision, _) = self
                .analysis_workflow_decision(key, input)
                .await
                .map_err(StorageError::NativeStartup)?;
            let WorkflowDecisionV1::CandidateRecovery { policy } = decision else {
                return Err(StorageError::NativeStartup(
                    Self::error(
                        "plugin_host_response_mismatch",
                        "文献启动候选恢复结果种类不匹配。",
                    )
                    .package(key),
                ));
            };
            let current = candidate_recovery::stranded_candidates(connection, &statements)?;
            if !current.iter().any(|row| row == &candidate) {
                return Err(StorageError::NativeStartup(
                    Self::error(
                        "plugin_host_source_changed",
                        "文献启动候选在核对期间变化；请重新打开后核对。",
                    )
                    .package(key),
                ));
            }
            let Some(policy) = policy else {
                continue;
            };
            if !candidate_recovery::shared_candidate_completed(
                connection,
                candidate.candidate_id(),
            )? {
                continue;
            }
            candidate_recovery::recover_candidate(connection, &candidate, &statements, &policy)?;
        }
        Ok(resources)
    }
}
