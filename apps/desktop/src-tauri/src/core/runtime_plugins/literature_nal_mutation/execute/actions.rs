use super::*;
use crate::runtime_plugins::literature_nal_annotation;
use research_workbench_native_sdk::{
    nal_mutation::{
        NalConceptKindPreparation, NalMutationPlan, NalTransitivityAction, NalUsageCondition,
    },
    nal_state::NalTerm,
};

impl Execution<'_, '_> {
    pub(super) fn action(&self, plan: &NalMutationPlan) -> Result<()> {
        use NalMutation as M;
        use NalMutationPlan as P;
        match (self.mutation, plan) {
            (
                M::DeleteConcept { id: original },
                P::DeleteConcept {
                    id,
                    missing_rejection,
                },
            ) if id == original => {
                if self.write(
                    Phase::Mutation,
                    self.resources.bootstrap().mutation_concept_delete(),
                    vec![DataValue::Text(id.clone())],
                )? != 1
                {
                    return Err(MutationProblem::rejected(
                        Phase::Mutation,
                        *missing_rejection,
                    ));
                }
                self.write(
                    Phase::Mutation,
                    self.resources.bootstrap().mutation_concept_search_delete(),
                    vec![DataValue::Text(id.clone())],
                )?;
            }
            (
                M::SetConceptKind {
                    concept_id: original,
                    concept_kind: original_kind,
                },
                P::SetConceptKind {
                    concept_id,
                    term,
                    after_term,
                },
            ) if concept_id == original => {
                self.term(
                    &NalTerm::Concept {
                        id: original.clone(),
                    },
                    term,
                )?;
                let (kind, on_used) = match after_term {
                    NalConceptKindPreparation::Rejected { reason } => {
                        return Err(MutationProblem::rejected(Phase::Mutation, *reason));
                    }
                    NalConceptKindPreparation::ReadUsage {
                        concept_kind,
                        on_used,
                    } if concept_kind == original_kind => (concept_kind, on_used),
                    _ => return Err(MutationProblem::binding(Phase::Mutation, "concept-kind")),
                };
                // Continue means the result does not reject; it never grants
                // permission to skip the original used query.
                let used = self.boolean(
                    Phase::KindUsage,
                    &self.resources.concept_used,
                    vec![DataValue::Text(concept_id.clone())],
                )?;
                if used && let NalUsageCondition::Reject { reason } = on_used {
                    return Err(MutationProblem::rejected(Phase::KindUsage, *reason));
                }
                self.write(
                    Phase::Mutation,
                    &self.resources.kind_upsert,
                    vec![
                        DataValue::Text(concept_id.clone()),
                        DataValue::Text(kind.clone()),
                    ],
                )?;
            }
            (
                M::SetTransitivity {
                    relation_concept_id: original,
                    enabled,
                },
                P::SetTransitivity {
                    relation_concept_id,
                    term,
                    action,
                },
            ) if relation_concept_id == original => {
                self.term(
                    &NalTerm::Concept {
                        id: original.clone(),
                    },
                    term,
                )?;
                if !matches!(
                    (enabled, action),
                    (true, NalTransitivityAction::Enable { .. })
                        | (false, NalTransitivityAction::Disable {})
                ) {
                    return Err(MutationProblem::binding(
                        Phase::Mutation,
                        "transitivity-action",
                    ));
                }
                // Disabled rules also perform this query at the original point.
                let relation = self.boolean(
                    Phase::RelationKind,
                    &self.resources.relation_kind,
                    vec![DataValue::Text(relation_concept_id.clone())],
                )?;
                let resource = match action {
                    NalTransitivityAction::Enable { missing_rejection } => {
                        if !relation {
                            return Err(MutationProblem::rejected(
                                Phase::RelationKind,
                                *missing_rejection,
                            ));
                        }
                        &self.resources.rule_insert
                    }
                    NalTransitivityAction::Disable {} => &self.resources.rule_delete,
                };
                self.write(
                    Phase::Mutation,
                    resource,
                    vec![DataValue::Text(relation_concept_id.clone())],
                )?;
            }
            (M::AddStatements { statements }, P::AddStatements { batch }) => {
                self.statements(statements, batch)?
            }
            (
                M::DeleteStatement { id: original },
                P::DeleteStatement {
                    id,
                    missing_rejection,
                },
            ) if id == original => {
                if self.write(
                    Phase::Mutation,
                    &self.resources.statement_delete,
                    vec![DataValue::Text(id.clone())],
                )? == 0
                {
                    return Err(MutationProblem::rejected(
                        Phase::Mutation,
                        *missing_rejection,
                    ));
                }
            }
            (
                M::LinkAnnotation {
                    annotation: original,
                },
                P::LinkAnnotation { annotation, link },
            ) if annotation == original => {
                literature_nal_annotation::apply_link_in(
                    self.tx,
                    &self.resources.annotation,
                    self.captured,
                    &self.input.bootstrap,
                    &self.plan.bootstrap,
                    annotation,
                    link,
                )
                .map_err(|problem| MutationProblem {
                    phase: Phase::Link,
                    cause: Cause::Link(Box::new(problem)),
                })?;
            }
            (M::UnlinkAnnotation { id: original }, P::UnlinkAnnotation { id })
                if id == original =>
            {
                self.write(
                    Phase::Mutation,
                    &self.resources.annotation_delete,
                    vec![DataValue::Text(id.clone())],
                )?;
            }
            _ => {
                return Err(MutationProblem::binding(
                    Phase::Mutation,
                    "mutation-variant",
                ));
            }
        }
        Ok(())
    }
}
