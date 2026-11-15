use super::outcome::{MutationCause as Cause, MutationPhase as Phase};
use super::{MutationFailure, MutationOutcome, MutationProblem, MutationResources};
use crate::{
    data_interface::{DatabaseGuard, guarded_transaction, plugin_resources::write_in},
    runtime_plugins::{
        data::AcceptedDataResource,
        literature_nal_bootstrap::{self as bootstrap, PreparedBootstrapFacts},
        native::SafeDatabaseError,
    },
};
use research_workbench_native_sdk::{
    data::{DataCall, DataValue},
    nal_mutation::{
        NalMutation, NalMutationDecision, NalMutationPreparation, NalMutationPrepareInput,
    },
    nal_state::NalState,
};
use rusqlite::{Connection, DropBehavior, Transaction, TransactionBehavior};

mod actions;
mod statements;
mod terms;

type Result<T> = std::result::Result<T, MutationProblem>;

struct Execution<'a, 'connection> {
    tx: &'a Transaction<'connection>,
    resources: &'a MutationResources,
    captured: &'a PreparedBootstrapFacts,
    input: &'a NalMutationPrepareInput,
    plan: &'a NalMutationPreparation,
    mutation: &'a NalMutation,
    expected_revision: i64,
}

impl Execution<'_, '_> {
    fn run(&self) -> Result<NalState> {
        let state = bootstrap::execute_in(
            self.tx,
            self.resources.bootstrap(),
            self.captured,
            &self.input.bootstrap,
            &self.plan.bootstrap,
        )
        .map_err(|problem| MutationProblem {
            phase: Phase::Startup,
            cause: Cause::Startup(Box::new(problem)),
        })?;
        // Startup and the first complete load precede CAS and every mutation
        // rejection. Neither Native preparation nor an extra read may move it.
        if state.revision != self.expected_revision {
            return Err(MutationProblem {
                phase: Phase::RevisionCheck,
                cause: Cause::Conflict,
            });
        }
        let encoded = serde_json::to_string(self.mutation)
            .map_err(|_| MutationProblem::binding(Phase::Mutation, "request-codec"))?;
        if encoded != self.input.mutation_json {
            return Err(MutationProblem::binding(
                Phase::Mutation,
                "original-request",
            ));
        }
        let NalMutationDecision::Prepared { mutation } = &self.plan.decision else {
            return Err(MutationProblem::binding(
                Phase::Mutation,
                "startup-succeeded",
            ));
        };
        self.action(mutation)?;
        // A no-op unlink, ignored insert or disabled rule still increments.
        self.write(
            Phase::Revision,
            self.resources.annotation.revision_increment(),
            vec![],
        )?;
        // This is the actual second full load, without post-mutation cleanup
        // or an equality assertion against the pre-mutation state.
        bootstrap::load_mutation_state_in(
            self.tx,
            self.resources.bootstrap(),
            self.captured,
            &self.input.bootstrap,
            &self.plan.bootstrap,
            &self.plan.rule_projection,
        )
        .map_err(|problem| MutationProblem {
            phase: Phase::FinalLoad,
            cause: Cause::Startup(Box::new(problem)),
        })
    }

    fn write(
        &self,
        phase: Phase,
        resource: &AcceptedDataResource,
        args: Vec<DataValue>,
    ) -> Result<u64> {
        write_in(self.tx, resource, &call(resource, args)).map_err(|error| MutationProblem {
            phase,
            cause: Cause::Data(error),
        })
    }
}

fn call(resource: &AcceptedDataResource, arguments: Vec<DataValue>) -> DataCall {
    DataCall {
        id: resource.grant().id.clone(),
        version: resource.grant().version,
        arguments,
    }
}

pub(crate) fn apply(
    guard: &mut DatabaseGuard<'_>,
    resources: &MutationResources,
    captured: &PreparedBootstrapFacts,
    input: &NalMutationPrepareInput,
    plan: &NalMutationPreparation,
    mutation: &NalMutation,
    expected_revision: i64,
) -> std::result::Result<NalState, MutationFailure> {
    guarded_transaction::run(guard, |db| {
        run(
            db,
            resources,
            captured,
            input,
            plan,
            mutation,
            expected_revision,
        )
    })
}

fn run(
    db: &Connection,
    resources: &MutationResources,
    captured: &PreparedBootstrapFacts,
    input: &NalMutationPrepareInput,
    plan: &NalMutationPreparation,
    mutation: &NalMutation,
    expected_revision: i64,
) -> (std::result::Result<NalState, MutationFailure>, bool) {
    let was_autocommit = db.is_autocommit();
    let mut tx = match Transaction::new_unchecked(db, TransactionBehavior::Deferred) {
        Ok(tx) => tx,
        Err(error) => {
            let unknown = was_autocommit && !db.is_autocommit();
            return (
                Err(MutationFailure {
                    problem: MutationProblem::sql(Phase::Begin, "begin", &error),
                    outcome: if unknown {
                        MutationOutcome::Unknown {
                            observed_revision: None,
                        }
                    } else {
                        MutationOutcome::NotStarted
                    },
                    rollback_error: None,
                }),
                unknown,
            );
        }
    };
    tx.set_drop_behavior(DropBehavior::Ignore);
    let result = Execution {
        tx: &tx,
        resources,
        captured,
        input,
        plan,
        mutation,
        expected_revision,
    }
    .run();
    let state = match result {
        Ok(state) => state,
        Err(problem) => return failed(db, problem, tx.rollback(), None),
    };
    let revision = Some(state.revision);
    if let Err(error) = tx.commit() {
        let problem = MutationProblem::sql(Phase::Commit, "commit", &error);
        if db.is_autocommit() {
            return (
                Err(MutationFailure {
                    problem,
                    outcome: MutationOutcome::Unknown {
                        observed_revision: revision,
                    },
                    rollback_error: None,
                }),
                true,
            );
        }
        return failed(db, problem, db.execute_batch("ROLLBACK"), revision);
    }
    if !db.is_autocommit() {
        return (
            Err(MutationFailure {
                problem: MutationProblem {
                    phase: Phase::Commit,
                    cause: Cause::Database(Box::new(SafeDatabaseError::invariant(
                        "nal_mutation_transaction_unsettled",
                        "commit",
                    ))),
                },
                outcome: MutationOutcome::Unknown {
                    observed_revision: revision,
                },
                rollback_error: None,
            }),
            true,
        );
    }
    (Ok(state), false)
}

fn failed(
    db: &Connection,
    problem: MutationProblem,
    rollback: rusqlite::Result<()>,
    observed_revision: Option<i64>,
) -> (std::result::Result<NalState, MutationFailure>, bool) {
    let rollback_error = match rollback {
        Ok(()) if db.is_autocommit() => None,
        Err(error) => Some(Box::new(SafeDatabaseError::sql("rollback", &error))),
        Ok(()) => Some(Box::new(SafeDatabaseError::invariant(
            "nal_mutation_transaction_unsettled",
            "rollback",
        ))),
    };
    let unknown = rollback_error.is_some();
    (
        Err(MutationFailure {
            problem,
            outcome: if unknown {
                MutationOutcome::Unknown { observed_revision }
            } else {
                MutationOutcome::RolledBack
            },
            rollback_error,
        }),
        unknown,
    )
}
