use super::*;
use research_workbench_native_sdk::{
    duplicate_merge::{CollectionAssignment, CollectionMembership},
    nal_annotation::NalAnnotation,
    nal_state::NalStatement,
};

impl Execution<'_, '_> {
    pub(super) fn inner(&self) -> Result<()> {
        let Some(plan) = &self.plan.classification else {
            return Ok(());
        };
        let r = self.resources;
        let survivor = &self.captured.survivor_item_id;
        let (memberships, assignments) = load::classifications(self.tx, r)?;
        let memberships: Vec<CollectionMembership> = load::scope(
            self.tx,
            &r.collection_scope,
            &load::encode(&memberships, Phase::ClassificationCheck)?,
            survivor,
            Phase::ClassificationCheck,
        )?;
        let assignments: Vec<CollectionAssignment> = load::scope(
            self.tx,
            &r.collection_scope,
            &load::encode(&assignments, Phase::ClassificationCheck)?,
            survivor,
            Phase::ClassificationCheck,
        )?;
        if !load::equal(
            self.tx,
            r,
            &memberships,
            &plan.expected_memberships_json,
            Phase::ClassificationCheck,
        )? || !load::equal(
            self.tx,
            r,
            &assignments,
            &plan.expected_assignments_json,
            Phase::ClassificationCheck,
        )? {
            return Err(UndoProblem {
                phase: Phase::ClassificationCheck,
                cause: UndoCause::Conflict,
            });
        }
        if let Some(plan) = &plan.nal {
            let (statements, annotations) = load::nal(self.tx, r)?;
            let statements: Vec<NalStatement> = load::scope(
                self.tx,
                &r.statement_scope,
                &load::encode(&statements, Phase::NalCheck)?,
                survivor,
                Phase::NalCheck,
            )?;
            let statements_json = load::encode(&statements, Phase::NalCheck)?;
            let annotations: Vec<NalAnnotation> = load::scope(
                self.tx,
                &r.annotation_scope,
                &load::encode(&annotations, Phase::NalCheck)?,
                &statements_json,
                Phase::NalCheck,
            )?;
            if !load::equal(
                self.tx,
                r,
                &statements,
                &plan.expected_statements_json,
                Phase::NalCheck,
            )? || !load::equal(
                self.tx,
                r,
                &annotations,
                &plan.expected_annotations_json,
                Phase::NalCheck,
            )? {
                return Err(UndoProblem {
                    phase: Phase::NalCheck,
                    cause: UndoCause::Conflict,
                });
            }
            for id in &plan.delete_statement_ids {
                self.write(Phase::NalDelete, &r.delete_statement, vec![load::text(id)])?;
            }
            for id in &plan.delete_annotation_ids {
                self.write(Phase::NalDelete, &r.delete_annotation, vec![load::text(id)])?;
            }
            self.write(Phase::NalRevision, &r.increment_nal_revision, vec![])?;
        }
        for assignment in &plan.delete_assignments {
            self.write(
                Phase::ClassificationDelete,
                &r.delete_assignment,
                vec![load::text(survivor), load::text(&assignment.relation_id)],
            )?;
        }
        for membership in &plan.delete_memberships {
            self.write(
                Phase::ClassificationDelete,
                &r.delete_membership,
                vec![load::text(survivor), load::text(&membership.collection_id)],
            )?;
        }
        self.write(
            Phase::ClassificationRevision,
            &r.increment_collection_revision,
            vec![],
        )
    }
}
