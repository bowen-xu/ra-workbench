use super::super::{
    CompleteNalLoadError, DecodedNalState, NalLoadResources, capture::first, load_complete,
};
use super::*;
use crate::data_interface::plugin_resources::{ColumnCodec as C, RowPolicy as P};
use research_workbench_native_sdk::{
    nal_annotation::NalAnnotation,
    nal_bootstrap::{CleanupPreparationV1, StateProjectionV1},
};
use std::collections::BTreeMap;

impl Execution<'_, '_> {
    fn load_at(&self, phase: BootstrapPhaseV1) -> Result<DecodedNalState> {
        load_complete(
            NalLoadResources {
                revision: &self.resources.state_revision,
                concepts: &self.resources.state_concepts,
                statements: &self.resources.state_statements,
                annotations: &self.resources.state_annotations,
                rules: &self.resources.state_rules,
            },
            |phase, resource, policy, codecs| self.read(phase, resource, vec![], policy, codecs),
        )
        .map_err(|error| match error {
            CompleteNalLoadError::Read(error) => error,
            CompleteNalLoadError::Capture { phase, cause } => self.problem(phase, cause),
            CompleteNalLoadError::Structure(_error) => BootstrapProblem::structure(phase),
        })
    }

    pub(super) fn link_annotations(&self) -> Result<Vec<NalAnnotation>> {
        Ok(self.load_at(BootstrapPhaseV1::FinalLoad)?.raw.annotations)
    }

    pub(super) fn mutation_state(&self, rule_projection: &[NalRule]) -> Result<NalState> {
        let loaded = self.load_at(BootstrapPhaseV1::FinalLoad)?;
        let mut projection = BTreeMap::new();
        for rule in rule_projection {
            projection
                .entry(rule.relation_concept_id.as_str())
                .or_insert(rule);
        }
        let rules = loaded
            .raw
            .relation_concept_ids
            .iter()
            .map(|id| {
                projection
                    .get(id.as_str())
                    .copied()
                    .cloned()
                    .ok_or_else(|| {
                        BootstrapProblem::binding(
                            BootstrapPhaseV1::FinalLoad,
                            "mutation-rule-projection",
                        )
                    })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(NalState {
            revision: loaded.raw.revision,
            concepts: loaded.raw.concepts,
            statements: loaded.statements,
            annotations: loaded.raw.annotations,
            rules,
        })
    }

    pub(super) fn cleanup(&self) -> Result<()> {
        let rows = self.read(
            BootstrapPhaseV1::CleanupExistence,
            &self.resources.cleanup_exists,
            vec![],
            P::FirstRequired,
            &[C::Bool],
        )?;
        let exists = first(rows)
            .and_then(|mut row| row.boolean())
            .map_err(|error| self.problem(BootstrapPhaseV1::CleanupExistence, error))?;
        if !exists {
            return match self.plan.cleanup {
                CleanupPreparationV1::Skipped {} | CleanupPreparationV1::Deferred {} => Ok(()),
                _ => Err(BootstrapProblem::binding(
                    BootstrapPhaseV1::CleanupExistence,
                    "cleanup-existence",
                )),
            };
        }
        let loaded = self.load_at(BootstrapPhaseV1::CleanupLoad)?;
        let CleanupPreparationV1::Prepared {
            delete_ids,
            expected_post_repair,
        } = &self.plan.cleanup
        else {
            return Err(BootstrapProblem::binding(
                BootstrapPhaseV1::CleanupLoad,
                "cleanup-projection",
            ));
        };
        if loaded.raw != *expected_post_repair {
            return Err(BootstrapProblem::binding(
                BootstrapPhaseV1::CleanupLoad,
                "post-repair-observation",
            ));
        }
        for id in delete_ids {
            if self.write(
                BootstrapPhaseV1::CleanupDelete,
                &self.resources.concept_delete,
                vec![DataValue::Text(id.clone())],
            )? != 1
            {
                return Err(BootstrapProblem {
                    phase: BootstrapPhaseV1::CleanupDelete,
                    context: FailureContext::Network,
                    cause: BootstrapCause::MissingConcept,
                });
            }
            self.write(
                BootstrapPhaseV1::CleanupDelete,
                &self.resources.concept_search_delete,
                vec![DataValue::Text(id.clone())],
            )?;
        }
        Ok(())
    }

    pub(super) fn finish_state(&self) -> Result<NalState> {
        let loaded = self.load_at(BootstrapPhaseV1::FinalLoad)?;
        let StateProjectionV1::Prepared { raw_state, rules } = &self.plan.final_projection else {
            return Err(BootstrapProblem::binding(
                BootstrapPhaseV1::FinalLoad,
                "final-projection",
            ));
        };
        if loaded.raw != *raw_state
            || self.plan.terminal.is_some()
            || rules
                .iter()
                .map(|rule| &rule.relation_concept_id)
                .ne(loaded.raw.relation_concept_ids.iter())
        {
            return Err(BootstrapProblem::binding(
                BootstrapPhaseV1::FinalLoad,
                "final-observation",
            ));
        }
        Ok(NalState {
            revision: loaded.raw.revision,
            concepts: loaded.raw.concepts,
            statements: loaded.statements,
            annotations: loaded.raw.annotations,
            rules: rules.clone(),
        })
    }
}
