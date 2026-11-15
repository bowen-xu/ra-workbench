use super::{
    CapturedMerge, MergeCause, MergeFailure, MergeOutcome, MergePhase as Phase, MergeProblem,
    MergeResources, load,
};
use crate::{
    data_interface::{
        DatabaseGuard, guarded_transaction,
        plugin_resources::{ColumnCodec as C, RowPolicy as P, write_in},
    },
    runtime_plugins::{data::AcceptedDataResource, native::SafeDatabaseError},
};
use research_workbench_native_sdk::{
    data::{DataRows, DataValue as V},
    duplicate_merge::DuplicateMergeSnapshot,
    duplicate_merge_prepare::MergePreparationV1,
    duplicate_review::MergeOperation,
};
use rusqlite::{Connection, DropBehavior, Transaction, TransactionBehavior};
use uuid::Uuid;

mod inner;
type Result<T> = std::result::Result<T, MergeProblem>;

struct Execution<'a, 'connection> {
    tx: &'a Transaction<'connection>,
    r: &'a MergeResources,
    captured: &'a CapturedMerge,
    source: &'a str,
    state: &'a str,
    created_reading_state: bool,
    operation_id: Option<String>,
}

pub(crate) fn verify_preparation(
    captured: &CapturedMerge,
    plan: &MergePreparationV1,
) -> Result<()> {
    let MergePreparationV1::Ready {
        input,
        merged_item_id,
        operation_state,
        created_reading_state,
    } = plan
    else {
        return Err(MergeProblem {
            phase: Phase::Preparation,
            cause: MergeCause::Missing,
        });
    };
    let bound = (input.survivor_item_id == input.left_item_id
        && merged_item_id == &input.right_item_id)
        || (input.survivor_item_id == input.right_item_id && merged_item_id == &input.left_item_id);
    if input != &captured.input || !bound || operation_state != "applied" || *created_reading_state
    {
        return Err(MergeProblem::binding(
            Phase::Preparation,
            "candidate-preparation",
        ));
    }
    Ok(())
}

pub(crate) fn apply(
    guard: &mut DatabaseGuard<'_>,
    r: &MergeResources,
    captured: &CapturedMerge,
    plan: &MergePreparationV1,
) -> std::result::Result<MergeOperation, MergeFailure> {
    if let Err(problem) = verify_preparation(captured, plan) {
        return Err(MergeFailure {
            problem,
            outcome: MergeOutcome::NotStarted,
            rollback_error: None,
            operation_id: None,
        });
    }
    guarded_transaction::run(guard, |db| run(db, r, captured, plan))
}

fn run(
    db: &Connection,
    r: &MergeResources,
    captured: &CapturedMerge,
    plan: &MergePreparationV1,
) -> (std::result::Result<MergeOperation, MergeFailure>, bool) {
    let was_autocommit = db.is_autocommit();
    let mut tx = match Transaction::new_unchecked(db, TransactionBehavior::Deferred) {
        Ok(tx) => tx,
        Err(e) => {
            let unknown = was_autocommit && !db.is_autocommit();
            return (
                Err(MergeFailure {
                    problem: MergeProblem::sql(Phase::Begin, "begin", &e),
                    outcome: if unknown {
                        MergeOutcome::Unknown
                    } else {
                        MergeOutcome::NotStarted
                    },
                    rollback_error: None,
                    operation_id: None,
                }),
                unknown,
            );
        }
    };
    tx.set_drop_behavior(DropBehavior::Ignore);
    let MergePreparationV1::Ready {
        merged_item_id,
        operation_state,
        created_reading_state,
        ..
    } = plan
    else {
        return failed(
            db,
            MergeProblem::binding(Phase::Preparation, "verified-plan-shape"),
            None,
            tx.rollback(),
        );
    };
    let mut execution = Execution {
        tx: &tx,
        r,
        captured,
        source: merged_item_id,
        state: operation_state,
        created_reading_state: *created_reading_state,
        operation_id: None,
    };
    let result = execution.run();
    let operation_id = execution.operation_id.clone();
    let operation = match result {
        Ok(op) => op,
        Err(problem) => return failed(db, problem, operation_id, tx.rollback()),
    };
    if let Err(e) = tx.commit() {
        let problem = MergeProblem::sql(Phase::Commit, "commit", &e);
        if db.is_autocommit() {
            return (
                Err(MergeFailure {
                    problem,
                    outcome: MergeOutcome::Unknown,
                    rollback_error: None,
                    operation_id,
                }),
                true,
            );
        }
        return failed(db, problem, operation_id, db.execute_batch("ROLLBACK"));
    }
    if !db.is_autocommit() {
        return (
            Err(MergeFailure {
                problem: MergeProblem {
                    phase: Phase::Commit,
                    cause: MergeCause::Database(Box::new(SafeDatabaseError::invariant(
                        "duplicate_merge_transaction_unsettled",
                        "commit",
                    ))),
                },
                outcome: MergeOutcome::Unknown,
                rollback_error: None,
                operation_id,
            }),
            true,
        );
    }
    (Ok(operation), false)
}

fn failed(
    db: &Connection,
    problem: MergeProblem,
    operation_id: Option<String>,
    rollback: rusqlite::Result<()>,
) -> (std::result::Result<MergeOperation, MergeFailure>, bool) {
    let rollback_error = match rollback {
        Ok(()) if db.is_autocommit() => None,
        Err(e) => Some(Box::new(SafeDatabaseError::sql("rollback", &e))),
        Ok(()) => Some(Box::new(SafeDatabaseError::invariant(
            "duplicate_merge_transaction_unsettled",
            "rollback",
        ))),
    };
    let unknown = rollback_error.is_some();
    (
        Err(MergeFailure {
            problem,
            outcome: if unknown {
                MergeOutcome::Unknown
            } else {
                MergeOutcome::RolledBack
            },
            rollback_error,
            operation_id,
        }),
        unknown,
    )
}

