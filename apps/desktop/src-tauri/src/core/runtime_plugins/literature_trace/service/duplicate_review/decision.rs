use super::*;
use crate::runtime_plugins::literature_duplicates::{DecisionSqlState, execute_decision};
use research_workbench_native_sdk::duplicate_review::{
    DuplicateDecisionCompletionV1, DuplicateDecisionPreparationV1, DuplicateDecisionRequestV1,
    FinishDuplicateDecisionInputV1,
};

impl LiteratureTraceService {
    #[cfg(unix)]
    pub(super) async fn run_duplicate_decision(
        &self,
        host: &mut NativeHost,
        database: SharedDatabase,
        resources: DuplicateResources,
        request: DuplicateDecisionRequestV1,
    ) -> NativeResult<ReviewResult> {
        let stopped = self.stopped.notified();
        tokio::pin!(stopped);
        stopped.as_mut().enable();
        self.available()?;
        let output = tokio::select! {
            biased;
            _ = &mut stopped => Err(host.error("plugin_host_stopping", "应用正在关闭重复项审核准备。")),
            output = host.catalog_call(CatalogInput::PrepareDuplicateDecision { request: request.clone() }, Duration::from_secs(60)) => output,
        }?;
        match output {
            CatalogOutput::DuplicateDecisionPreparation {
                outcome: DuplicateDecisionPreparationV1::Rejected { error },
            } => return Err(host.error(error.code.as_str(), error.message)),
            CatalogOutput::DuplicateDecisionPreparation {
                outcome: DuplicateDecisionPreparationV1::Ready { request: prepared },
            } if prepared == request => {}
            _ => {
                return Err(host.error(
                    "plugin_host_response_mismatch",
                    "重复项审核准备与原请求不匹配。",
                ));
            }
        }
        self.available()
            .map_err(|error| errors::bind(host, error))?;
        let sql_request = request.clone();
        // No cancellation select or host wait crosses this accepted write.
        let affected_rows = tokio::task::spawn_blocking(move || {
            let mut guard = database.lock().map_err(|cause| {
                let mut error = errors::owner(cause);
                error.duplicate_sql = Some(DecisionSqlState::NotDispatched);
                error
            })?;
            execute_decision(&mut guard, &resources, &sql_request)
        })
        .await
        .map_err(|_| {
            errors::bind(
                host,
                errors::unknown("重复项审核执行未能结算，禁止自动重试。", true),
            )
        })?
        .map_err(|error| errors::bind(host, error))?;
        let state = DecisionSqlState::Returned {
            affected_rows,
            observed_autocommit: true,
        };
        // Keep the completed SQL observation even if the host or finisher dies.
        let result = async {
            let output = host
                .catalog_call(
                    CatalogInput::FinishDuplicateDecision {
                        input: FinishDuplicateDecisionInputV1 {
                            request,
                            affected_rows,
                        },
                    },
                    Duration::from_secs(60),
                )
                .await?;
            match output {
                CatalogOutput::DuplicateDecisionCompletion {
                    outcome: DuplicateDecisionCompletionV1::Applied {},
                } => Ok(ReviewResult::Decided),
                CatalogOutput::DuplicateDecisionCompletion {
                    outcome: DuplicateDecisionCompletionV1::Rejected { error },
                } => Err(host.error(error.code.as_str(), error.message)),
                _ => Err(host.error(
                    "plugin_host_response_mismatch",
                    "重复项审核完成阶段不匹配。",
                )),
            }
        }
        .await;
        result.map_err(|mut error| {
            error.duplicate_sql = Some(state);
            error
        })
    }
}
