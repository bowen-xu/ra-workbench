use crate::{
    ai::error::AiError,
    runtime_plugins::{RuntimePluginCatalog, RuntimePluginError, RuntimePluginPackage, data},
};
use research_workbench_native_sdk::reading_units::accepted::AcceptedReadingUnitV1;
use rusqlite::{Transaction, params};
use uuid::Uuid;
const ASSETS: [(&str, &str); 3] = [
    (
        "reading-unit-accepted-insert.v1.sql",
        "90e3d1280361df0ae6a81f95fed5323e0b9757b3ccabc6de98032e2a08cd1a91",
    ),
    (
        "reading-unit-accepted-evidence.v1.sql",
        "3aef45b98e7e95fded7398238b06d4561eb54376558a3ed9b0887300cf3baa60",
    ),
    (
        "nal-bootstrap-research-revision-insert.v1.sql",
        "1a31986a2e74182d3f5a70491fec1feda7cde5cd3cefd8d4ffb5723608393e9d",
    ),
];
pub(crate) struct ReadingUnitAcceptanceSql([String; 3]);
impl ReadingUnitAcceptanceSql {
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
                .expect("three reading acceptance statements"),
        ))
    }

    pub(crate) fn apply<E: From<AiError>>(
        &self,
        transaction: &Transaction<'_>,
        draft: &AcceptedReadingUnitV1,
        validate: &mut impl FnMut(&AcceptedReadingUnitV1) -> Result<(), E>,
    ) -> Result<(String, i64), E> {
        validate(draft)?;
        let title = draft.title.trim();
        let body = draft.body.trim();
        let id = Uuid::new_v4().to_string();
        transaction
            .execute(
                self.0[0].trim_end_matches('\n'),
                params![
                    id,
                    draft.project_id,
                    draft.paper_id,
                    draft.unit_kind,
                    title,
                    body,
                    draft.provenance_kind,
                    draft.epistemic_role,
                ],
            )
            .map_err(AiError::from)?;
        for anchor_id in &draft.anchor_ids {
            transaction
                .execute(
                    self.0[1].trim_end_matches('\n'),
                    params![
                        Uuid::new_v4().to_string(),
                        id,
                        anchor_id,
                        format!(
                            "AI candidate {} revision {} reviewed by human",
                            draft.candidate_id, draft.candidate_revision
                        ),
                    ],
                )
                .map_err(AiError::from)?;
        }
        let snapshot = serde_json::json!({
            "id": id,
            "projectId": draft.project_id,
            "paperId": draft.paper_id,
            "unitKind": draft.unit_kind,
            "title": title,
            "body": body,
            "provenanceKind": draft.provenance_kind,
            "epistemicRole": draft.epistemic_role,
            "revision": 1,
            "acceptedFrom": {
                "candidateId": draft.candidate_id,
                "candidateRevision": draft.candidate_revision
            }
        });
        transaction
            .execute(
                self.0[2].trim_end_matches('\n'),
                params![
                    Uuid::new_v4().to_string(),
                    "reading_unit",
                    id,
                    1,
                    snapshot.to_string(),
                    "ai_review",
                    "accepted_ai_candidate"
                ],
            )
            .map_err(AiError::from)?;
        Ok((id, 1))
    }
}