impl Execution<'_, '_> {
    fn read(
        &self,
        phase: Phase,
        resource: &AcceptedDataResource,
        args: Vec<V>,
        policy: P,
        codecs: &[C],
    ) -> Result<DataRows> {
        load::read(self.tx, resource, args, phase, policy, codecs)
    }
    fn write(&self, phase: Phase, resource: &AcceptedDataResource, args: Vec<V>) -> Result<()> {
        write_in(self.tx, resource, &load::call(resource, args))
            .map(|_| ())
            .map_err(|e| MergeProblem::data(phase, e))
    }
    fn strings(&self, resource: &AcceptedDataResource, args: Vec<V>) -> Result<Vec<String>> {
        load::strings(
            self.read(Phase::Outer, resource, args, P::All, &[C::String])?,
            Phase::Outer,
        )
    }
    fn run(&mut self) -> Result<MergeOperation> {
        use load::text;
        let r = &self.r.outer;
        let source = self.source;
        let survivor = &self.captured.input.survivor_item_id;
        let rows = self.read(
            Phase::Outer,
            &r.archived_at,
            vec![text(source)],
            P::FirstRequired,
            &[C::OptionalString],
        )?;
        let archived = match rows.rows.first().map(Vec::as_slice) {
            Some([V::Text(value)]) => Some(value.clone()),
            Some([V::Null]) => None,
            _ => return Err(MergeProblem::binding(Phase::Outer, "source-archive-row")),
        };
        let attachments = self.strings(&r.attachments, vec![text(source)])?;
        let identifiers = self.strings(&r.identifiers, vec![text(source)])?;
        let anchors = self.strings(&r.anchors, vec![text(source)])?;
        let projects = self.strings(&r.added_projects, vec![text(source), text(survivor)])?;
        let tags = self.strings(&r.added_tags, vec![text(source), text(survivor)])?;
        let provenance = self.strings(&r.added_provenance, vec![text(source), text(survivor)])?;
        for id in &projects {
            self.write(
                Phase::Outer,
                &r.insert_project,
                vec![text(id), text(survivor), text(source)],
            )?;
        }
        for id in &tags {
            self.write(Phase::Outer, &r.insert_tag, vec![text(survivor), text(id)])?;
        }
        for field in &provenance {
            self.write(
                Phase::Outer,
                &r.insert_provenance,
                vec![text(source), text(survivor), text(field)],
            )?;
        }
        self.write(
            Phase::Outer,
            &r.move_reading_states,
            vec![text(source), text(survivor)],
        )?;
        self.write(
            Phase::Outer,
            &r.move_attachments,
            vec![text(source), text(survivor)],
        )?;
        self.write(
            Phase::Outer,
            &r.move_identifiers,
            vec![text(source), text(survivor)],
        )?;
        self.write(
            Phase::Outer,
            &r.move_anchors,
            vec![text(source), text(survivor)],
        )?;
        self.write(
            Phase::Outer,
            &r.move_reading_sources,
            vec![
                text(survivor),
                V::Text(load::encode(&anchors, Phase::Outer)?),
            ],
        )?;
        self.write(Phase::Outer, &r.archive_source, vec![text(source)])?;
        self.write(
            Phase::Outer,
            &r.mark_candidate,
            vec![text(&self.captured.input.candidate_id)],
        )?;
        self.write(Phase::Search, &r.search_delete, vec![text(source)])?;
        self.write(Phase::Search, &r.search_delete, vec![text(survivor)])?;
        self.write(Phase::Search, &r.search_insert, vec![text(survivor)])?;
        let titles = self.read(
            Phase::Titles,
            &r.titles,
            vec![text(survivor), text(source)],
            P::FirstRequired,
            &[C::String; 2],
        )?;
        let Some([V::Text(survivor_title), V::Text(merged_title)]) =
            titles.rows.first().map(Vec::as_slice)
        else {
            return Err(MergeProblem::binding(Phase::Titles, "title-pair"));
        };
        let id = Uuid::new_v4().to_string();
        self.operation_id = Some(id.clone());
        let created_at = load::one(
            self.read(
                Phase::Timestamp,
                &r.timestamp,
                vec![],
                P::FirstRequired,
                &[C::String],
            )?,
            Phase::Timestamp,
        )?;
        let operation = MergeOperation {
            id,
            survivor_item_id: survivor.clone(),
            survivor_title: survivor_title.clone(),
            merged_item_id: source.to_owned(),
            merged_title: merged_title.clone(),
            state: self.state.to_owned(),
            created_at,
        };
        let snapshot = DuplicateMergeSnapshot {
            classification: Some(self.classification()?),
            candidate_id: self.captured.input.candidate_id.clone(),
            attachment_ids: attachments,
            identifier_ids: identifiers,
            anchor_ids: anchors,
            added_project_ids: projects,
            added_tag_ids: tags,
            added_provenance_fields: provenance,
            created_reading_state: self.created_reading_state,
            merged_archived_at: archived,
        };
        self.write(
            Phase::History,
            &r.history_insert,
            vec![
                text(&operation.id),
                text(survivor),
                text(source),
                V::Text(load::encode(&snapshot, Phase::Snapshot)?),
                text(&operation.created_at),
            ],
        )?;
        Ok(operation)
    }
}
