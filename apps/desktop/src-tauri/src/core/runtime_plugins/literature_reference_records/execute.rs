//! Consume a pure preparation at the original SQL, transaction and codec positions.
use super::{
    ReferenceFailure, ReferenceOutcome, ReferenceProblem, ReferenceProgress, ReferenceResources,
    data::{self, optional_text, text},
};
use crate::data_interface::{
    DatabaseGuard, guarded_transaction,
    plugin_resources::{ColumnCodec, RowPolicy},
};
use research_workbench_native_sdk::{
    data::DataValue,
    reference_records::{
        ReferenceRecord, ReferenceRecordPlanV1, ReferenceRecordRejectionV1 as Rejection,
    },
};
use rusqlite::{Connection, DropBehavior, Transaction, TransactionBehavior};

type Result<T> = std::result::Result<T, ReferenceFailure>;

pub(crate) fn save(
    guard: &mut DatabaseGuard<'_>,
    resources: &ReferenceResources,
    plan: &ReferenceRecordPlanV1,
) -> Result<ReferenceRecord> {
    guarded_transaction::run(guard, |db| {
        let result = run(db, resources, plan);
        let unknown = result
            .as_ref()
            .is_err_and(|failure| failure.progress.outcome == ReferenceOutcome::Unknown);
        (result, unknown)
    })
}

fn run(
    db: &Connection,
    resources: &ReferenceResources,
    plan: &ReferenceRecordPlanV1,
) -> Result<ReferenceRecord> {
    let mut progress = ReferenceProgress::new("source-anchor");
    progress.record_id = plan.draft.id.clone();
    relations(db, resources, plan, &progress)?;
    progress.phase = "begin";
    let was_autocommit = db.is_autocommit();
    let mut transaction = match Transaction::new_unchecked(db, TransactionBehavior::Deferred) {
        Ok(transaction) => transaction,
        Err(cause) => {
            if was_autocommit && !db.is_autocommit() {
                progress.outcome = ReferenceOutcome::Unknown;
            }
            return Err(progress.database(&cause));
        }
    };
    transaction.set_drop_behavior(DropBehavior::Ignore);
    if let Err(mut failure) = mutate(&transaction, resources, plan, &mut progress) {
        settle_failed(db, transaction.rollback(), &mut failure);
        return Err(failure);
    }
    progress.phase = "commit";
    if let Err(cause) = transaction.commit() {
        let mut failure = progress.database(&cause);
        if db.is_autocommit() {
            failure.progress.outcome = ReferenceOutcome::Unknown;
        } else {
            settle_failed(db, db.execute_batch("ROLLBACK"), &mut failure);
        }
        return Err(failure);
    }
    if !db.is_autocommit() {
        progress.outcome = ReferenceOutcome::Unknown;
        return Err(progress.failure(ReferenceProblem::Owner("REFERENCE_TRANSACTION_UNSETTLED")));
    }
    progress.outcome = ReferenceOutcome::Committed;
    progress.phase = "readback";
    let id = progress
        .record_id
        .as_deref()
        .ok_or_else(|| progress.failure(ReferenceProblem::Binding))?;
    data::record(db, resources, id, &progress)
}

fn relations(
    db: &Connection,
    resources: &ReferenceResources,
    plan: &ReferenceRecordPlanV1,
    progress: &ReferenceProgress,
) -> Result<()> {
    let draft = &plan.draft;
    if !data::belongs(
        db,
        resources,
        &draft.source_anchor_id,
        &draft.citing_paper_id,
        progress,
    )? {
        return Err(progress.failure(ReferenceProblem::Domain(Rejection::AnchorOwnership)));
    }
    if let Some(anchor) = draft.usage_anchor_id.as_deref() {
        let progress = progress.at("usage-anchor");
        if !data::belongs(db, resources, anchor, &draft.citing_paper_id, &progress)? {
            return Err(progress.failure(ReferenceProblem::Domain(Rejection::AnchorOwnership)));
        }
    }
    if let Some(item) = draft.matched_item_id.as_deref() {
        let progress = progress.at("matched-item");
        if !data::matched_exists(db, resources, item, &progress)? {
            return Err(progress.failure(ReferenceProblem::Domain(Rejection::MatchedItemMissing)));
        }
    }
    Ok(())
}

