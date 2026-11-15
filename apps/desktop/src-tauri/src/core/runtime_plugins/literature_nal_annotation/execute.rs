use super::{
    AnnotationFailure, AnnotationOutcome, AnnotationProblem, AnnotationResources,
    outcome::{AnnotationCause, AnnotationPhase as Phase, AnnotationRejection as Rejection},
};
use crate::{
    data_interface::{
        DatabaseGuard, guarded_transaction,
        plugin_resources::{ColumnCodec, RowPolicy, read_on_connection, write_in},
    },
    notes,
    runtime_plugins::{
        data::AcceptedDataResource,
        literature_nal_bootstrap::{self as bootstrap, PreparedBootstrapFacts},
        native::SafeDatabaseError,
    },
};
use research_workbench_native_sdk::{
    data::{DataCall, DataValue},
    nal_annotation::NalAnnotation,
    nal_annotation_prepare::{
        NalAnnotationDecision, NalAnnotationLinkPreparation, NalAnnotationLinkTarget,
        NalAnnotationPreparation, NalAnnotationPrepareInput,
    },
    nal_bootstrap::{BootstrapInputV1, BootstrapPreparationV1},
};
use rusqlite::{Connection, DropBehavior, Transaction, TransactionBehavior};

type Result<T> = std::result::Result<T, AnnotationProblem>;

struct Execution<'a, 'connection> {
    tx: &'a Transaction<'connection>,
    resources: &'a AnnotationResources,
    captured: &'a PreparedBootstrapFacts,
    input: &'a NalAnnotationPrepareInput,
    plan: &'a NalAnnotationPreparation,
}

impl Execution<'_, '_> {
    fn run(&self) -> Result<NalAnnotation> {
        let state = bootstrap::execute_in(
            self.tx,
            &self.resources.bootstrap,
            self.captured,
            &self.input.bootstrap,
            &self.plan.bootstrap,
        )
        .map_err(|problem| AnnotationProblem {
            phase: Phase::Startup,
            cause: AnnotationCause::Startup(Box::new(problem)),
        })?;
        match &self.plan.decision {
            NalAnnotationDecision::DeferredAfterBootstrap {} => Err(AnnotationProblem::binding(
                Phase::Decision,
                "startup-succeeded",
            )),
            NalAnnotationDecision::TargetMissing {} => Err(AnnotationProblem::rejected(
                Phase::Decision,
                Rejection::TargetMissing,
            )),
            NalAnnotationDecision::Existing { annotation } => {
                if annotation.target_kind != self.input.request.target_kind
                    || annotation.target_id != self.input.request.target_id
                    || !state.annotations.contains(annotation)
                {
                    return Err(AnnotationProblem::binding(
                        Phase::Decision,
                        "existing-annotation",
                    ));
                }
                // Existing still commits the preceding startup effects. It
                // does not check Note activity or apply Core's title limit.
                Ok(annotation.clone())
            }
            NalAnnotationDecision::Create {
                annotation,
                raw_title,
                link,
            } => {
                if annotation.id != self.input.identities.annotation_id
                    || annotation.note_page_id != self.input.identities.page_id
                    || annotation.target_kind != self.input.request.target_kind
                    || annotation.target_id != self.input.request.target_id
                    || raw_title != &self.input.request.title
                {
                    return Err(AnnotationProblem::binding(
                        Phase::Decision,
                        "creation-identities",
                    ));
                }
                let page = notes::create_page_with_paper_in(
                    self.tx,
                    notes::NotePageDraft {
                        title: raw_title.clone(),
                        parent_page_id: None,
                        parent_block_position: None,
                    },
                    None,
                    Some(&self.input.identities.page_id),
                )
                .map_err(AnnotationProblem::note)?;
                if page != self.input.identities.page_id {
                    return Err(AnnotationProblem {
                        phase: Phase::CreateNote,
                        cause: AnnotationCause::Owner("core_note_identity_mismatch"),
                    });
                }
                apply_link_in(
                    self.tx,
                    self.resources,
                    self.captured,
                    &self.input.bootstrap,
                    &self.plan.bootstrap,
                    annotation,
                    link,
                )?;
                write(
                    self.tx,
                    Phase::Revision,
                    &self.resources.revision_increment,
                    vec![],
                )?;
                // Original ensure returns only the annotation. No added final
                // State/revision query is used to populate diagnostics.
                Ok(annotation.clone())
            }
        }
    }
}

