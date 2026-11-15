use super::super::bibliography_adoption::{AdoptionBatch, AdoptionSql, adopt_in};
use super::*;
use crate::data_interface::guarded_transaction;
use research_workbench_native_sdk::{
    data::DataRows,
    writing_bibliography::adoption::{FileSelection, GeneratedHash},
};
use std::collections::BTreeMap;

#[derive(PartialEq)]
struct Snapshot {
    manuscript: DataRows,
    files: DataRows,
    known: Vec<String>,
    occupied: Vec<String>,
}
fn data(
    cause: Box<crate::data_interface::plugin_resources::PluginDataError>,
) -> Flow<WritingError, Need> {
    Flow::Fail(Failure {
        error: WritingError::Storage(crate::storage::StorageError::DataInterface(
            crate::data_interface::DataError::Unavailable,
        )),
        cause: FailureCause::SourceData(cause),
    })
}
impl Runtime<'_> {
    fn adoption_snapshot(&self, db: &Connection, manuscript: DataRows) -> Read<Snapshot> {
        let known = self
            .strings(db, "project-bibliography-adoption-known.v1.sql")
            .map_err(original)?;
        let mut occupied = self
            .strings(db, "project-bibliography-adoption-bound-keys.v1.sql")
            .map_err(original)?;
        occupied.extend(
            self.strings(db, "project-bibliography-adoption-occupied.v1.sql")
                .map_err(original)?,
        );
        let files = self
            .operation
            .resources
            .files(db, &self.operation.manuscript_id)
            .map_err(data)?;
        Ok(Snapshot {
            manuscript,
            files,
            known,
            occupied,
        })
    }
    pub(super) fn adopt(&self, outcome: &mut SaveOutcome) -> NativeResult<()> {
        self.adopt_with_originals(outcome, None)
    }
    pub(super) fn adopt_with_originals(
        &self,
        outcome: &mut SaveOutcome,
        originals: Option<&[(String, String)]>,
    ) -> NativeResult<()> {
        let mut reader = Reader::default();
        loop {
            reader.begin();
            let mut rollback_error = None;
            let result = (|| {
                let (snapshot, files) = {
                    let db = self
                        .operation
                        .store
                        .connection()
                        .map_err(|cause| original(WritingError::from(cause)))?;
                    let (_, manuscript) = self.project(&db, &mut reader)?;
                    let snapshot = self.adoption_snapshot(&db, manuscript)?;
                    let decision = reader.call(Input::ListFiles {
                        manuscript_id: self.operation.manuscript_id.clone(),
                        manuscript: snapshot.manuscript.clone(),
                        files: Some(snapshot.files.clone()),
                    })?;
                    let Decision::Files { files } = decision.as_ref() else {
                        return reader.mismatch();
                    };
                    (snapshot, files.clone())
                };
                let mut physical = Vec::new();
                let mut hashes = BTreeMap::new();
                for file in &files {
                    let known = snapshot.known.contains(&file.relative_path);
                    let selected = reader.adoption(AdoptionInput::SelectFile {
                        file: Box::new(file.clone()),
                        known,
                        generated: GeneratedHash::Unqueried,
                    })?;
                    let selected = match selected {
                        AdoptionDecision::Selected {
                            result: FileSelection::NeedGeneratedHash,
                        } => {
                            let hash = {
                                let db = self
                                    .operation
                                    .store
                                    .connection()
                                    .map_err(|cause| original(WritingError::from(cause)))?;
                                self.status_in(&db, &mut reader)?.content_hash
                            };
                            reader.adoption(AdoptionInput::SelectFile {
                                file: Box::new(file.clone()),
                                known,
                                generated: GeneratedHash::Ready { value: hash },
                            })?
                        }
                        other => other,
                    };
                    match selected {
                        AdoptionDecision::Selected {
                            result: FileSelection::Skip,
                        } => continue,
                        AdoptionDecision::Selected {
                            result: FileSelection::Read,
                        } => {}
                        _ => return reader.mismatch(),
                    }
                    let (text, hash) = self
                        .operation
                        .store
                        .manuscript_files(&self.operation.manuscript_id)
                        .read(&file.relative_path)
                        .map_err(|cause| original(WritingError::from(cause)))?;
                    hashes.insert(file.relative_path.clone(), hash);
                    physical.push((file.relative_path.clone(), text));
                }
                if physical.is_empty() {
                    return Ok(());
                }
                let AdoptionDecision::Parsed { files: parsed } =
                    reader.adoption(AdoptionInput::Parse {
                        physical_files: physical,
                        originals: originals.map(<[_]>::to_vec),
                        occupied: snapshot.occupied.clone(),
                    })?
                else {
                    return reader.mismatch();
                };
                let mut guard = self
                    .operation
                    .store
                    .connection()
                    .map_err(|cause| original(WritingError::from(cause)))?;
                let manuscript = self
                    .operation
                    .resources
                    .projects
                    .get(&guard, &self.operation.manuscript_id)
                    .map_err(data)?;
                if manuscript != snapshot.manuscript
                    || self.adoption_snapshot(&guard, manuscript)? != snapshot
                {
                    return Err(Flow::Need(Need::Recapture));
                }
                let sql = AdoptionSql {
                    registered: self
                        .sql
                        .get("project-bibliography-adoption-registered.v1.sql"),
                    insert_file: self
                        .sql
                        .get("project-bibliography-adoption-insert-file.v1.sql"),
                    insert_entry: self
                        .sql
                        .get("project-bibliography-adoption-insert-entry.v1.sql"),
                    insert_member: self
                        .sql
                        .get("project-bibliography-adoption-insert-member.v1.sql"),
                };
                let attempt = guarded_transaction::run(&mut guard, |db| {
                    let changes = db.total_changes();
                    let shared = RefCell::new(&mut reader);
                    let attempt = adopt_in(
                        db,
                        &sql,
                        &AdoptionBatch {
                            manuscript_id: &self.operation.manuscript_id,
                            parsed: &parsed,
                            originals,
                            files: &files,
                        },
                        &mut |path| {
                            let (_, hash) = self
                                .operation
                                .store
                                .manuscript_files(&self.operation.manuscript_id)
                                .read(path)?;
                            if hashes.get(path) != Some(&hash) {
                                return Err(WritingError::FileConflict);
                            }
                            Ok(hash)
                        },
                        &mut |input| shared.borrow_mut().adoption(input),
                        &mut |db, draft| {
                            if self.library_available() {
                                self.match_library(db, &mut shared.borrow_mut(), draft)
                            } else {
                                Ok(None)
                            }
                        },
                    );
                    if attempt.requires_recovery {
                        *outcome = SaveOutcome::Unknown;
                    } else if attempt.result.is_ok() && db.total_changes() != changes {
                        *outcome = SaveOutcome::Committed {
                            item_id: self.operation.manuscript_id.clone(),
                        };
                    } else if matches!(attempt.result, Err(Flow::Fail(_)))
                        && *outcome == SaveOutcome::NotStarted
                    {
                        *outcome = SaveOutcome::RolledBack;
                    }
                    let unknown = attempt.requires_recovery;
                    (Ok::<_, std::convert::Infallible>(attempt), unknown)
                })
                .expect("transaction result carries the complete finite outcome");
                rollback_error = attempt.rollback_error;
                attempt.result
            })();
            match result {
                Ok(()) => return Ok(()),
                Err(Flow::Fail(cause)) => {
                    let mut error = self.failed(cause, &reader);
                    error.save_rollback_error = rollback_error.map(Box::new);
                    return Err(error);
                }
                Err(Flow::Need(need)) => self.resume(&mut reader, need)?,
            }
        }
    }
}
