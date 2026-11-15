use super::*;
use crate::runtime_plugins::literature_nal_bootstrap::{
    BootstrapCause, BootstrapProblem, CaptureError, CompleteNalLoadError, DecodedNalState,
    FailureContext, load_complete, merge_import,
};
use research_workbench_native_sdk::{
    duplicate_merge::{
        CollectionAssignment, CollectionMembership, MergeClassificationSnapshot, MergeNalSnapshot,
    },
    nal_annotation::NalAnnotation,
    nal_bootstrap::BootstrapPhaseV1,
    nal_state::NalStatement,
};
use serde::Serialize;

impl Execution<'_, '_> {
    pub(super) fn classification(&self) -> Result<MergeClassificationSnapshot> {
        let nal = self.nal()?;
        let (memberships, assignments) = self.classification_state()?;
        let r = &self.r.inner;
        let survivor = &self.captured.input.survivor_item_id;
        let before_memberships = load::scope(
            self.tx,
            &r.collection_scope,
            &memberships,
            survivor,
            Phase::Classification,
        )?;
        let before_assignments = load::scope(
            self.tx,
            &r.collection_scope,
            &assignments,
            survivor,
            Phase::Classification,
        )?;
        self.write(
            Phase::Classification,
            &r.copy_memberships,
            vec![load::text(survivor), load::text(self.source)],
        )?;
        self.write(
            Phase::Classification,
            &r.copy_assignments,
            vec![load::text(survivor), load::text(self.source)],
        )?;
        self.write(
            Phase::Classification,
            &r.collection_revision_increment,
            vec![],
        )?;
        let (memberships, assignments) = self.classification_state()?;
        Ok(MergeClassificationSnapshot {
            nal: Some(nal),
            before_memberships,
            before_assignments,
            after_memberships: load::scope(
                self.tx,
                &r.collection_scope,
                &memberships,
                survivor,
                Phase::Classification,
            )?,
            after_assignments: load::scope(
                self.tx,
                &r.collection_scope,
                &assignments,
                survivor,
                Phase::Classification,
            )?,
        })
    }

    fn classification_state(
        &self,
    ) -> Result<(Vec<CollectionMembership>, Vec<CollectionAssignment>)> {
        let r = &self.r.inner;
        let phase = Phase::Classification;
        self.read(
            phase,
            &r.collection_revision,
            vec![],
            P::FirstRequired,
            &[C::I64],
        )?;
        self.read(
            phase,
            &r.collection_collections,
            vec![],
            P::All,
            &[C::String, C::String, C::String, C::OptionalString],
        )?;
        self.read(
            phase,
            &r.collection_types,
            vec![],
            P::All,
            &[C::String, C::String, C::String, C::Bool, C::Bool],
        )?;
        self.read(
            phase,
            &r.collection_relations,
            vec![],
            P::All,
            &[C::String; 4],
        )?;
        let memberships = self
            .read(
                phase,
                &r.collection_memberships,
                vec![],
                P::All,
                &[C::String; 2],
            )?
            .rows
            .into_iter()
            .map(|row| match row.as_slice() {
                [V::Text(item), V::Text(collection)] => Ok(CollectionMembership {
                    item_id: item.clone(),
                    collection_id: collection.clone(),
                }),
                _ => Err(MergeProblem::binding(phase, "membership-pair")),
            })
            .collect::<Result<Vec<_>>>()?;
        let assignments = self
            .read(
                phase,
                &r.collection_assignments,
                vec![],
                P::All,
                &[C::String; 2],
            )?
            .rows
            .into_iter()
            .map(|row| match row.as_slice() {
                [V::Text(item), V::Text(relation)] => Ok(CollectionAssignment {
                    item_id: item.clone(),
                    relation_id: relation.clone(),
                }),
                _ => Err(MergeProblem::binding(phase, "assignment-pair")),
            })
            .collect::<Result<Vec<_>>>()?;
        Ok((memberships, assignments))
    }

    fn nal_complete(&self) -> Result<DecodedNalState> {
        load_complete(
            self.r.bootstrap.state_resources(),
            |phase, resource, policy, codecs| {
                crate::data_interface::plugin_resources::read_on_connection(
                    self.tx,
                    resource,
                    &load::call(resource, vec![]),
                    policy,
                    codecs,
                )
                .map_err(|e| BootstrapProblem {
                    phase,
                    context: FailureContext::Network,
                    cause: BootstrapCause::Capture(CaptureError::Data(e)),
                })
            },
        )
        .map_err(|e| {
            let problem = match e {
                CompleteNalLoadError::Read(e) => e,
                CompleteNalLoadError::Capture { phase, cause } => BootstrapProblem {
                    phase,
                    context: FailureContext::Network,
                    cause: BootstrapCause::Capture(cause),
                },
                CompleteNalLoadError::Structure(_) => BootstrapProblem {
                    phase: BootstrapPhaseV1::FinalLoad,
                    context: FailureContext::Network,
                    cause: BootstrapCause::InvalidStructure,
                },
            };
            MergeProblem {
                phase: Phase::Nal,
                cause: MergeCause::Nal(Box::new(problem)),
            }
        })
    }

