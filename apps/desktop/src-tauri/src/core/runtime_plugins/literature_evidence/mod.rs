//! Original evidence inventory used by the retained project matrix component.
pub(crate) mod commands;
mod read;
mod resources;
use crate::runtime_plugins::native::{NativeHostError, NativeProfile, SafeDatabaseError};
pub(crate) use read::source_anchor_from;
pub(crate) use read::{list_claim_evidence_from, list_claims_from, list_source_anchors_from};
use research_workbench_native_sdk::evidence::{Claim, EvidenceLink, SourceAnchorSummary};
pub(crate) use resources::EvidenceReadSql;

#[derive(Debug, thiserror::Error)]
pub(crate) enum EvidenceReadError {
    #[error("证据数据库操作失败")]
    Database(#[from] rusqlite::Error),
    #[error("找不到观点或原文锚点")]
    NotFound,
}
impl EvidenceReadError {
    pub(crate) fn native(self) -> Box<NativeHostError> {
        let (code, message) = match self {
            Self::Database(cause) => (
                "storage_error",
                format!(
                    "证据数据读取失败：{}",
                    SafeDatabaseError::sql("read", &cause)
                ),
            ),
            Self::NotFound => ("not_found", "找不到观点或原文锚点".into()),
        };
        NativeHostError::for_profile(NativeProfile::NoteTraceDecision, code, message)
    }
}
pub(crate) enum EvidenceReadRequest {
    Claims { project_id: Option<String> },
    Sources { project_id: Option<String> },
    Links { claim_id: String },
}
#[derive(Debug)]
pub(crate) enum EvidenceReadResult {
    Claims(Vec<Claim>),
    Sources(Vec<SourceAnchorSummary>),
    Links(Vec<EvidenceLink>),
}
pub(crate) fn execute(
    sql: &EvidenceReadSql,
    db: &rusqlite::Connection,
    request: EvidenceReadRequest,
) -> Result<EvidenceReadResult, EvidenceReadError> {
    match request {
        EvidenceReadRequest::Claims { project_id } => {
            list_claims_from(sql, db, project_id.as_deref()).map(EvidenceReadResult::Claims)
        }
        EvidenceReadRequest::Sources { project_id } => {
            list_source_anchors_from(sql, db, project_id.as_deref())
                .map(EvidenceReadResult::Sources)
        }
        EvidenceReadRequest::Links { claim_id } => {
            list_claim_evidence_from(sql, db, &claim_id).map(EvidenceReadResult::Links)
        }
    }
}
