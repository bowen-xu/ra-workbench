//! Original per-file Deferred transaction, with pending-hash check first.
use super::super::super::{data_failure, failure, sql_error};
use super::super::catalog::CatalogDocument;
use super::{Operation, Origin};
use crate::{
    data_interface::guarded_transaction,
    runtime_plugins::{
        literature_catalog::save::SaveOutcome,
        native::{NativeHostError, NativeResult, SafeDatabaseError},
    },
};
use research_workbench_native_sdk::{
    data::DataValue,
    writing_catalog::{FileRead, ItemEffect, ItemPlan, PendingFile},
    writing_file::WritingNoteCatalogProjection,
};
use rusqlite::{Connection, DropBehavior, Transaction};

#[cfg(test)]
mod tests;

impl Operation {
    pub(super) fn publish(
        &self,
        file: &PendingFile,
        candidate: NativeResult<(ItemPlan, Origin)>,
        outcome: &mut SaveOutcome,
    ) -> NativeResult<bool> {
        let origin = match candidate.as_ref() {
            Ok((_, origin)) => Some(origin.clone()),
            Err(error) => error
                .instance_id
                .as_ref()
                .zip(error.request_id.as_ref())
                .map(|(instance_id, request_id)| Origin {
                    instance_id: instance_id.clone(),
                    request_id: request_id.clone(),
                }),
        };
        let result = (|| {
            let mut guard = self
                .store
                .connection()
                .map_err(|error| failure("storage_error", error.to_string(), &self.key))?;
            guarded_transaction::run(&mut guard, |db| {
                self.publish_in(db, file, candidate, outcome)
            })
        })();
        result.map_err(|error| match origin {
            Some(origin) => origin.attach(error),
            None => error,
        })
    }

    fn publish_in(
        &self,
        db: &Connection,
        file: &PendingFile,
        candidate: NativeResult<(ItemPlan, Origin)>,
        outcome: &mut SaveOutcome,
    ) -> (NativeResult<bool>, bool) {
        let earlier = *outcome != SaveOutcome::NotStarted;
        let mut tx = match db.unchecked_transaction() {
            Ok(tx) => tx,
            Err(cause) => {
                return (
                    Err(sql_error(&self.key, "catalog_begin", &cause)),
                    !db.is_autocommit(),
                );
            }
        };
        tx.set_drop_behavior(DropBehavior::Ignore);
        let current = self
            .resources
            .current(&tx, &file.manuscript_id, &file.path)
            .map_err(data_failure);
        let result = match current {
            Ok(rows) if rows.rows != [vec![DataValue::Text(file.hash.clone())]] => {
                return match tx.rollback() {
                    Ok(()) if db.is_autocommit() => (Ok(false), false),
                    rollback => self.rollback(
                        db,
                        failure(
                            "storage_error",
                            "writing_note_catalog_skip_rollback_unsettled",
                            &self.key,
                        ),
                        rollback,
                        outcome,
                    ),
                };
            }
            Ok(_) => candidate
                .and_then(|(plan, _)| self.apply(&tx, &plan, outcome))
                .map(|()| true),
            Err(error) => Err(error),
        };
        let error = match result {
            Ok(processed) => match tx.commit() {
                Ok(()) if db.is_autocommit() => return (Ok(processed), false),
                Ok(()) => {
                    *outcome = SaveOutcome::Unknown;
                    return (
                        Err(failure(
                            "storage_error",
                            "writing_note_catalog_transaction_unsettled",
                            &self.key,
                        )),
                        true,
                    );
                }
                Err(cause) => {
                    *outcome = SaveOutcome::Unknown;
                    let error = sql_error(&self.key, "catalog_commit", &cause);
                    if db.is_autocommit() {
                        return (Err(error), true);
                    }
                    error
                }
            },
            Err(error) => {
                let result = self.rollback(db, error, tx.rollback(), outcome);
                if !result.1 && !earlier && *outcome == SaveOutcome::Unknown {
                    *outcome = SaveOutcome::RolledBack;
                }
                return result;
            }
        };
        self.rollback(db, error, db.execute_batch("ROLLBACK"), outcome)
    }

    fn rollback(
        &self,
        db: &Connection,
        mut error: Box<NativeHostError>,
        rollback: rusqlite::Result<()>,
        outcome: &mut SaveOutcome,
    ) -> (NativeResult<bool>, bool) {
        let cause = match rollback {
            Ok(()) if db.is_autocommit() => return (Err(error), false),
            Err(cause) => SafeDatabaseError::sql("catalog_rollback", &cause),
            Ok(()) => SafeDatabaseError::invariant(
                "writing_note_catalog_transaction_unsettled",
                "catalog_rollback",
            ),
        };
        *outcome = SaveOutcome::Unknown;
        error.save_rollback_error = Some(Box::new(cause));
        (Err(error), true)
    }

    fn apply(
        &self,
        tx: &Transaction<'_>,
        plan: &ItemPlan,
        outcome: &mut SaveOutcome,
    ) -> NativeResult<()> {
        let input = &plan.preparation.input;
        let file = &input.file;
        match &plan.effect {
            ItemEffect::Record { projection } => {
                let FileRead::Read { content, hash } = &input.read else {
                    return Err(self.mismatch("目录正文投影缺少实际文件读取。"));
                };
                if !matches!(projection, WritingNoteCatalogProjection::Ignored) {
                    *outcome = SaveOutcome::Unknown;
                }
                self.resources
                    .catalog
                    .record_projection(
                        tx,
                        CatalogDocument {
                            manuscript_id: &file.manuscript_id,
                            path: &file.path,
                            content,
                            hash,
                        },
                        projection,
                        &plan.duplicate_identity_cause,
                    )
                    .map_err(|mut error| {
                        error.code = "index_failed".into();
                        error.message = format!(
                            "笔记索引操作失败：writing_note_catalog_prepare_failed：{}",
                            error.message
                        );
                        error
                    })
            }
            ItemEffect::ExternalChange {} => {
                *outcome = SaveOutcome::Unknown;
                self.resources
                    .external(tx, &file.manuscript_id, &file.path)
                    .map_err(data_failure)
            }
            ItemEffect::Error { cause } => {
                *outcome = SaveOutcome::Unknown;
                self.resources
                    .error(tx, &file.manuscript_id, &file.path, cause)
                    .map_err(data_failure)
            }
        }
    }
}