    fn nal_scope(&self, paper: &str) -> Result<(Vec<NalStatement>, Vec<NalAnnotation>)> {
        let actual = self.nal_complete()?;
        let statements: Vec<NalStatement> = load::scope(
            self.tx,
            &self.r.inner.statement_scope,
            &actual.statements,
            paper,
            Phase::Nal,
        )?;
        let annotations = load::scope(
            self.tx,
            &self.r.inner.annotation_scope,
            &actual.raw.annotations,
            &self.nal_json(&statements)?,
            Phase::Nal,
        )?;
        Ok((statements, annotations))
    }

    fn nal_json(&self, value: &impl Serialize) -> Result<String> {
        serde_json::to_string(value).map_err(|_| MergeProblem {
            phase: Phase::Nal,
            cause: MergeCause::Nal(Box::new(BootstrapProblem {
                phase: BootstrapPhaseV1::FinalLoad,
                context: FailureContext::Network,
                cause: BootstrapCause::InvalidStructure,
            })),
        })
    }

    fn nal(&self) -> Result<MergeNalSnapshot> {
        use load::text;
        merge_import::import_in(self.tx, &self.r.bootstrap, &self.r.rules).map_err(|e| {
            MergeProblem {
                phase: Phase::Nal,
                cause: MergeCause::Nal(Box::new(e)),
            }
        })?;
        let survivor = &self.captured.input.survivor_item_id;
        let (before, before_annotations) = self.nal_scope(survivor)?;
        let (source_statements, source_notes) = self.nal_scope(self.source)?;
        let r = &self.r.inner;
        for statement in source_statements {
            let id = Uuid::new_v4().to_string();
            let subject = self.nal_json(&statement.subject)?;
            let predicate = self.nal_json(&statement.predicate)?;
            let subject = load::one(
                self.read(
                    Phase::Nal,
                    &self.r.rules.legacy_replace_paper,
                    vec![V::Text(subject), text(self.source), text(survivor)],
                    P::FirstRequired,
                    &[C::String],
                )?,
                Phase::Nal,
            )?;
            let predicate = load::one(
                self.read(
                    Phase::Nal,
                    &self.r.rules.legacy_replace_paper,
                    vec![V::Text(predicate), text(self.source), text(survivor)],
                    P::FirstRequired,
                    &[C::String],
                )?,
                Phase::Nal,
            )?;
            let policy = self.read(
                Phase::Nal,
                &self.r.rules.legacy_nal_insert_policy,
                vec![text(&subject), text(&predicate), text(&statement.origin)],
                P::FirstRequired,
                &[C::OptionalString],
            )?;
            match policy.rows.first().map(Vec::as_slice) {
                Some([V::Text(relation)]) => {
                    self.write(Phase::Nal, &r.statement_relation_kind, vec![text(relation)])?
                }
                Some([V::Null]) => (),
                _ => return Err(MergeProblem::binding(Phase::Nal, "insert-policy-row")),
            }
            self.write(
                Phase::Nal,
                &r.statement_insert,
                vec![
                    V::Text(id),
                    text(&subject),
                    text(&predicate),
                    V::Text(statement.origin),
                ],
            )?;
            let target = load::one(
                self.read(
                    Phase::Nal,
                    &r.nal_target,
                    vec![V::Text(subject), V::Text(predicate)],
                    P::FirstRequired,
                    &[C::String],
                )?,
                Phase::Nal,
            )?;
            // Observe the actual conflict target before selecting/copying notes.
            let notes = load::strings(
                self.read(
                    Phase::Nal,
                    &r.nal_annotation_notes,
                    vec![
                        V::Text(self.nal_json(&source_notes)?),
                        V::Text(statement.id),
                    ],
                    P::All,
                    &[C::String],
                )?,
                Phase::Nal,
            )?;
            for note in notes {
                self.write(
                    Phase::Nal,
                    &r.insert_annotation,
                    vec![
                        V::Text(Uuid::new_v4().to_string()),
                        text(&target),
                        V::Text(note),
                    ],
                )?;
            }
        }
        let (after, after_annotations) = self.nal_scope(survivor)?;
        self.write(Phase::Nal, &r.nal_revision_increment, vec![])?;
        Ok(MergeNalSnapshot {
            before,
            after,
            before_annotations,
            after_annotations,
        })
    }
}
