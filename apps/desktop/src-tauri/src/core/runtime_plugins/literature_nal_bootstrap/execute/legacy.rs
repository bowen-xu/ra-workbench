use super::super::{capture::first, rows::Row};
use super::*;
use crate::data_interface::plugin_resources::{ColumnCodec as C, RowPolicy as P};
use research_workbench_native_sdk::{
    nal_bootstrap::{
        ImportPreparationV1, PreparedNalStatementV1, RepairPreparationV1, SourceProbe,
    },
    nal_state::NalTerm,
};

impl Execution<'_, '_> {
    pub(super) fn import(&mut self) -> Result<()> {
        let rows = self.read_source(
            BootstrapPhaseV1::LegacyHeader,
            &self.resources.legacy_header,
            vec![],
            P::FirstRequired,
            &[C::Bool],
        )?;
        let done = first(rows)
            .and_then(|mut row| row.boolean())
            .map_err(|error| self.problem(BootstrapPhaseV1::LegacyHeader, error))?;
        let plan = self.plan;
        if done {
            return if matches!(plan.import, ImportPreparationV1::Skipped {}) {
                Ok(())
            } else {
                Err(BootstrapProblem::binding(
                    BootstrapPhaseV1::LegacyHeader,
                    "import-skip",
                ))
            };
        }
        let collections = self.read_source(
            BootstrapPhaseV1::Collections,
            &self.resources.legacy_collections,
            vec![],
            P::All,
            &[C::String, C::String, C::String, C::OptionalString],
        )?;
        let ImportPreparationV1::Prepared(data) = &plan.import else {
            return Err(BootstrapProblem::binding(
                BootstrapPhaseV1::Collections,
                "import-plan",
            ));
        };
        if !collections.rows.is_empty() {
            self.concept(&ConceptCallKeyV1::Membership {}, FailureContext::Import)?;
            self.concept(&ConceptCallKeyV1::Tagged {}, FailureContext::Import)?;
            for row in collections.rows {
                let source_id = Row::from(row)
                    .text()
                    .map_err(|error| self.problem(BootstrapPhaseV1::Collections, error))?;
                self.concept(
                    &ConceptCallKeyV1::Collection { source_id },
                    FailureContext::Import,
                )?;
            }
            if self.has_intent(&ConceptCallKeyV1::LinkedPredicate {}) {
                self.concept(
                    &ConceptCallKeyV1::LinkedPredicate {},
                    FailureContext::Import,
                )?;
            }
            self.insert_statements(BootstrapPhaseV1::Collections, &data.linked_statements)?;
            let types = self.read_source(
                BootstrapPhaseV1::RelationTypes,
                &self.resources.legacy_types,
                vec![],
                P::All,
                &[C::String, C::String, C::String, C::Bool],
            )?;
            for row in types.rows {
                let source_id = Row::from(row)
                    .text()
                    .map_err(|error| self.problem(BootstrapPhaseV1::RelationTypes, error))?;
                if let Some(probe) = data
                    .related_field_probe
                    .as_ref()
                    .filter(|probe| probe.source_id == source_id)
                {
                    let rows = self.read_source(
                        BootstrapPhaseV1::RelatedFieldExistence,
                        &self.resources.legacy_related_used,
                        vec![DataValue::Text(source_id.clone())],
                        P::FirstRequired,
                        &[C::Bool],
                    )?;
                    let used = first(rows)
                        .and_then(|mut row| row.boolean())
                        .map_err(|error| {
                            self.problem(BootstrapPhaseV1::RelatedFieldExistence, error)
                        })?;
                    if !matches!(probe.result,SourceProbe::Available{value} if value==used) {
                        return Err(BootstrapProblem::binding(
                            BootstrapPhaseV1::RelatedFieldExistence,
                            "related-field-probe",
                        ));
                    }
                }
                let key = ConceptCallKeyV1::TypePredicate { source_id };
                if self.has_intent(&key) {
                    self.concept(&key, FailureContext::Import)?;
                }
            }
            self.read_source(
                BootstrapPhaseV1::Relations,
                &self.resources.legacy_relations,
                vec![],
                P::All,
                &[C::String, C::String, C::String, C::String],
            )?;
            if self.has_intent(&ConceptCallKeyV1::SubFieldPredicate {}) {
                self.concept(
                    &ConceptCallKeyV1::SubFieldPredicate {},
                    FailureContext::Import,
                )?;
            }
            self.insert_statements(BootstrapPhaseV1::Relations, &data.relation_statements)?;
            self.read_source(
                BootstrapPhaseV1::Memberships,
                &self.resources.legacy_memberships,
                vec![],
                P::All,
                &[C::String, C::String],
            )?;
            self.insert_statements(BootstrapPhaseV1::Memberships, &data.membership_statements)?;
            self.read_source(
                BootstrapPhaseV1::Assignments,
                &self.resources.legacy_assignments,
                vec![],
                P::All,
                &[C::String, C::String],
            )?;
            self.insert_statements(BootstrapPhaseV1::Assignments, &data.assignment_statements)?;
        }
        self.write(
            BootstrapPhaseV1::ImportFlag,
            &self.resources.legacy_import_flag,
            vec![],
        )?;
        Ok(())
    }