pub(crate) fn apply_link_in(
    tx: &Transaction<'_>,
    resources: &AnnotationResources,
    captured: &PreparedBootstrapFacts,
    input: &BootstrapInputV1,
    bootstrap_plan: &BootstrapPreparationV1,
    annotation: &NalAnnotation,
    plan: &NalAnnotationLinkPreparation,
) -> Result<()> {
    let expected = match plan {
        NalAnnotationLinkPreparation::RejectedBeforeLoad { reason } => {
            return Err(AnnotationProblem::rejected(
                Phase::LinkIdentity,
                (*reason).into(),
            ));
        }
        NalAnnotationLinkPreparation::AfterLoadRejected { annotations, .. }
        | NalAnnotationLinkPreparation::ReadActive { annotations, .. } => annotations,
    };
    // This performs the original complete load and Fact codec, including
    // rules, after the Core page/block writes and before any link policy.
    let annotations = bootstrap::load_link_annotations_in(
        tx,
        &resources.bootstrap,
        captured,
        input,
        bootstrap_plan,
    )
    .map_err(|problem| AnnotationProblem {
        phase: Phase::LinkLoad,
        cause: AnnotationCause::Startup(Box::new(problem)),
    })?;
    if annotations != *expected {
        return Err(AnnotationProblem {
            phase: Phase::LinkConflict,
            cause: AnnotationCause::AnnotationSnapshotChanged,
        });
    }
    let target = match plan {
        NalAnnotationLinkPreparation::AfterLoadRejected { reason, .. } => {
            return Err(AnnotationProblem::rejected(
                Phase::LinkConflict,
                (*reason).into(),
            ));
        }
        NalAnnotationLinkPreparation::ReadActive { target, .. } => target,
        NalAnnotationLinkPreparation::RejectedBeforeLoad { .. } => {
            return Err(AnnotationProblem::binding(
                Phase::LinkConflict,
                "link-stage",
            ));
        }
    };
    let rows = read_on_connection(
        tx,
        &resources.note_active,
        &call(
            &resources.note_active,
            vec![DataValue::Text(annotation.note_page_id.clone())],
        ),
        RowPolicy::FirstRequired,
        &[ColumnCodec::Bool],
    )
    .map_err(|error| AnnotationProblem {
        phase: Phase::LinkNoteActive,
        cause: AnnotationCause::Data(error),
    })?;
    let active = match rows.rows.first().and_then(|row| row.first()) {
        Some(DataValue::Integer(value)) => *value != 0,
        _ => {
            return Err(AnnotationProblem {
                phase: Phase::LinkNoteActive,
                cause: AnnotationCause::Owner("annotation_note_active_shape"),
            });
        }
    };
    if !active {
        return Err(AnnotationProblem::rejected(
            Phase::LinkNoteActive,
            Rejection::InactiveNote,
        ));
    }
    let (concept, statement) = match target {
        NalAnnotationLinkTarget::Concept { id }
            if id == &annotation.target_id && annotation.target_kind == "concept" =>
        {
            (DataValue::Text(id.clone()), DataValue::Null)
        }
        NalAnnotationLinkTarget::Statement { id }
            if id == &annotation.target_id && annotation.target_kind == "statement" =>
        {
            (DataValue::Null, DataValue::Text(id.clone()))
        }
        NalAnnotationLinkTarget::InvalidTargetKind {} => {
            return Err(AnnotationProblem::rejected(
                Phase::LinkTarget,
                Rejection::InvalidTargetKind,
            ));
        }
        _ => return Err(AnnotationProblem::binding(Phase::LinkTarget, "link-target")),
    };
    // Conflict-ignored insertion is intentionally not required to affect a
    // row: this is the original fixed SQL, not a new receipt condition.
    write(
        tx,
        Phase::LinkInsert,
        &resources.insert,
        vec![
            DataValue::Text(annotation.id.clone()),
            concept,
            statement,
            DataValue::Text(annotation.note_page_id.clone()),
        ],
    )?;
    Ok(())
}

