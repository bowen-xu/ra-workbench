//! Shared generation publication diagnostics retain known identities and safe SQL causes.
use crate::ai::{error::AiError, structured_resource_validation::FailureCause};
use crate::hbs_storage::boundary::BoundaryFailure;

#[derive(Debug, thiserror::Error)]
#[error(
    "AI 生成数据保存结果尚未确认，请重启应用后核对原运行 {run_id}、{object_kind} {object_id:?}；阶段：{phase}；原因：{safe_cause}；回滚：{rollback_cause:?}；原错误码：{prior_code}"
)]
pub(crate) struct GenerationStorageUnknown {
    run_id: String,
    object_kind: &'static str,
    object_id: Option<String>,
    phase: &'static str,
    safe_cause: String,
    rollback_cause: Option<String>,
    prior_code: &'static str,
}
impl GenerationStorageUnknown {
    pub(super) fn run_id(&self) -> &str {
        &self.run_id
    }

    pub(crate) fn from_failure(
        run: &str,
        object_kind: &'static str,
        object_id: Option<String>,
        failure: BoundaryFailure<AiError>,
    ) -> Self {
        let safe_cause = match &failure.failure.cause {
            FailureCause::SourceDatabase(cause) => cause.to_string(),
            _ => failure.failure.error.code().into(),
        };
        Self {
            run_id: run.into(),
            object_kind,
            object_id,
            phase: failure.phase,
            safe_cause,
            rollback_cause: failure.rollback_error.map(|cause| cause.to_string()),
            prior_code: failure.failure.error.code(),
        }
    }
    pub(crate) fn unsettled(
        run: &str,
        object_kind: &'static str,
        phase: &'static str,
        cause: &'static str,
    ) -> Self {
        Self {
            run_id: run.into(),
            object_kind,
            object_id: None,
            phase,
            safe_cause: cause.into(),
            rollback_cause: None,
            prior_code: "data_operation_outcome_unknown",
        }
    }
}