    fn insert_statements(
        &mut self,
        phase: BootstrapPhaseV1,
        statements: &[PreparedNalStatementV1],
    ) -> Result<()> {
        for prepared in statements {
            let statement = &prepared.statement;
            if self
                .input
                .identities
                .statement_ids
                .get(self.statement_cursor)
                != Some(&statement.id)
            {
                return Err(BootstrapProblem::binding(phase, "statement-slot"));
            }
            self.statement_cursor += 1;
            if let Some(id) = &prepared.relation_concept_id {
                if !self
                    .input
                    .identities
                    .concepts
                    .iter()
                    .any(|slots| &slots.concept_id == id)
                {
                    return Err(BootstrapProblem::binding(phase, "statement-relation-slot"));
                }
                self.write(
                    phase,
                    &self.resources.statement_relation_kind,
                    vec![DataValue::Text(id.clone())],
                )?;
            }
            self.write(
                phase,
                &self.resources.statement_insert,
                vec![
                    DataValue::Text(statement.id.clone()),
                    DataValue::Text(statement.subject_json.clone()),
                    DataValue::Text(statement.predicate_json.clone()),
                    DataValue::Text(statement.origin.clone()),
                ],
            )?;
        }
        Ok(())
    }

    pub(super) fn repair(&mut self) -> Result<()> {
        let rows = self.read_source(
            BootstrapPhaseV1::RepairVersion,
            &self.resources.repair_version,
            vec![],
            P::FirstRequired,
            &[C::I64],
        )?;
        let version = first(rows)
            .and_then(|mut row| row.integer())
            .map_err(|error| self.problem(BootstrapPhaseV1::RepairVersion, error))?;
        let plan = self.plan;
        if version == 1 {
            return if matches!(plan.repair, RepairPreparationV1::Skipped {}) {
                Ok(())
            } else {
                Err(BootstrapProblem::binding(
                    BootstrapPhaseV1::RepairVersion,
                    "repair-skip",
                ))
            };
        }
        let rows = self.read_source(
            BootstrapPhaseV1::RepairRecords,
            &self.resources.repair_records,
            vec![],
            P::All,
            &[C::String, C::String, C::String],
        )?;
        let RepairPreparationV1::Prepared { relation, rewrites } = &plan.repair else {
            return Err(BootstrapProblem::binding(
                BootstrapPhaseV1::RepairRecords,
                "repair-plan",
            ));
        };
        if !rows.rows.is_empty() {
            let id = self.concept(
                &ConceptCallKeyV1::RepairPredicate {},
                FailureContext::Repair,
            )?;
            if relation.as_ref() != Some(&id) {
                return Err(BootstrapProblem::binding(
                    BootstrapPhaseV1::RepairRole,
                    "repair-relation",
                ));
            }
            self.write(
                BootstrapPhaseV1::RepairRole,
                &self.resources.repair_relation_kind,
                vec![DataValue::Text(id)],
            )?;
            for (index, row) in rows.rows.into_iter().enumerate() {
                let mut row = Row::from(row);
                let (id, subject, predicate) =
                    (|| Ok::<_, CaptureError>((row.text()?, row.text()?, row.text()?)))()
                        .map_err(|error| self.problem(BootstrapPhaseV1::RepairRecords, error))?;
                // Decode each original old fact at its original position, after
                // the relation concept/role and before that statement's UPDATE.
                serde_json::from_str::<NalTerm>(&subject)
                    .and_then(|_| serde_json::from_str::<NalTerm>(&predicate))
                    .map_err(|_| BootstrapProblem::structure(BootstrapPhaseV1::RepairTermDecode))?;
                let rewrite = rewrites
                    .get(index)
                    .filter(|rewrite| rewrite.id == id)
                    .ok_or_else(|| {
                        BootstrapProblem::binding(
                            BootstrapPhaseV1::RepairTermDecode,
                            "repair-statement",
                        )
                    })?;
                self.write(
                    BootstrapPhaseV1::RepairTermDecode,
                    &self.resources.repair_statement,
                    vec![
                        DataValue::Text(rewrite.subject_json.clone()),
                        DataValue::Text(rewrite.predicate_json.clone()),
                        DataValue::Text(id),
                    ],
                )?;
            }
        }
        self.write(
            BootstrapPhaseV1::RepairFlag,
            &self.resources.repair_flag,
            vec![],
        )?;
        Ok(())
    }
}
