//! Replay only confirmed read prefixes; publish once in the original Immediate tx.
use super::*;
use crate::{
    data_interface::guarded_transaction,
    runtime_plugins::{literature_catalog::save::SaveOutcome, native::SafeDatabaseError},
};
use rusqlite::{Connection, DropBehavior, TransactionBehavior};

#[derive(Default)]
struct State {
    project: Option<Candidate<()>>,
    record: Option<Candidate<Option<CheckpointRecord>>>,
    plan: Option<NativeResult<Ready<CheckpointPlan>>>,
    origin: Option<Origin>,
}
enum Step {
    Project(DataRows),
    Read(DataRows),
    Plan(Option<CheckpointRecord>),
    Complete(CheckpointRecord),
}
struct Progress {
    before: SaveOutcome,
    attempted: bool,
}
impl FileOperation {
    pub(super) fn checkpoint_save(
        &self,
        resources: &Resources,
        write: &CheckpointWrite,
        outcome: &mut SaveOutcome,
    ) -> NativeResult<CheckpointRecord> {
        let validated = self.checkpoint_validate(write)?;
        let mut state = State {
            origin: Some(validated),
            ..State::default()
        };
        loop {
            match self.checkpoint_save_step(resources, write, &mut state, outcome)? {
                Step::Project(rows) => {
                    let result = self.checkpoint_project(rows.clone());
                    state.origin = origin(&result).or(state.origin.take());
                    state.project = Some(Candidate { rows, result });
                    state.record = None;
                    state.plan = None;
                }
                Step::Read(rows) => {
                    let result =
                        self.checkpoint_read(&write.checkpoint.relative_path, false, rows.clone());
                    state.origin = origin(&result).or(state.origin.take());
                    state.record = Some(Candidate { rows, result });
                    state.plan = None;
                }
                Step::Plan(current) => {
                    let result = self.checkpoint_plan(write, current);
                    state.origin = origin(&result).or(state.origin.take());
                    state.plan = Some(result);
                }
                Step::Complete(record) => return Ok(record),
            }
        }
    }
    fn checkpoint_save_step(
        &self,
        resources: &Resources,
        write: &CheckpointWrite,
        state: &mut State,
        outcome: &mut SaveOutcome,
    ) -> NativeResult<Step> {
        let result = (|| {
            let mut guard = self
                .store
                .connection()
                .map_err(|cause| failure("storage_error", cause.to_string(), &self.key))?;
            guarded_transaction::run(&mut guard, |db| {
                self.checkpoint_save_in(db, resources, write, state, outcome)
            })
        })();
        result.map_err(|error| match &state.origin {
            Some(origin) => origin.attach(error),
            None => error,
        })
    }
    fn checkpoint_save_in(
        &self,
        db: &Connection,
        resources: &Resources,
        write: &CheckpointWrite,
        state: &State,
        outcome: &mut SaveOutcome,
    ) -> (NativeResult<Step>, bool) {
        let mut tx = match rusqlite::Transaction::new_unchecked(db, TransactionBehavior::Immediate)
        {
            Ok(tx) => tx,
            Err(cause) => {
                return (
                    Err(super::super::super::sql_error(
                        &self.key,
                        "checkpoint_begin",
                        &cause,
                    )),
                    !db.is_autocommit(),
                );
            }
        };
        tx.set_drop_behavior(DropBehavior::Ignore);
        let mut progress = Progress {
            before: outcome.clone(),
            attempted: false,
        };
        let result = (|| {
            let manuscript = self
                .resources
                .projects
                .get(&tx, &self.manuscript_id)
                .map_err(data_failure)?;
            let Some(candidate) = state
                .project
                .as_ref()
                .filter(|candidate| candidate.rows == manuscript)
            else {
                return Ok(Step::Project(manuscript));
            };
            candidate.result.clone()?;
            let rows = resources
                .read(&tx, &self.manuscript_id, &write.checkpoint.relative_path)
                .map_err(data_failure)?;
            let Some(candidate) = state
                .record
                .as_ref()
                .filter(|candidate| candidate.rows == rows)
            else {
                return Ok(Step::Read(rows));
            };
            let current = candidate.result.clone()?.value;
            let Some(candidate) = &state.plan else {
                return Ok(Step::Plan(current));
            };
            let ready = candidate.clone()?;
            let plan = ready.value;
            if !plan.write_required {
                return Ok(Step::Complete(plan.record));
            }
            let json = serde_json::to_string(&plan.record.checkpoint).map_err(|cause| {
                failure(
                    "storage_error",
                    format!("写作状态序列化失败：{cause}"),
                    &self.key,
                )
            })?;
            progress.attempted = true;
            *outcome = SaveOutcome::Unknown;
            resources
                .write(&tx, &plan.record, json)
                .map_err(data_failure)?;
            Ok(Step::Complete(plan.record))
        })();
        let step = match result {
            Ok(step) => step,
            Err(error) => {
                return self.checkpoint_rollback(db, error, tx.rollback(), progress, outcome);
            }
        };
        if !progress.attempted {
            return match tx.rollback() {
                Ok(()) if db.is_autocommit() => (Ok(step), false),
                result => self.checkpoint_rollback(
                    db,
                    failure(
                        "storage_error",
                        "writing_checkpoint_read_rollback_unsettled",
                        &self.key,
                    ),
                    result,
                    progress,
                    outcome,
                ),
            };
        }
        match tx.commit() {
            Ok(()) if db.is_autocommit() => {
                *outcome = SaveOutcome::Committed {
                    item_id: self.manuscript_id.clone(),
                };
                (Ok(step), false)
            }
            Ok(()) => {
                *outcome = SaveOutcome::Unknown;
                (
                    Err(failure(
                        "storage_error",
                        "writing_checkpoint_commit_unsettled",
                        &self.key,
                    )),
                    true,
                )
            }
            Err(cause) => {
                let error = super::super::super::sql_error(&self.key, "checkpoint_commit", &cause);
                if db.is_autocommit() {
                    *outcome = SaveOutcome::Unknown;
                    return (Err(error), true);
                }
                self.checkpoint_rollback(db, error, db.execute_batch("ROLLBACK"), progress, outcome)
            }
        }
    }
    fn checkpoint_rollback(
        &self,
        db: &Connection,
        mut error: Box<NativeHostError>,
        result: rusqlite::Result<()>,
        progress: Progress,
        outcome: &mut SaveOutcome,
    ) -> (NativeResult<Step>, bool) {
        let diagnostic = match result {
            Ok(()) if db.is_autocommit() => {
                *outcome = if !progress.attempted {
                    progress.before
                } else if progress.before == SaveOutcome::NotStarted {
                    SaveOutcome::RolledBack
                } else {
                    SaveOutcome::Unknown
                };
                return (Err(error), false);
            }
            Ok(()) => SafeDatabaseError::invariant(
                "writing_checkpoint_rollback_unsettled",
                "checkpoint_rollback",
            ),
            Err(cause) => SafeDatabaseError::sql("checkpoint_rollback", &cause),
        };
        *outcome = SaveOutcome::Unknown;
        error.save_rollback_error = Some(Box::new(diagnostic));
        (Err(error), true)
    }
}
