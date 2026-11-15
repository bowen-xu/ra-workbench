//! Original manuscript assets over the selected rules and Core byte owners.
pub(crate) mod commands;
use super::*;
use crate::{
    agent::writing_error::WritingError,
    ai::structured_resource_validation::{DecisionOrigin, Failure, FailureCause, Flow},
    data_interface::guarded_transaction,
    scoped_text_files::legacy,
    writing_resource_files, writing_transfer_files,
};
use bibliography_transaction::{self as transaction, Phases};
use research_workbench_native_sdk::writing_note_asset::{self, NoteAsset};
use std::{convert::Infallible, fs, path::Path};

pub(crate) enum Call {
    Import {
        display_name: String,
        mime_type: String,
        bytes: Vec<u8>,
    },
    Read {
        asset_id: String,
    },
    Export {
        asset_id: String,
        destination_path: String,
    },
}

impl FileOperation {
    pub(super) fn note_asset(
        &self,
        call: &Call,
        outcome: &mut SaveOutcome,
    ) -> NativeResult<FileResult> {
        let rows = {
            let db = self
                .store
                .connection()
                .map_err(|cause| self.asset_error(cause.into()))?;
            self.resources
                .projects
                .get(&db, &self.manuscript_id)
                .map_err(data_failure)?
        };
        let decided = self.decide(Input::GetProject {
            manuscript_id: self.manuscript_id.clone(),
            rows: rows.clone(),
        })?;
        let mut origin = DecisionOrigin {
            package_key: self.key.clone(),
            instance_id: decided.instance_id.clone(),
            native_request_id: decided.request_id.clone(),
        };
        project_reply(&self.key, decided)?;
        let result = (|| {
            if let Call::Import {
                display_name,
                mime_type,
                bytes,
            } = call
            {
                let decided = self.decide(Input::NoteAsset {
                    input: writing_note_asset::ImportInput {
                        display_name: display_name.clone(),
                        mime_type: mime_type.clone(),
                        byte_size: bytes.len() as u64,
                    },
                })?;
                origin.instance_id = decided.instance_id;
                origin.native_request_id = decided.request_id;
                match decided.output.decision {
                    Decision::NoteAsset {
                        decision: writing_note_asset::Decision::Validated,
                    } => {}
                    Decision::NoteAsset {
                        decision: writing_note_asset::Decision::Rejected { reason },
                    } => {
                        return Err(failure(reason.code(), reason.to_string(), &self.key));
                    }
                    _ => {
                        return Err(failure(
                            "plugin_host_response_mismatch",
                            "附件校验结果种类不匹配。",
                            &self.key,
                        ));
                    }
                }
            }
            {
                let db = self
                    .store
                    .connection()
                    .map_err(|cause| self.asset_error(cause.into()))?;
                self.resources
                    .projects
                    .recheck(&db, &self.manuscript_id, &rows)
                    .map_err(data_failure)?;
            }
            match call {
                Call::Import {
                    display_name,
                    mime_type,
                    bytes,
                } => self
                    .import_note_asset(display_name, mime_type, bytes, outcome)
                    .map(FileResult::NoteAsset),
                Call::Read { asset_id } => {
                    self.read_note_asset(asset_id).map(FileResult::AssetBytes)
                }
                Call::Export {
                    asset_id,
                    destination_path,
                } => {
                    let bytes = self.read_note_asset(asset_id)?;
                    let destination = Path::new(destination_path);
                    if !destination.is_absolute() || !destination.parent().is_some_and(Path::is_dir)
                    {
                        return Err(self.asset_error(WritingError::InvalidPath));
                    }
                    writing_transfer_files::reject_managed_destination(&self.store, destination)
                        .map_err(|cause| self.asset_error(cause))?;
                    writing_transfer_files::export_numbered(destination_path, true, |path| {
                        writing_transfer_files::write_external_atomically(Path::new(path), &bytes)?;
                        Ok(research_workbench_native_sdk::writing_project::ManuscriptExportResult {
                            destination_path: path.into(),
                            file_count: 1,
                            byte_size: bytes.len() as u64,
                        })
                    })
                    .map(FileResult::Exported)
                    .map_err(|cause| self.asset_error(cause))
                }
            }
        })();
        result.map_err(|mut error| {
            if error.request_id.is_none() && error.package_key.as_deref() == Some(&self.key) {
                error.instance_id = Some(origin.instance_id);
                error.request_id = Some(origin.native_request_id);
            }
            error
        })
    }

