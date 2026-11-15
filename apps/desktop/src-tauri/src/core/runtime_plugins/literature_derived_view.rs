//! Original derived-view storage, shared by static composition and finite Native rules.
pub(crate) mod commands;
mod error;
mod manual;
mod repository;
mod resources;
mod validation;
use crate::{
    ai::error::AiError,
    runtime_plugins::{RuntimePluginCatalog, RuntimePluginError, RuntimePluginPackage, data},
};
pub(crate) use error::DerivedError;
use manual::settle;
pub(crate) use manual::{DerivedPolicy, DerivedRequest, DerivedResult, execute};
pub(crate) use repository::{list_derived_views_from, save_derived_view_from};
use research_workbench_native_sdk::derived_view::*;
pub(crate) use resources::DerivedSql;
use rusqlite::{Connection, Transaction, params};
use uuid::Uuid;
use validation::validate_with;
const ASSETS: [(&str, &str); 3] = [
    (
        "derived-view-ai-insert.v1.sql",
        "c70d4f3817c93acf1ca4f95321d76016c0abf075ee0183f3c712d78b7b981948",
    ),
    (
        "derived-view-ai-revision.v1.sql",
        "36270ca687038c0983041733896acf7cae5342f9366ba4e90d10e1d16e3c7cc7",
    ),
    (
        "derived-view-input-snapshot.v1.sql",
        "f71f9c15f165e0b24845760fed2421310f22b4f6ed24a7808827f24a06ed18a5",
    ),
];
pub(crate) struct DerivedAcceptanceSql([String; 3]);
impl DerivedAcceptanceSql {
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
                .expect("three derived acceptance statements"),
        ))
    }

    pub(crate) fn validate<E: From<AiError>>(
        &self,
        db: &Connection,
        draft: &DerivedResearchViewDraft,
        decide: &mut impl FnMut(DerivedValidationInputV1) -> Result<(), E>,
    ) -> Result<(), E> {
        validate_with(self.0[2].trim_end_matches('\n'), db, draft, decide)
    }
    pub(crate) fn apply<E: From<AiError>>(
        &self,
        tx: &Transaction<'_>,
        accepted: &AcceptedDerivedViewV1,
        decide: &mut impl FnMut(DerivedValidationInputV1) -> Result<(), E>,
    ) -> Result<(String, i64), E> {
        decide(DerivedValidationInputV1::Provenance {
            provenance_kind: accepted.provenance_kind.clone(),
        })?;
        let draft = DerivedResearchViewDraft {
            id: None,
            project_id: accepted.project_id.clone(),
            view_kind: accepted.view_kind.clone(),
            title: accepted.title.clone(),
            scope_note: accepted.scope_note.clone(),
            inputs: accepted.inputs.clone(),
            items: accepted.items.clone(),
            review_state: "accepted".into(),
            expected_revision: None,
            change_reason: "accepted_ai_candidate".into(),
        };
        self.validate(tx, &draft, decide)?;
        let id = Uuid::new_v4().to_string();
        tx.execute(
            self.0[0].trim_end_matches('\n'),
            params![
                id,
                draft.project_id,
                draft.view_kind,
                draft.title.trim(),
                draft.scope_note.trim(),
                json(&draft.inputs, "派生视图输入")?,
                json(&draft.items, "派生视图条目")?,
                accepted.provenance_kind,
                accepted.provider_id,
                accepted.model_id,
                accepted.generated_at
            ],
        )
        .map_err(AiError::from)?;
        let snapshot = serde_json::json!({"view": draft, "acceptedFrom": {"candidateId": accepted.candidate_id, "candidateRevision": accepted.candidate_revision}});
        let editor_kind = if accepted.provenance_kind == "ai_suggestion" {
            "ai"
        } else {
            "human"
        };
        tx.execute(
            self.0[1].trim_end_matches('\n'),
            params![
                Uuid::new_v4().to_string(),
                id,
                snapshot.to_string(),
                editor_kind
            ],
        )
        .map_err(AiError::from)?;
        Ok((id, 1))
    }
}
fn json<T: serde::Serialize + ?Sized>(value: &T, label: &str) -> Result<String, AiError> {
    serde_json::to_string(value).map_err(|_| AiError::InvalidInput(format!("{label}无法序列化")))
}