fn write(
    tx: &Transaction<'_>,
    phase: Phase,
    resource: &AcceptedDataResource,
    args: Vec<DataValue>,
) -> Result<()> {
    write_in(tx, resource, &call(resource, args))
        .map(|_| ())
        .map_err(|error| AnnotationProblem {
            phase,
            cause: AnnotationCause::Data(error),
        })
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
    resources: &AnnotationResources,
    captured: &PreparedBootstrapFacts,
    input: &NalAnnotationPrepareInput,
    plan: &NalAnnotationPreparation,
) -> std::result::Result<NalAnnotation, AnnotationFailure> {
    guarded_transaction::run(guard, |db| run(db, resources, captured, input, plan))
}

fn run(
    db: &Connection,
    resources: &AnnotationResources,
    captured: &PreparedBootstrapFacts,
    input: &NalAnnotationPrepareInput,
    plan: &NalAnnotationPreparation,
) -> (std::result::Result<NalAnnotation, AnnotationFailure>, bool) {
    let was_autocommit = db.is_autocommit();
    let mut tx = match Transaction::new_unchecked(db, TransactionBehavior::Immediate) {
        Ok(tx) => tx,
        Err(error) => {
            let unknown = was_autocommit && !db.is_autocommit();
            return (
                Err(AnnotationFailure {
                    problem: AnnotationProblem::sql(Phase::Begin, "begin", &error),
                    outcome: if unknown {
                        AnnotationOutcome::Unknown
                    } else {
                        AnnotationOutcome::NotStarted
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
    }
    .run();
    let annotation = match result {
        Ok(annotation) => annotation,
        Err(problem) => return failed(db, problem, tx.rollback()),
    };
    if let Err(error) = tx.commit() {
        let problem = AnnotationProblem::sql(Phase::Commit, "commit", &error);
        if db.is_autocommit() {
            return (
                Err(AnnotationFailure {
                    problem,
                    outcome: AnnotationOutcome::Unknown,
                    rollback_error: None,
                }),
                true,
            );
        }
        return failed(db, problem, db.execute_batch("ROLLBACK"));
    }
    if !db.is_autocommit() {
        return (
            Err(AnnotationFailure {
                problem: AnnotationProblem {
                    phase: Phase::Commit,
                    cause: AnnotationCause::Database(Box::new(SafeDatabaseError::invariant(
                        "nal_annotation_transaction_unsettled",
                        "commit",
                    ))),
                },
                outcome: AnnotationOutcome::Unknown,
                rollback_error: None,
            }),
            true,
        );
    }
    (Ok(annotation), false)
}

fn failed(
    db: &Connection,
    problem: AnnotationProblem,
    rollback: rusqlite::Result<()>,
) -> (std::result::Result<NalAnnotation, AnnotationFailure>, bool) {
    let rollback_error = match rollback {
        Ok(()) if db.is_autocommit() => None,
        Err(error) => Some(Box::new(SafeDatabaseError::sql("rollback", &error))),
        Ok(()) => Some(Box::new(SafeDatabaseError::invariant(
            "nal_annotation_transaction_unsettled",
            "rollback",
        ))),
    };
    let unknown = rollback_error.is_some();
    (
        Err(AnnotationFailure {
            problem,
            outcome: if unknown {
                AnnotationOutcome::Unknown
            } else {
                AnnotationOutcome::RolledBack
            },
            rollback_error,
        }),
        unknown,
    )
}
