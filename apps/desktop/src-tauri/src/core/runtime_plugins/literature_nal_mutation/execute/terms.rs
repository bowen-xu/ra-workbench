use super::*;
use crate::data_interface::plugin_resources::{ColumnCodec, RowPolicy, read_on_connection};
use research_workbench_native_sdk::{nal_mutation::NalTermPreparation, nal_state::NalTerm};

impl Execution<'_, '_> {
    pub(super) fn term(&self, original: &NalTerm, plan: &NalTermPreparation) -> Result<()> {
        use NalTermPreparation as P;
        match (original, plan) {
            (_, P::Rejected { reason }) => Err(MutationProblem::rejected(Phase::TermRead, *reason)),
            (NalTerm::Product { terms: original }, P::Product { terms }) => {
                for (index, term) in terms.iter().enumerate() {
                    let source = original.get(index).ok_or_else(|| {
                        MutationProblem::binding(Phase::TermRead, "term-position")
                    })?;
                    self.term(source, term)?;
                }
                // A rejecting prefix returns above; a successful prefix must
                // include every original child, even when later ones are bad.
                if original.len() != terms.len() {
                    return Err(MutationProblem::binding(Phase::TermRead, "term-tail"));
                }
                Ok(())
            }
            (
                NalTerm::Concept { id: original },
                P::Concept {
                    id,
                    missing_rejection,
                },
            ) if id == original => {
                if !self.boolean(
                    Phase::TermRead,
                    &self.resources.term_concept,
                    vec![DataValue::Text(id.clone())],
                )? {
                    return Err(MutationProblem::rejected(
                        Phase::TermRead,
                        *missing_rejection,
                    ));
                }
                Ok(())
            }
            (
                NalTerm::Paper { id: original },
                P::Paper {
                    id,
                    missing_rejection,
                },
            ) if id == original => {
                if !self.boolean(
                    Phase::TermRead,
                    &self.resources.term_paper,
                    vec![DataValue::Text(id.clone())],
                )? {
                    return Err(MutationProblem::rejected(
                        Phase::TermRead,
                        *missing_rejection,
                    ));
                }
                Ok(())
            }
            _ => Err(MutationProblem::binding(Phase::TermRead, "term-identity")),
        }
    }

    pub(super) fn boolean(
        &self,
        phase: Phase,
        resource: &AcceptedDataResource,
        arguments: Vec<DataValue>,
    ) -> Result<bool> {
        let rows = read_on_connection(
            self.tx,
            resource,
            &call(resource, arguments),
            RowPolicy::FirstRequired,
            &[ColumnCodec::Bool],
        )
        .map_err(|error| MutationProblem {
            phase,
            cause: Cause::Data(error),
        })?;
        match rows.rows.first().and_then(|row| row.first()) {
            Some(DataValue::Integer(value)) => Ok(*value != 0),
            _ => Err(MutationProblem::binding(phase, "boolean-result")),
        }
    }
}
