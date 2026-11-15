use super::{
    BootstrapFailure, BootstrapOutcome, BootstrapProblem, BootstrapResources, CaptureError,
    FailureContext, PreparedBootstrapFacts, outcome::BootstrapCause,
};
use crate::{
    data_interface::{
        DatabaseGuard, guarded_transaction,
        plugin_resources::{ColumnCodec, RowPolicy, write_in},
    },
    runtime_plugins::{data::AcceptedDataResource, native::SafeDatabaseError},
};
use research_workbench_native_sdk::{
    data::{DataRows, DataValue},
    nal_annotation::NalAnnotation,
    nal_bootstrap::{BootstrapInputV1, BootstrapPhaseV1, BootstrapPreparationV1, ConceptCallKeyV1},
    nal_state::{NalRule, NalState},
};
use rusqlite::{Connection, DropBehavior, Transaction, TransactionBehavior};

mod concept;
mod legacy;
mod load;
mod semantic;

pub(super) type Result<T> = std::result::Result<T, BootstrapProblem>;

pub(super) struct Execution<'a, 'connection> {
    pub(super) db: &'a Transaction<'connection>,
    pub(super) resources: &'a BootstrapResources,
    pub(super) captured: &'a PreparedBootstrapFacts,
    pub(super) input: &'a BootstrapInputV1,
    pub(super) plan: &'a BootstrapPreparationV1,
    pub(super) context: FailureContext,
    pub(super) statement_cursor: usize,
}

impl Execution<'_, '_> {
    pub(super) fn problem(&self, phase: BootstrapPhaseV1, cause: CaptureError) -> BootstrapProblem {
        BootstrapProblem::capture(phase, self.context, cause)
    }

    pub(super) fn read(
        &self,
        phase: BootstrapPhaseV1,
        resource: &AcceptedDataResource,
        arguments: Vec<DataValue>,
        policy: RowPolicy,
        codecs: &[ColumnCodec],
    ) -> Result<DataRows> {
        self.captured
            .require_probe_in(self.db, phase, resource, &arguments)
            .map_err(|error| self.problem(phase, error))?;
        super::rows::read(self.db, resource, arguments, policy, codecs)
            .map_err(|error| self.problem(phase, error))
    }

    pub(super) fn read_source(
        &self,
        phase: BootstrapPhaseV1,
        resource: &AcceptedDataResource,
        arguments: Vec<DataValue>,
        policy: RowPolicy,
        codecs: &[ColumnCodec],
    ) -> Result<DataRows> {
        let rows = self.read(phase, resource, arguments.clone(), policy, codecs)?;
        self.captured
            .check_source_rows(phase, resource, &arguments, &rows)
            .map_err(|error| self.problem(phase, error))?;
        Ok(rows)
    }

    pub(super) fn write(
        &self,
        phase: BootstrapPhaseV1,
        resource: &AcceptedDataResource,
        arguments: Vec<DataValue>,
    ) -> Result<u64> {
        write_in(self.db, resource, &super::rows::call(resource, arguments))
            .map_err(|error| self.problem(phase, CaptureError::Data(error)))
    }

    pub(super) fn has_intent(&self, key: &ConceptCallKeyV1) -> bool {
        self.input
            .probes
            .concepts
            .iter()
            .any(|value| &value.key == key)
    }

    fn run(&mut self) -> Result<NalState> {
        self.import()?;
        self.repair()?;
        self.cleanup()?;
        self.finish_state()
    }
}

/// Execute the fixed startup chain inside the caller's existing transaction.
/// This neither starts nor settles a transaction and contains no host wait.
pub(crate) fn execute_in(
    tx: &Transaction<'_>,
    resources: &BootstrapResources,
    captured: &PreparedBootstrapFacts,
    input: &BootstrapInputV1,
    plan: &BootstrapPreparationV1,
) -> Result<NalState> {
    captured.recheck_in(tx).map_err(|error| {
        BootstrapProblem::capture(
            BootstrapPhaseV1::LegacyHeader,
            FailureContext::Network,
            error,
        )
    })?;
    if input.source != captured.source || input.facts != captured.facts {
        return Err(BootstrapProblem::binding(
            BootstrapPhaseV1::LegacyHeader,
            "coherent-input",
        ));
    }
    Execution {
        db: tx,
        resources,
        captured,
        input,
        plan,
        context: FailureContext::Network,
        statement_cursor: 0,
    }
    .run()
}

/// LinkAnnotation's second complete load observes the caller's current writes.
/// Only its annotation-dependent policy is prepared; other state is decoded
/// at the original point without comparing it to the earlier final projection.
pub(crate) fn load_link_annotations_in(
    tx: &Transaction<'_>,
    resources: &BootstrapResources,
    captured: &PreparedBootstrapFacts,
    input: &BootstrapInputV1,
    plan: &BootstrapPreparationV1,
) -> Result<Vec<NalAnnotation>> {
    Execution {
        db: tx,
        resources,
        captured,
        input,
        plan,
        context: FailureContext::Network,
        statement_cursor: 0,
    }
    .link_annotations()
}

