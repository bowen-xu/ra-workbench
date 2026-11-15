use super::*;
use research_workbench_native_sdk::{
    nal_mutation::{NalStatementBatchPreparation, NalStatementPreparation},
    nal_state::NalStatement,
};

impl Execution<'_, '_> {
    pub(super) fn statements(
        &self,
        original: &[NalStatement],
        plan: &NalStatementBatchPreparation,
    ) -> Result<()> {
        let statements = match plan {
            NalStatementBatchPreparation::Rejected { reason } => {
                return Err(MutationProblem::rejected(Phase::Mutation, *reason));
            }
            NalStatementBatchPreparation::Prepared { statements } => statements,
        };
        for (index, prepared) in statements.iter().enumerate() {
            let source = original
                .get(index)
                .ok_or_else(|| MutationProblem::binding(Phase::Mutation, "statement-position"))?;
            self.statement(source, prepared)?;
            // The actual INSERT above precedes the next statement's identity,
            // term checks and any later preparation rejection.
        }
        if statements.len() != original.len() {
            return Err(MutationProblem::binding(Phase::Mutation, "statement-tail"));
        }
        Ok(())
    }

    fn statement(&self, source: &NalStatement, plan: &NalStatementPreparation) -> Result<()> {
        use NalStatementPreparation as P;
        let (id, origin) = match plan {
            P::RejectedIdentity { id, origin, .. }
            | P::SubjectRejected { id, origin, .. }
            | P::PredicateRejected { id, origin, .. }
            | P::Ready { id, origin, .. } => (id, origin),
        };
        if id != &source.id || origin != &source.origin {
            return Err(MutationProblem::binding(
                Phase::Mutation,
                "statement-identity",
            ));
        }
        match plan {
            P::RejectedIdentity { reason, .. } => {
                Err(MutationProblem::rejected(Phase::Mutation, *reason))
            }
            P::SubjectRejected { subject, .. } => {
                self.term(&source.subject, subject)?;
                Err(MutationProblem::binding(
                    Phase::TermRead,
                    "subject-rejection-missing",
                ))
            }
            P::PredicateRejected {
                subject, predicate, ..
            } => {
                self.term(&source.subject, subject)?;
                self.term(&source.predicate, predicate)?;
                Err(MutationProblem::binding(
                    Phase::TermRead,
                    "predicate-rejection-missing",
                ))
            }
            P::Ready {
                subject,
                predicate,
                subject_json,
                predicate_json,
                ..
            } => {
                self.term(&source.subject, subject)?;
                self.term(&source.predicate, predicate)?;
                // The original uniqueness key uses JSON bytes. Bind Native's
                // codec output to the same SDK DTO serialization at the
                // original insert position; semantic JSON equality is weaker.
                let subject_bound = serde_json::to_string(&source.subject).map_err(|_| {
                    MutationProblem::binding(Phase::StatementInsert, "subject-codec")
                })?;
                let predicate_bound = serde_json::to_string(&source.predicate).map_err(|_| {
                    MutationProblem::binding(Phase::StatementInsert, "predicate-codec")
                })?;
                if &subject_bound != subject_json || &predicate_bound != predicate_json {
                    return Err(MutationProblem::binding(
                        Phase::StatementInsert,
                        "statement-codec",
                    ));
                }
                self.write(
                    Phase::StatementInsert,
                    self.resources.bootstrap().mutation_statement_insert(),
                    vec![
                        DataValue::Text(id.clone()),
                        DataValue::Text(subject_json.clone()),
                        DataValue::Text(predicate_json.clone()),
                        DataValue::Text(origin.clone()),
                    ],
                )?;
                // Human statements never acquire legacy relation-role writes.
                Ok(())
            }
        }
    }
}
