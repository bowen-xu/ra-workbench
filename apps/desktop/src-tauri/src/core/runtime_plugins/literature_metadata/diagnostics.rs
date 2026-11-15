use super::{MetadataFailure, MetadataPhaseOutcome, MetadataProblem};
use crate::runtime_plugins::native::{NativeHostError, NativeProfile};
use research_workbench_native_sdk::metadata::MetadataRejectionV1;

impl MetadataFailure {
    pub(crate) fn native(self) -> Box<NativeHostError> {
        let (code, message) = match self.problem.as_ref() {
            MetadataProblem::Domain(MetadataRejectionV1::UnsupportedType { item_type }) => {
                ("invalid_input", format!("不支持的文献类型：{item_type}"))
            }
            MetadataProblem::Domain(MetadataRejectionV1::EmptyTitle {}) => {
                ("invalid_input", "标题不能为空".into())
            }
            MetadataProblem::Domain(MetadataRejectionV1::InvalidFields { reason }) => {
                ("invalid_input", format!("元数据确认字段无效：{reason}"))
            }
            MetadataProblem::Domain(MetadataRejectionV1::NotFound {}) => {
                ("not_found", "找不到文献或附件".into())
            }
            MetadataProblem::Connector(cause) => {
                ("connector_error", format!("DOI 元数据请求失败：{cause}"))
            }
            MetadataProblem::Data(cause) if cause.code == "plugin_data_snapshot_changed" => {
                (cause.code, cause.message.into())
            }
            MetadataProblem::Data(cause) => (
                "storage_error",
                format!("文献数据库操作失败：{}", cause.safe_cause),
            ),
            MetadataProblem::Database(cause) => {
                ("storage_error", format!("文献数据库操作失败：{cause}"))
            }
            MetadataProblem::Binding => (
                "plugin_host_response_mismatch",
                "元数据准备与本次请求不匹配。".into(),
            ),
            MetadataProblem::Owner(_) => ("storage_error", "元数据数据库操作未完成。".into()),
        };
        let message = if self.outcome == MetadataPhaseOutcome::Unknown {
            format!("元数据操作结果未知，禁止自动重试。{message}")
        } else {
            message
        };
        let mut error =
            NativeHostError::for_profile(NativeProfile::NoteTraceDecision, code, message);
        if let MetadataProblem::Data(cause) = self.problem.as_ref() {
            error.data_error = Some(cause.clone());
        }
        error.literature_metadata = Some(Box::new(self));
        error
    }
}
