//! Stable research resources. Draft persistence, publication and review share one owner.
#[path = "commands.rs"]
mod commands;
#[path = "content.rs"]
mod content;
#[path = "drafts.rs"]
mod drafts;
#[path = "history.rs"]
mod history;
#[path = "legacy.rs"]
mod legacy;
#[path = "legacy_guard.rs"]
mod legacy_guard;
#[path = "legacy_profiles.rs"]
mod legacy_profiles;
#[path = "model.rs"]
pub(crate) mod model;
#[path = "provenance.rs"]
mod provenance;
#[path = "repository.rs"]
mod repository;
#[path = "retention.rs"]
mod retention;
#[path = "review.rs"]
mod review;
#[path = "startup.rs"]
mod startup;
#[path = "templates.rs"]
mod templates;
#[cfg(test)]
#[path = "tests.rs"]
mod tests;
#[path = "validation.rs"]
mod validation;
#[path = "write_ahead.rs"]
mod write_ahead;

pub use commands::*;
#[cfg(test)]
pub(crate) use drafts::patch_with;
pub(crate) use drafts::{applied_content_for_submission, patch, query_outcome, write, write_with};
pub(crate) use history::restore_with as restore_history_with;
pub(crate) use legacy::import_legacy;
pub(crate) use legacy::import_legacy_with_evidence;
pub(crate) use legacy_guard::require_legacy_imported;
pub(crate) use model::*;
pub(crate) use repository::{
    ensure_paper_resources, find_by_paper, get, get_header, get_version, has_owned_draft, list,
    list_headers, read_text,
};
pub(crate) use review::{ReviewPolicy, ReviewRoute, review, review_with};
pub(crate) use startup::import_startup;
pub(crate) use startup::import_startup_with_evidence;
pub(crate) use templates::{
    adopt_template, adopt_template_with, create_template, ensure_paper_template,
};

#[derive(Debug, thiserror::Error)]
pub(crate) enum Error {
    #[error("结构化资源存储不可用")]
    Database(#[from] rusqlite::Error),
    #[error("{0}")]
    Invalid(String),
    #[error("找不到结构化资源")]
    NotFound,
    #[error("资源或草稿版本已变化，请重新读取后修改")]
    Conflict,
    #[error("旧文献内容尚未迁移，请先完成启动数据迁移后重试；编辑草稿已保留")]
    LegacyImportRequired,
    #[error(
        "旧文献内容尚未迁移，但正式资源已被修改。为避免遗漏旧正文，迁移已停止，请保留当前及旧版数据后进行恢复"
    )]
    LegacyImportConflict,
    #[error(transparent)]
    Evidence(Box<crate::ai::structured_resource_validation::ExplicitEvidenceError>),
}

impl From<crate::ai::structured_resource_validation::ExplicitEvidenceError> for Error {
    fn from(error: crate::ai::structured_resource_validation::ExplicitEvidenceError) -> Self {
        if error.data.is_some() {
            Self::Evidence(Box::new(error))
        } else {
            Self::Invalid(error.to_string())
        }
    }
}
impl Error {
    pub(crate) fn command_code(&self) -> &'static str {
        match self {
            Self::Conflict => "revision_conflict",
            Self::NotFound => "not_found",
            Self::Database(_) => "storage_error",
            Self::Invalid(_) | Self::Evidence(_) => "invalid_input",
            Self::LegacyImportRequired => "legacy_import_required",
            Self::LegacyImportConflict => "legacy_import_conflict",
        }
    }
}

pub(crate) fn hash(text: &str) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(text.as_bytes()))
}
fn encode<T: serde::Serialize>(value: &T) -> Result<String, Error> {
    serde_json::to_string(value).map_err(|_| Error::Invalid("无法编码结构化资源".into()))
}
fn decode<T: serde::de::DeserializeOwned>(text: &str) -> Result<T, Error> {
    serde_json::from_str(text).map_err(|_| Error::Invalid("结构化资源存储格式无效".into()))
}
#[path = "hbs.rs"]
pub(crate) mod hbs;