    fn asset_error(
        &self,
        cause: WritingError,
    ) -> Box<crate::runtime_plugins::native::NativeHostError> {
        failure(cause.code(), cause.to_string(), &self.key)
    }

    fn read_note_asset(&self, asset_id: &str) -> NativeResult<Vec<u8>> {
        writing_resource_files::read_note_asset(
            self.store.manuscript_directory(),
            &self.manuscript_id,
            asset_id,
        )
        .map_err(|cause| self.asset_error(cause))
    }

    fn import_note_asset(
        &self,
        display_name: &str,
        mime_type: &str,
        bytes: &[u8],
        outcome: &mut SaveOutcome,
    ) -> NativeResult<NoteAsset> {
        let asset_id = uuid::Uuid::new_v4().to_string();
        let relative =
            research_workbench_native_sdk::resource_path::writing_note_asset_path(&asset_id)
                .map_err(|_| self.asset_error(WritingError::InvalidPath))?;
        let root = legacy::manuscript_source_directory(
            self.store.manuscript_directory(),
            &self.manuscript_id,
        )
        .map_err(|cause| self.file_error(cause))?;
        let destination = writing_resource_files::create_note_asset(&root, &relative, bytes)
            .map_err(|cause| self.asset_error(cause))?;
        let registered = self.register_note_asset(&relative, bytes, outcome);
        if let Err(mut error) = registered {
            if !matches!(outcome, SaveOutcome::Unknown) {
                match fs::remove_file(&destination) {
                    Ok(()) => *outcome = SaveOutcome::RolledBack,
                    Err(cause) => {
                        *outcome = SaveOutcome::Unknown;
                        error.cleanup_error = Some(cause.to_string());
                    }
                }
            }
            return Err(error);
        }
        Ok(NoteAsset {
            display_name: crate::content::note_image_display_name(&asset_id, mime_type)
                .unwrap_or_else(|| display_name.into()),
            asset_id,
            mime_type: mime_type.into(),
        })
    }

    fn register_note_asset(
        &self,
        relative: &str,
        bytes: &[u8],
        outcome: &mut SaveOutcome,
    ) -> NativeResult<()> {
        let mut db = self
            .store
            .connection()
            .map_err(|cause| self.asset_error(cause.into()))?;
        let attempt = guarded_transaction::run(&mut db, |db| {
            let attempt = transaction::run::<_, Infallible>(
                db,
                Phases {
                    begin: "writing_note_asset_begin",
                    commit: "writing_note_asset_commit",
                    rollback: "writing_note_asset_rollback",
                    unsettled: "writing_note_asset_unsettled",
                },
                |tx| {
                    self.resources
                        .index(
                            tx,
                            &self.manuscript_id,
                            relative,
                            "other",
                            bytes.len(),
                            &legacy::sha256(bytes),
                        )
                        .map_err(|cause| {
                            Flow::Fail(Failure {
                                error: WritingError::Storage(
                                    crate::storage::StorageError::DataInterface(
                                        crate::data_interface::DataError::Unavailable,
                                    ),
                                ),
                                cause: FailureCause::SourceData(cause),
                            })
                        })
                },
            );
            let unknown = attempt.requires_recovery;
            (Ok::<_, Infallible>(attempt), unknown)
        })
        .expect("transaction result carries note asset settlement");
        if attempt.requires_recovery {
            *outcome = SaveOutcome::Unknown;
        }
        match attempt.result {
            Ok(()) => {
                *outcome = SaveOutcome::Committed {
                    item_id: self.manuscript_id.clone(),
                };
                Ok(())
            }
            Err(Flow::Need(never)) => match never {},
            Err(Flow::Fail(cause)) => {
                let mut error = match cause.cause {
                    FailureCause::SourceData(data) => data_failure(data),
                    FailureCause::SourceDatabase(data) => {
                        let mut error = failure("storage_error", data.to_string(), &self.key);
                        error.save_database_error = Some(data);
                        error
                    }
                    _ => self.asset_error(cause.error),
                };
                error.save_rollback_error = attempt.rollback_error.map(Box::new);
                Err(error)
            }
        }
    }
}
