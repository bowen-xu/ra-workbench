use super::*;
use research_workbench_native_sdk::{
    data::DataRows,
    writing_project::{BibliographyStatus, CitationBinding, CitationMetadataKind},
};
use rusqlite::params;

impl Runtime<'_> {
    pub(super) fn project(
        &self,
        db: &Connection,
        reader: &mut Reader,
    ) -> Read<(Manuscript, DataRows)> {
        let rows = self
            .operation
            .resources
            .projects
            .get(db, &self.operation.manuscript_id)
            .map_err(|cause| {
                Flow::Fail(Failure {
                    error: WritingError::Storage(crate::storage::StorageError::DataInterface(
                        crate::data_interface::DataError::Unavailable,
                    )),
                    cause: FailureCause::SourceData(cause),
                })
            })?;
        let decision = reader.call(Input::GetProject {
            manuscript_id: self.operation.manuscript_id.clone(),
            rows: rows.clone(),
        })?;
        let Decision::Project { manuscript } = decision.as_ref() else {
            return reader.mismatch();
        };
        if manuscript.id != self.operation.manuscript_id {
            return reader.mismatch();
        }
        Ok((manuscript.clone(), rows))
    }
    pub(super) fn files(
        &self,
        db: &Connection,
        reader: &mut Reader,
        manuscript: DataRows,
    ) -> Read<Vec<ManuscriptFile>> {
        let files = self
            .operation
            .resources
            .files(db, &self.operation.manuscript_id)
            .map_err(|cause| {
                Flow::Fail(Failure {
                    error: WritingError::Storage(crate::storage::StorageError::DataInterface(
                        crate::data_interface::DataError::Unavailable,
                    )),
                    cause: FailureCause::SourceData(cause),
                })
            })?;
        match reader
            .call(Input::ListFiles {
                manuscript_id: self.operation.manuscript_id.clone(),
                manuscript,
                files: Some(files),
            })?
            .as_ref()
        {
            Decision::Files { files } => Ok(files.clone()),
            _ => reader.mismatch(),
        }
    }
    pub(super) fn bindings(&self, db: &Connection) -> Result<Vec<CitationBinding>, WritingError> {
        self.sql
            .prepare(db, "project-citation-bindings.v1.sql")?
            .query_map([&self.operation.manuscript_id], |row| {
                Ok(CitationBinding {
                    manuscript_id: row.get(0)?,
                    literature_item_id: row.get(1)?,
                    citation_key: row.get(2)?,
                    bibtex_entry: row.get(3)?,
                    state: row.get(4)?,
                    pinned_at: row.get(5)?,
                    updated_at: row.get(6)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(Into::into)
    }
    pub(super) fn strings(&self, db: &Connection, name: &str) -> Result<Vec<String>, WritingError> {
        self.sql
            .prepare(db, name)?
            .query_map([&self.operation.manuscript_id], |row| row.get(0))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(Into::into)
    }
    pub(super) fn status_in(
        &self,
        db: &Connection,
        reader: &mut Reader,
    ) -> Read<BibliographyStatus> {
        let (_, manuscript) = self.project(db, reader)?;
        let rows = self
            .sql
            .prepare(db, "project-bibliography-status.v1.sql")
            .map_err(original)?
            .query_map([&self.operation.manuscript_id], |row| {
                Ok(vec![
                    research_workbench_native_sdk::data::DataValue::Text(row.get(0)?),
                    row.get::<_, Option<String>>(1)?.map_or(
                        research_workbench_native_sdk::data::DataValue::Null,
                        research_workbench_native_sdk::data::DataValue::Text,
                    ),
                    row.get::<_, Option<String>>(2)?.map_or(
                        research_workbench_native_sdk::data::DataValue::Null,
                        research_workbench_native_sdk::data::DataValue::Text,
                    ),
                    research_workbench_native_sdk::data::DataValue::Text(row.get(3)?),
                ])
            })
            .map_err(original)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(original)?;
        match reader
            .call(Input::CitationMetadata {
                metadata_kind: CitationMetadataKind::BibliographyStatus,
                manuscript_id: self.operation.manuscript_id.clone(),
                manuscript,
                data: Some(DataRows { rows }),
            })?
            .as_ref()
        {
            Decision::BibliographyStatus { status } => Ok(status.clone()),
            _ => reader.mismatch(),
        }
    }
    pub(super) fn entries(&self) -> NativeResult<Vec<ProjectBibliographyEntry>> {
        self.read(|db, reader| {
            let shared = RefCell::new(reader);
            let sql = bibliography_entries::EntrySql {
                rows: self.sql.get("project-bibliography-entry-rows.v1.sql"),
                support: self.sql.get("project-bibliography-entry-support.v1.sql"),
                members: self.sql.get("project-bibliography-entry-members.v1.sql"),
                base: self.sql.get("project-bibliography-entry-base.v1.sql"),
                bindings: self.sql.get("project-citation-bindings.v1.sql"),
            };
            bibliography_entries::read_in(
                db,
                &sql,
                &self.operation.manuscript_id,
                &mut |input| {
                    let mut reader = shared.borrow_mut();
                    match reader
                        .call(Input::BibliographyEntries {
                            input: Box::new(input),
                        })?
                        .as_ref()
                    {
                        Decision::BibliographyEntries { decision } => Ok(*decision.clone()),
                        _ => reader.mismatch(),
                    }
                },
                &mut |db, source, entry| {
                    if self.library_available() {
                        return self
                            .library_item(db, &mut shared.borrow_mut(), source)
                            .map(Some);
                    }
                    let Some(entry) = entry else {
                        return Ok(None);
                    };
                    let json: Option<String> = self
                        .sql
                        .prepare(db, "project-bibliography-entry-library-snapshot.v1.sql")
                        .map_err(original)?
                        .query_row([entry], |row| row.get(0))
                        .map_err(original)?;
                    json.as_deref()
                        .map(serde_json::from_str)
                        .transpose()
                        .map_err(original)
                },
            )
        })
    }
    pub(super) fn retained(&self) -> NativeResult<Vec<String>> {
        self.retained_with_imports(true)
    }
    pub(super) fn retained_with_imports(
        &self,
        preserve_imported: bool,
    ) -> NativeResult<Vec<String>> {
        let (files, root, imported, automatic, decision) = self.read(|db, reader| {
            let (project, manuscript) = self.project(db, reader)?;
            let imported = self
                .sql
                .prepare(db, "project-bibliography-retained-imports.v1.sql")
                .map_err(original)?
                .query_map([&self.operation.manuscript_id], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })
                .map_err(original)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(original)?;
            let automatic = self
                .bindings(db)
                .map_err(original)?
                .into_iter()
                .map(|binding| (binding.citation_key, binding.bibtex_entry))
                .collect::<Vec<_>>();
            let decision = reader.publication(PublicationInput::Retention {
                sources: None,
                automatic: automatic.clone(),
                imported: imported.clone(),
                preserve_imported,
            })?;
            let files = if matches!(decision, PublicationDecision::NeedSources) {
                self.files(db, reader, manuscript)?
            } else {
                vec![]
            };
            Ok((files, project.root_path, imported, automatic, decision))
        })?;
        match decision {
            PublicationDecision::Retained { keys } => Ok(keys),
            PublicationDecision::NeedSources => {
                let PublicationDecision::SourcePaths { paths } =
                    self.publication_decision(PublicationInput::SourcePaths {
                        files,
                        root_path: root,
                    })?
                else {
                    return Err(self.mismatch());
                };
                let sources = paths
                    .iter()
                    .map(|path| {
                        self.operation
                            .store
                            .manuscript_files(&self.operation.manuscript_id)
                            .read(path)
                            .map(|(text, _)| text)
                            .map_err(|cause| self.file_error(cause))
                    })
                    .collect::<NativeResult<Vec<_>>>()?;
                match self.publication_decision(PublicationInput::Retention {
                    sources: Some(sources),
                    automatic,
                    imported,
                    preserve_imported,
                })? {
                    PublicationDecision::Retained { keys } => Ok(keys),
                    _ => Err(self.mismatch()),
                }
            }
            _ => Err(self.mismatch()),
        }
    }
    pub(super) fn record_status(
        &self,
        db: &Connection,
        path: &str,
        hash: Option<&str>,
        status: &str,
    ) -> NativeResult<()> {
        db.execute(
            self.sql.get("project-bibliography-record-status.v1.sql"),
            params![self.operation.manuscript_id, hash, status, path],
        )
        .map_err(|cause| self.error(cause.into()))?;
        Ok(())
    }
    pub(super) fn write(
        &self,
        outcome: &mut SaveOutcome,
        execute: impl FnOnce(&Connection) -> NativeResult<()>,
    ) -> NativeResult<()> {
        let mut db =
            self.operation.store.connection().map_err(|cause| {
                failure("storage_error", cause.to_string(), &self.operation.key)
            })?;
        *outcome = SaveOutcome::Unknown;
        execute(&db).inspect_err(|_| {
            if !db.is_autocommit() {
                db.require_recovery();
            }
        })
    }
}