/// Read the actual post-mutation state inside the caller's transaction. Rules
/// are selected from Native's finite projection using actual ordered SQL IDs.
/// This does not compare the earlier State, clean up or settle the transaction.
pub(crate) fn load_mutation_state_in(
    tx: &Transaction<'_>,
    resources: &BootstrapResources,
    captured: &PreparedBootstrapFacts,
    input: &BootstrapInputV1,
    plan: &BootstrapPreparationV1,
    rule_projection: &[NalRule],
) -> Result<NalState> {
    Execution {
        db: tx,
        resources,
        captured,
        input,
        plan,
        context: FailureContext::Network,
        statement_cursor: 0,
    }
    .mutation_state(rule_projection)
}

/// There is no host wait inside this adapter. The accepted call rechecks owned
/// observations, executes the original schedule and decodes its final state
/// before committing once under the existing recovery-aware database owner.
pub(crate) fn apply(
    guard: &mut DatabaseGuard<'_>,
    resources: &BootstrapResources,
    captured: &PreparedBootstrapFacts,
    input: &BootstrapInputV1,
    plan: &BootstrapPreparationV1,
) -> std::result::Result<NalState, BootstrapFailure> {
    guarded_transaction::run(guard, |connection| {
        run(connection, resources, captured, input, plan)
    })
}

fn run(
    db: &Connection,
    resources: &BootstrapResources,
    captured: &PreparedBootstrapFacts,
    input: &BootstrapInputV1,
    plan: &BootstrapPreparationV1,
) -> (std::result::Result<NalState, BootstrapFailure>, bool) {
    let was_autocommit = db.is_autocommit();
    let mut tx = match Transaction::new_unchecked(db, TransactionBehavior::Deferred) {
        Ok(tx) => tx,
        Err(error) => {
            let unknown = was_autocommit && !db.is_autocommit();
            return (
                Err(BootstrapFailure {
                    problem: transaction_problem(BootstrapPhaseV1::LegacyHeader, "begin", &error),
                    outcome: if unknown {
                        BootstrapOutcome::Unknown {
                            observed_revision: None,
                        }
                    } else {
                        BootstrapOutcome::NotStarted
                    },
                    rollback_error: None,
                }),
                unknown,
            );
        }
    };
    tx.set_drop_behavior(DropBehavior::Ignore);
    let result = execute_in(&tx, resources, captured, input, plan);
    let state = match result {
        Ok(state) => state,
        Err(problem) => return failed(db, problem, tx.rollback(), None),
    };
    let observed_revision = Some(state.revision);
    if let Err(error) = tx.commit() {
        let problem = transaction_problem(BootstrapPhaseV1::FinalLoad, "commit", &error);
        if db.is_autocommit() {
            return (
                Err(BootstrapFailure {
                    problem,
                    outcome: BootstrapOutcome::Unknown { observed_revision },
                    rollback_error: None,
                }),
                true,
            );
        }
        return failed(db, problem, db.execute_batch("ROLLBACK"), observed_revision);
    }
    if !db.is_autocommit() {
        return (
            Err(BootstrapFailure {
                problem: BootstrapProblem {
                    phase: BootstrapPhaseV1::FinalLoad,
                    context: FailureContext::Network,
                    cause: BootstrapCause::Database(Box::new(SafeDatabaseError::invariant(
                        "nal_bootstrap_transaction_unsettled",
                        "commit",
                    ))),
                },
                outcome: BootstrapOutcome::Unknown { observed_revision },
                rollback_error: None,
            }),
            true,
        );
    }
    (Ok(state), false)
}

fn transaction_problem(
    phase: BootstrapPhaseV1,
    operation: &'static str,
    error: &rusqlite::Error,
) -> BootstrapProblem {
    BootstrapProblem {
        phase,
        context: FailureContext::Network,
        cause: BootstrapCause::Database(Box::new(SafeDatabaseError::sql(operation, error))),
    }
}

fn failed<T>(
    db: &Connection,
    problem: BootstrapProblem,
    rollback: rusqlite::Result<()>,
    observed_revision: Option<i64>,
) -> (std::result::Result<T, BootstrapFailure>, bool) {
    let rollback_error = match rollback {
        Ok(()) if db.is_autocommit() => None,
        Err(error) => Some(Box::new(SafeDatabaseError::sql("rollback", &error))),
        Ok(()) => Some(Box::new(SafeDatabaseError::invariant(
            "nal_bootstrap_transaction_unsettled",
            "rollback",
        ))),
    };
    let unknown = rollback_error.is_some();
    (
        Err(BootstrapFailure {
            problem,
            outcome: if unknown {
                BootstrapOutcome::Unknown { observed_revision }
            } else {
                BootstrapOutcome::RolledBack
            },
            rollback_error,
        }),
        unknown,
    )
}
