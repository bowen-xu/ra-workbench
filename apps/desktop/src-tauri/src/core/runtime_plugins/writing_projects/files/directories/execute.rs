//! Original Deferred relation transaction. Deferred probes stay at the original
//! source phase, after preceding writes, and never span a Native wait.
use super::super::super::{data_failure, failure, sql_error};
use super::probe::Relations;
use super::{DirectoryRuntime, PhysicalChange, value};
use crate::{
    data_interface::guarded_transaction,
    runtime_plugins::{
        literature_catalog::save::SaveOutcome,
        native::{NativeHostError, NativeResult, SafeDatabaseError},
    },
};
use research_workbench_native_sdk::{
    data::DataRows,
    writing_directory::{DirectoryPlan, DirectoryRequest},
};
use rusqlite::{Connection, DropBehavior, Transaction};

struct Failure {
    error: Box<NativeHostError>,
    rolled_back: bool,
}

impl DirectoryRuntime<'_> {
    pub(super) fn execute(
        &self,
        request: &DirectoryRequest,
        captured: &Relations,
        plan: &DirectoryPlan,
        physical: &PhysicalChange,
        outcome: &mut SaveOutcome,
    ) -> NativeResult<Option<DataRows>> {
        let mut guard =
            self.operation.store.connection().map_err(|error| {
                failure("storage_error", error.to_string(), &self.operation.key)
            })?;
        guarded_transaction::run(&mut guard, |db| {
            match self.persist(db, request, captured, plan) {
                Ok(rows) => {
                    *outcome = SaveOutcome::Committed {
                        item_id: self.operation.manuscript_id.clone(),
                    };
                    (Ok(rows), false)
                }
                Err(mut failed) => {
                    if failed.rolled_back {
                        match self.restore(request, physical) {
                            Ok(()) => *outcome = SaveOutcome::RolledBack,
                            Err(cause) => {
                                *outcome = SaveOutcome::Unknown;
                                failed.error.message.push_str(&format!(
                                    "；writing_directory_restore_failed：{cause}"
                                ));
                            }
                        }
                    } else {
                        *outcome = SaveOutcome::Unknown;
                    }
                    (
                        Err(failed.error),
                        !failed.rolled_back || !db.is_autocommit(),
                    )
                }
            }
        })
    }

    fn persist(
        &self,
        db: &Connection,
        request: &DirectoryRequest,
        captured: &Relations,
        plan: &DirectoryPlan,
    ) -> Result<Option<DataRows>, Failure> {
        if let Err(error) = value(&plan.paths, &self.operation.key) {
            return Err(Failure {
                error,
                rolled_back: true,
            });
        }
        let mut tx = db.unchecked_transaction().map_err(|cause| Failure {
            error: sql_error(&self.operation.key, "begin", &cause),
            rolled_back: db.is_autocommit(),
        })?;
        tx.set_drop_behavior(DropBehavior::Ignore);
        let result = self.relations_in(&tx, request, captured, plan);
        let error = match result {
            Ok(rows) => match tx.commit() {
                Ok(()) if db.is_autocommit() => return Ok(rows),
                Ok(()) => {
                    return Err(Failure {
                        error: failure(
                            "storage_error",
                            "writing_directory_transaction_unsettled",
                            &self.operation.key,
                        ),
                        rolled_back: false,
                    });
                }
                Err(cause) => {
                    let error = sql_error(&self.operation.key, "commit", &cause);
                    if db.is_autocommit() {
                        return Err(Failure {
                            error,
                            rolled_back: false,
                        });
                    }
                    error
                }
            },
            Err(error) => return Err(self.rolled_back(db, error, tx.rollback())),
        };
        Err(self.rolled_back(db, error, db.execute_batch("ROLLBACK")))
    }

    fn rolled_back(
        &self,
        db: &Connection,
        mut error: Box<NativeHostError>,
        rollback: rusqlite::Result<()>,
    ) -> Failure {
        let cause = match rollback {
            Ok(()) if db.is_autocommit() => {
                return Failure {
                    error,
                    rolled_back: true,
                };
            }
            Err(cause) => SafeDatabaseError::sql("rollback", &cause),
            Ok(()) => {
                SafeDatabaseError::invariant("writing_directory_transaction_unsettled", "rollback")
            }
        };
        error.save_rollback_error = Some(Box::new(cause));
        Failure {
            error,
            rolled_back: false,
        }
    }

    fn relations_in(
        &self,
        tx: &Transaction<'_>,
        request: &DirectoryRequest,
        captured: &Relations,
        plan: &DirectoryPlan,
    ) -> NativeResult<Option<DataRows>> {
        let id = &self.operation.manuscript_id;
        captured.manuscript.require(tx)?;
        let manuscript = value(&plan.manuscript, &self.operation.key)?;
        if manuscript.id != *id {
            return Err(failure(
                "plugin_host_response_mismatch",
                "目录稿件身份不匹配。",
                &self.operation.key,
            ));
        }
        captured.files.require(tx)?;
        for change in value(&plan.files, &self.operation.key)? {
            match request {
                DirectoryRequest::Rename { .. } => {
                    let target = change.target.as_deref().ok_or_else(|| {
                        failure(
                            "plugin_host_response_mismatch",
                            "目录索引改名缺少目标路径。",
                            &self.operation.key,
                        )
                    })?;
                    if self
                        .resources
                        .changes
                        .rename(tx, id, &change.path, target)
                        .map_err(data_failure)?
                        == 0
                    {
                        return Err(failure(
                            plan.on_unindexed_file.code(),
                            plan.on_unindexed_file.to_string(),
                            &self.operation.key,
                        ));
                    }
                }
                DirectoryRequest::Delete { .. } => self
                    .resources
                    .changes
                    .delete(tx, id, &change.path)
                    .map_err(data_failure)?,
            }
        }
        captured.drafts.require(tx)?;
        for change in value(&plan.drafts, &self.operation.key)? {
            if let Some(reason) = &change.rejection {
                return Err(failure(
                    reason.code(),
                    reason.to_string(),
                    &self.operation.key,
                ));
            }
            match request {
                DirectoryRequest::Rename { .. } => {
                    let target = change.target.as_deref().ok_or_else(|| {
                        failure(
                            "plugin_host_response_mismatch",
                            "目录草稿改名缺少目标路径。",
                            &self.operation.key,
                        )
                    })?;
                    self.resources
                        .rename_draft(tx, id, &change.path, target)
                        .map_err(data_failure)?;
                }
                DirectoryRequest::Delete { .. } => self
                    .resources
                    .delete_draft(tx, id, &change.path)
                    .map_err(data_failure)?,
            }
        }
        captured.session.require(tx)?;
        if let Some(draft) = value(&plan.session, &self.operation.key)? {
            self.resources.sessions.save(tx, draft)?;
        }
        match request {
            DirectoryRequest::Rename { .. } => {
                // The original metadata writer reads the manuscript again here.
                captured.manuscript.require(tx)?;
                let metadata = value(&plan.metadata, &self.operation.key)?
                    .as_ref()
                    .ok_or_else(|| {
                        failure(
                            "plugin_host_response_mismatch",
                            "目录改名缺少入口元数据。",
                            &self.operation.key,
                        )
                    })?;
                self.resources
                    .metadata
                    .write(tx, id, metadata)
                    .map_err(data_failure)?;
                self.operation
                    .resources
                    .projects
                    .get(tx, id)
                    .map(Some)
                    .map_err(data_failure)
            }
            DirectoryRequest::Delete { .. } => {
                self.resources.touch(tx, id).map_err(data_failure)?;
                Ok(None)
            }
        }
    }
}
