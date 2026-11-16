use super::*;
use crate::ai::{
    candidate_contracts::AiEvidenceBundle,
    error::AiError,
    structured_resource_validation::{DecisionOrigin, Failure, FailureCause, ValidationSources},
};
use crate::hbs_storage::boundary::HbsNeed;
use serde_json::{Value, json};

#[derive(Default)]
pub(super) struct Policy {
    pub selection_pause: bool,
    pub validation_pause: bool,
    pub reject: bool,
    pub selected: Vec<UnreviewedSelectionInputV1>,
    pub validated: usize,
}
impl StructuredPolicy for Policy {
    fn validate_editable_template(
        &mut self,
        value: Value,
    ) -> Result<Value, Flow<crate::ai::error::AiError, Self::Pause>> {
        Ok(value)
    }
    type Pause = HbsNeed;
    fn validate(
        &mut self,
        kind: &str,
        value: Value,
        evidence: &AiEvidenceBundle,
    ) -> Result<Value, Flow<AiError, HbsNeed>> {
        self.validated += 1;
        if self.validation_pause {
            return Err(Flow::Need(HbsNeed::Structured {
                slot: 0,
                input: research_workbench_native_sdk::structured_validation::StructuredValidationInputV1 {
                    target: if kind == "portrait" {
                        research_workbench_native_sdk::structured_validation::StructuredTargetV1::Portrait
                    } else {
                        research_workbench_native_sdk::structured_validation::StructuredTargetV1::Overview
                    },
                    candidate_json: value.to_string(),
                    evidence: evidence.items.iter().map(|item| research_workbench_native_sdk::candidate_contracts::EvidenceQuote {
                        citation_key:item.citation_key.clone(),quote_exact:item.quote_exact.clone(),
                    }).collect(),
                },
            }));
        }
        if self.reject {
            return Err(Flow::Fail(Failure {
                error: AiError::InvalidInput("合成领域拒绝".into()),
                cause: FailureCause::NativeRejection(Box::new(DecisionOrigin {
                    package_key: "synthetic-package".into(),
                    instance_id: "synthetic-instance".into(),
                    native_request_id: "synthetic-native-request".into(),
                })),
            }));
        }
        Ok(crate::hbs_storage::test_sources::accepted_response(value))
    }
}
impl ReviewPolicy for Policy {
    fn select_unreviewed(
        &mut self,
        input: UnreviewedSelectionInputV1,
    ) -> Result<Vec<String>, HbsNeed> {
        self.selected.push(input.clone());
        if self.selection_pause {
            return Err(HbsNeed::Unreviewed { slot: 0, input });
        }
        Ok(crate::hbs_storage::test_sources::unreviewed_selection(
            &input,
        ))
    }
}
pub(super) struct Sources {
    pub policy: Policy,
    pub ready: bool,
    pub requires: usize,
    reading: crate::hbs_storage::test_sources::ReadingEvidence,
}
impl Sources {
    pub fn new() -> Self {
        Self {
            policy: Policy::default(),
            ready: true,
            requires: 0,
            reading: crate::hbs_storage::test_sources::ReadingEvidence,
        }
    }
}
impl HbsSources for Sources {
    type Policy = Policy;
    fn policy_only(&mut self) -> Result<&mut Policy, Flow<String, HbsNeed>> {
        Ok(&mut self.policy)
    }
    fn require(&mut self) -> Result<ValidationSources<'_, Policy>, Flow<String, HbsNeed>> {
        self.requires += 1;
        if !self.ready {
            return Err(Flow::Need(HbsNeed::Sources));
        }
        Ok(ValidationSources {
            policy: &mut self.policy,
            reading: &self.reading,
        })
    }
}
pub(super) fn database() -> Connection {
    let mut db = Connection::open_in_memory().unwrap();
    crate::storage::migrate(&mut db).unwrap();
    db.execute("insert into bibliographic_items(id,item_type,title,created_at,updated_at) values('review-paper','journalArticle','Synthetic','now','now')",[]).unwrap();
    db
}
pub(super) fn body(text: &str) -> Value {
    json!({"kind":"paper_overview_unit_v1","schemaVersion":1,"profileId":"generic_overview_v1","fields":[
        {"field":"research_question","status":"filled","statements":[{"id":"question-1","text":text,"epistemicRole":"uncertain","evidenceItemIds":[]}]},
        {"field":"approach","status":"not_found","statements":[]},{"field":"key_result","status":"not_found","statements":[]},{"field":"conclusion","status":"not_found","statements":[]},{"field":"limitation","status":"not_found","statements":[]},{"field":"open_question","status":"not_found","statements":[]}
    ]})
}
pub(super) fn save(
    db: &mut Connection,
    resource: &Resource,
    content: String,
    editor: &str,
    request: &str,
) -> crate::structured_resources::WriteResult {
    crate::hbs_storage::test_sources::write(
        db,
        crate::structured_resources::WriteDraft {
            resource_id: resource.id.clone(),
            expected_revision: resource.revision,
            content,
            request_id: request.into(),
            conversation_id: None,
            expected_draft_revision: None,
            source_snapshot_id: None,
            submission_id: None,
            editor_kind: editor.into(),
        },
    )
    .unwrap()
}
pub(super) fn decision(
    db: &mut Connection,
    resource: &Resource,
    action: &str,
    ids: Option<Vec<String>>,
) -> Resource {
    crate::hbs_storage::test_sources::review(
        db,
        ReviewDraft {
            action: action.into(),
            field_ids: ids,
            scope: None,
            ..request(resource, &uuid::Uuid::new_v4().to_string())
        },
    )
    .unwrap()
}
pub(super) fn request(resource: &Resource, id: &str) -> ReviewDraft {
    ReviewDraft {
        resource_id: resource.id.clone(),
        expected_revision: resource.revision,
        request_id: id.into(),
        action: "keep".into(),
        field_ids: None,
        scope: Some("unreviewed".into()),
    }
}
pub(super) fn reading(db: &Connection) -> Resource {
    crate::structured_resources::find_by_paper(db, "review-paper", "structuredReading").unwrap()
}
pub(super) fn snapshot(db: &Connection, id: &str) -> Value {
    let resource = get(db, id).unwrap();
    let repair_draft = repository::get_draft(db, id, None).unwrap();
    let protected: String = db
        .query_row(
            "select protected_fields_json from structured_resources where id=?1",
            [id],
            |row| row.get(0),
        )
        .unwrap();
    let revisions: i64 = db
        .query_row(
            "select count(*) from structured_resource_revisions where resource_id=?1",
            [id],
            |row| row.get(0),
        )
        .unwrap();
    let receipts: i64 = db
        .query_row(
            "select count(*) from structured_resource_receipts where resource_id=?1",
            [id],
            |row| row.get(0),
        )
        .unwrap();
    let sources: i64 = db
        .query_row(
            "select count(*) from structured_resource_field_sources where resource_id=?1",
            [id],
            |row| row.get(0),
        )
        .unwrap();
    json!({"resource":resource,"repairDraft":repair_draft,"protected":protected,"revisions":revisions,"receipts":receipts,"sources":sources})
}
