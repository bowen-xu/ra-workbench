//! Original pending citation redo, under the caller's bibliography owner.
use super::super::{data_failure, decided_failure, failure};
use super::FileOperation;
use crate::{
    runtime_plugins::{literature_catalog::save::SaveOutcome, native::NativeResult},
    scoped_text_files::{TextFileError, legacy::sha256},
};
use research_workbench_native_sdk::writing_project::{Decision, Input};
impl FileOperation {
    pub(super) fn recover(&self, outcome: &mut SaveOutcome) -> NativeResult<()> {
        let captured = {
            let db = self
                .store
                .connection()
                .map_err(|error| failure("storage_error", error.to_string(), &self.key))?;
            self.resources
                .pending(&db, &self.manuscript_id)
                .map_err(data_failure)?
        };
        let decided = self.decide(Input::PrepareFileRecovery {
            manuscript_id: self.manuscript_id.clone(),
            pending: captured.clone(),
        })?;
        let recovery = match decided.output.decision {
            Decision::FileRecovery {
                manuscript_id,
                recovery,
            } if manuscript_id == self.manuscript_id => recovery,
            _ => {
                return Err(decided_failure(
                    "plugin_host_response_mismatch",
                    "书目恢复计划未保持原稿件身份。",
                    &self.key,
                    &decided.instance_id,
                    &decided.request_id,
                ));
            }
        };
        let result = (|| {
            {
                let db = self
                    .store
                    .connection()
                    .map_err(|error| failure("storage_error", error.to_string(), &self.key))?;
                self.resources
                    .recheck_pending(&db, &self.manuscript_id, &captured)
                    .map_err(data_failure)?;
            }
            let Some(recovery) = recovery else {
                return Ok(());
            };
            let files = self.store.manuscript_files(&self.manuscript_id);
            for write in &recovery.writes {
                let target = sha256(write.content.as_bytes());
                let (_, actual) = files
                    .read(&write.path)
                    .map_err(|error| self.file_error(error))?;
                if actual != target {
                    if actual != write.previous {
                        return Err(self.file_error(TextFileError::FileConflict));
                    }
                    *outcome = SaveOutcome::Unknown;
                    files
                        .save(&write.path, &write.content, Some(&write.previous))
                        .map_err(|error| self.file_error(error))?;
                }
                *outcome = SaveOutcome::Unknown;
                let mut db = self
                    .store
                    .connection()
                    .map_err(|error| failure("storage_error", error.to_string(), &self.key))?;
                self.resources
                    .index(
                        &db,
                        &self.manuscript_id,
                        &write.path,
                        &recovery.file_kind,
                        write.content.len(),
                        &target,
                    )
                    .map_err(|cause| {
                        if !db.is_autocommit() {
                            db.require_recovery();
                        }
                        data_failure(cause)
                    })?;
            }
            *outcome = SaveOutcome::Unknown;
            let mut db = self
                .store
                .connection()
                .map_err(|error| failure("storage_error", error.to_string(), &self.key))?;
            self.resources
                .clear(&db, &self.manuscript_id)
                .map_err(|cause| {
                    if !db.is_autocommit() {
                        db.require_recovery();
                    }
                    data_failure(cause)
                })?;
            *outcome = SaveOutcome::Committed {
                item_id: self.manuscript_id.clone(),
            };
            Ok(())
        })();
        result.map_err(|mut error| {
            error.instance_id = Some(decided.instance_id);
            error.request_id = Some(decided.request_id);
            error
        })
    }
}