fn mutate(
    transaction: &Transaction<'_>,
    resources: &ReferenceResources,
    plan: &ReferenceRecordPlanV1,
    progress: &mut ReferenceProgress,
) -> Result<()> {
    let draft = &plan.draft;
    let (id, revision) = match draft.id.as_deref() {
        None => {
            // This original rejection occurs after BEGIN, before allocating an ID.
            progress.phase = "revision";
            if draft.expected_revision.is_some() {
                return Err(
                    progress.failure(ReferenceProblem::Domain(Rejection::NewExpectedRevision))
                );
            }
            let id = uuid::Uuid::new_v4().to_string();
            progress.record_id = Some(id.clone());
            progress.phase = "statement";
            resources
                .write(transaction, "insert", values(plan, &id))
                .map_err(|cause| progress.data(cause))?;
            (id, 1)
        }
        Some(id) => {
            progress.phase = "header";
            let rows = resources
                .read(
                    transaction,
                    "header",
                    vec![text(id)],
                    RowPolicy::FirstOptional,
                    &[ColumnCodec::I64, ColumnCodec::String, ColumnCodec::String],
                )
                .map_err(|cause| progress.data(cause))?;
            let row = rows
                .rows
                .first()
                .ok_or_else(|| progress.failure(ReferenceProblem::Missing))?;
            let [
                DataValue::Integer(current),
                DataValue::Text(paper),
                DataValue::Text(anchor),
            ] = row.as_slice()
            else {
                return Err(progress.data(resources.shape_error("header")));
            };
            progress.phase = "identity";
            if &draft.citing_paper_id != paper || &draft.source_anchor_id != anchor {
                return Err(progress.failure(ReferenceProblem::Domain(Rejection::ImmutableOwner)));
            }
            progress.phase = "revision";
            if draft.expected_revision != Some(*current) {
                progress.current_revision = Some(*current);
                return Err(progress.failure(ReferenceProblem::RevisionConflict));
            }
            // Keep the original i64 arithmetic and its guarded panic boundary.
            let next = current + 1;
            let mut arguments = values(plan, id);
            arguments.extend([DataValue::Integer(next), DataValue::Integer(*current)]);
            progress.phase = "statement";
            let changed = resources
                .write(transaction, "update", arguments)
                .map_err(|cause| progress.data(cause))?;
            if changed != 1 {
                progress.current_revision = Some(*current);
                return Err(progress.failure(ReferenceProblem::RevisionConflict));
            }
            (id.into(), next)
        }
    };
    progress.phase = "history";
    resources
        .write(
            transaction,
            "revision",
            vec![
                text(&uuid::Uuid::new_v4().to_string()),
                text(&id),
                DataValue::Integer(revision),
                text(&plan.snapshot_json),
                text(&plan.change_reason),
            ],
        )
        .map_err(|cause| progress.data(cause))?;
    Ok(())
}

fn values(plan: &ReferenceRecordPlanV1, id: &str) -> Vec<DataValue> {
    let draft = &plan.draft;
    vec![
        text(id),
        text(&draft.citing_paper_id),
        text(&draft.source_anchor_id),
        text(&plan.raw_text),
        text(&plan.fields_json),
        text(&draft.parse_state),
        text(&draft.identity_state),
        optional_text(draft.matched_item_id.as_deref()),
        text(&draft.semantic_state),
        optional_text(draft.usage_kind.as_deref()),
        text(&plan.usage_summary),
        optional_text(draft.usage_anchor_id.as_deref()),
    ]
}

fn settle_failed(db: &Connection, rollback: rusqlite::Result<()>, failure: &mut ReferenceFailure) {
    match rollback {
        Ok(()) if db.is_autocommit() => failure.progress.outcome = ReferenceOutcome::RolledBack,
        result => {
            failure.progress.outcome = ReferenceOutcome::Unknown;
            failure.rollback_error = Some(Box::new(match result {
                Err(cause) => {
                    crate::runtime_plugins::native::SafeDatabaseError::sql("rollback", &cause)
                }
                Ok(()) => crate::runtime_plugins::native::SafeDatabaseError::invariant(
                    "reference_transaction_unsettled",
                    "rollback",
                ),
            }));
        }
    }
}
