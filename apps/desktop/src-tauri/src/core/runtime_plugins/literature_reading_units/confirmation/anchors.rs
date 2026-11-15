//! Original anchor INSERT and Core-generated identity; Native supplies literal quote interpretation.
use crate::{
    ai::error::AiError,
    runtime_plugins::{
        RuntimePluginCatalog, RuntimePluginError, RuntimePluginPackage, data::load_text,
    },
};
use research_workbench_native_sdk::{
    candidate_contracts::CandidateValidationError, paper_confirmation::field::FieldAnchorV1,
};
use rusqlite::{Transaction, params};
use uuid::Uuid;
pub(crate) struct ConfirmationAnchorSql(String);
impl ConfirmationAnchorSql {
    pub(crate) fn load(
        catalog: &RuntimePluginCatalog,
        package: &RuntimePluginPackage,
    ) -> Result<Self, RuntimePluginError> {
        load_text(
            catalog,
            package,
            "literature_library",
            "data/paper-confirmation-anchor-insert.v1.sql",
            "390a50f64984c96909a5172aa8d12f8acef57ede2ee7a74369863954a931711f",
        )
        .map(Self)
    }

    pub(crate) fn apply(
        &self,
        transaction: &Transaction<'_>,
        actions: Vec<Result<FieldAnchorV1, CandidateValidationError>>,
    ) -> Result<Vec<String>, AiError> {
        let mut anchors = Vec::new();
        for action in actions {
            match action.map_err(AiError::from)? {
                FieldAnchorV1::Existing { id } => anchors.push(id),
                FieldAnchorV1::Quoted {
                    paper_id,
                    attachment_id,
                    content_sha256,
                    page_index,
                    page_label,
                    quote,
                    prefix,
                    suffix,
                } => {
                    let id = Uuid::new_v4().to_string();
                    transaction.execute(
                        self.0.trim_end_matches('\n'),
                        params![
                            id,
                            paper_id,
                            attachment_id,
                            content_sha256,
                            page_index,
                            page_label,
                            quote,
                            prefix,
                            suffix
                        ],
                    )?;
                    anchors.push(id);
                }
            }
        }
        Ok(anchors)
    }
}
