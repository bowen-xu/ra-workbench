//! Fixed AI reference writes borrow the original review transaction; never commit here.
use crate::runtime_plugins::{
    RuntimePluginCatalog, RuntimePluginError, RuntimePluginPackage, data,
};
use research_workbench_native_sdk::candidate_review::reference::ReferencePromotionPlanV1;
use rusqlite::{Transaction, params};
use uuid::Uuid;

const ASSETS: [(&str, &str); 3] = [
    (
        "reference-records-ai-insert.v1.sql",
        "754d28131b84e49f21ac2a8442c8f966139b0925670b927d5a3e2988d5789426",
    ),
    (
        "reference-records-ai-revision.v1.sql",
        "e7e5ff6830d77f456fcf000aa85151ea33a7ffc7c50ed910fe4599b10f631e92",
    ),
    (
        "reference-records-anchor-belongs.v1.sql",
        "61ecf08567083debd6e54a8563fef4db418e460ed028c6fafd38637f8574a9ad",
    ),
];
pub(crate) struct ReferenceAcceptanceSql([String; 3]);
impl ReferenceAcceptanceSql {
    pub(crate) fn load(
        catalog: &RuntimePluginCatalog,
        package: &RuntimePluginPackage,
    ) -> Result<Self, RuntimePluginError> {
        let mut statements = Vec::with_capacity(3);
        for (name, hash) in ASSETS {
            statements.push(data::load_text(
                catalog,
                package,
                "literature_library",
                &format!("data/{name}"),
                hash,
            )?);
        }
        Ok(Self(
            statements
                .try_into()
                .expect("three fixed acceptance assets"),
        ))
    }

    pub(crate) fn apply(
        &self,
        tx: &Transaction<'_>,
        plan: &ReferencePromotionPlanV1,
    ) -> Result<(String, i64), ReferenceAcceptanceError> {
        let draft = &plan.record.draft;
        let belongs: bool = tx.query_row(
            self.0[2].trim_end_matches('\n'),
            params![draft.source_anchor_id, draft.citing_paper_id],
            |row| row.get(0),
        )?;
        if !belongs {
            return Err(ReferenceAcceptanceError::AnchorOwnership);
        }
        let id = Uuid::new_v4().to_string();
        tx.execute(
            self.0[0].trim_end_matches('\n'),
            params![
                id,
                draft.citing_paper_id,
                draft.source_anchor_id,
                plan.record.raw_text,
                plan.record.fields_json,
                draft.parse_state,
                plan.provenance_kind
            ],
        )?;
        tx.execute(
            self.0[1].trim_end_matches('\n'),
            params![Uuid::new_v4().to_string(), id, plan.record.snapshot_json],
        )?;
        Ok((id, 1))
    }
}
#[derive(Debug, thiserror::Error)]
pub(crate) enum ReferenceAcceptanceError {
    #[error(transparent)]
    Database(#[from] rusqlite::Error),
    #[error("参考条目来源必须属于当前文献")]
    AnchorOwnership,
}
