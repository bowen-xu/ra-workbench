//! Original import order and compensation over the shared Core file executor.
pub(crate) mod commands;
use super::*;
use crate::runtime_plugins::writing_projects::project_reply;
use crate::{
    data_interface::guarded_transaction, scoped_text_files::legacy, writing_transfer_files,
};
use research_workbench_native_sdk::{
    writing_project::{
        ManuscriptDraft, ManuscriptTransferResult, MetadataDecision, ProjectScope,
        STANDALONE_PROJECT_ID,
    },
    writing_transfer::{self, SourceInspection},
};
use std::{fs, path::Path};

impl Runtime<'_> {
    fn transfer(&self, input: writing_transfer::Input) -> NativeResult<writing_transfer::Decision> {
        let decided = self.operation.decide(Input::Transfer {
            input: Box::new(input),
        })?;
        self.remember(&decided);
        match decided.output.decision {
            Decision::Transfer { decision } => match *decision {
                writing_transfer::Decision::Rejected { code, message } => {
                    let mut error = failure(code.as_str(), message, &self.operation.key);
                    self.attach(&mut error);
                    Err(error)
                }
                decision => Ok(decision),
            },
            _ => Err(self.mismatch()),
        }
    }

    fn project_by_id(&self, id: &str) -> NativeResult<Manuscript> {
        let rows = {
            let db = self
                .operation
                .store
                .connection()
                .map_err(|cause| self.error(cause.into()))?;
            self.operation
                .resources
                .projects
                .get(&db, id)
                .map_err(data_failure)?
        };
        let decided = self.operation.decide(Input::GetProject {
            manuscript_id: id.into(),
            rows,
        })?;
        self.remember(&decided);
        project_reply(&self.operation.key, decided)
    }

    pub(super) fn duplicate_project(
        &self,
        source_id: &str,
        outcome: &mut SaveOutcome,
    ) -> NativeResult<ManuscriptTransferResult> {
        let source = self.project_by_id(source_id)?;
        let directory = legacy::manuscript_source_directory(
            self.operation.store.manuscript_directory(),
            &source.id,
        )
        .map_err(|cause| self.file_error(cause))?;
        let writing_transfer::Decision::Title { title } =
            self.transfer(writing_transfer::Input::DuplicateTitle {
                title: source.title,
            })?
        else {
            return Err(self.mismatch());
        };
        self.import_project(
            &title,
            &directory.to_string_lossy(),
            Some(&source.root_path),
            outcome,
        )
    }

    pub(super) fn import_project(
        &self,
        title: &str,
        source: &str,
        requested: Option<&str>,
        outcome: &mut SaveOutcome,
    ) -> NativeResult<ManuscriptTransferResult> {
        self.import_project_in(title, source, requested, outcome)
            .map_err(|mut error| {
                if matches!(outcome, SaveOutcome::Unknown) {
                    return error;
                }
                match self.cleanup_import() {
                    Ok(()) => *outcome = SaveOutcome::RolledBack,
                    Err(cleanup) => {
                        *outcome = SaveOutcome::Unknown;
                        error.cleanup_error =
                            Some(format!("{}: {}", cleanup.code, cleanup.message));
                    }
                }
                error.save_outcome = Some(outcome.clone());
                error
            })
    }

    fn import_project_in(
        &self,
        title: &str,
        source: &str,
        requested: Option<&str>,
        outcome: &mut SaveOutcome,
    ) -> NativeResult<ManuscriptTransferResult> {
        self.insert_import(title, outcome)?;
        let root = legacy::manuscript_source_directory(
            self.operation.store.manuscript_directory(),
            &self.operation.manuscript_id,
        )
        .map_err(|cause| self.file_error(cause))?;
        let inspected = self.transfer(writing_transfer::Input::InspectImport {
            source_path: source.into(),
        })?;
        let source_path =
            fs::canonicalize(source.trim()).map_err(|cause| self.error(cause.into()))?;
        let inspection = match inspected {
            writing_transfer::Decision::Directory { inspection } => {
                writing_transfer_files::copy_directory(&source_path, &root, &inspection)
                    .map_err(|cause| self.error(cause))?;
                inspection
            }
            writing_transfer::Decision::Zip { entries, headers } => {
                let mut archive = zip::ZipArchive::new(
                    fs::File::open(source_path).map_err(|cause| self.error(cause.into()))?,
                )
                .map_err(|cause| self.error(cause.into()))?;
                writing_transfer_files::verify_zip_headers(&mut archive, &headers)
                    .map_err(|cause| self.error(cause))?;
                writing_transfer_files::copy_zip(&mut archive, &root, &entries)
                    .map_err(|cause| self.error(cause))?;
                SourceInspection {
                    byte_size: entries.iter().map(|entry| entry.byte_size).sum(),
                    files: entries
                        .into_iter()
                        .map(|entry| entry.relative_path)
                        .collect(),
                    root_candidates: vec![],
                }
            }
            _ => return Err(self.mismatch()),
        };
        let writing_transfer::Decision::Roots { candidates } =
            self.transfer(writing_transfer::Input::DiscoverRoots {
                directory: root.to_string_lossy().into(),
                files: inspection.files.clone(),
            })?
        else {
            return Err(self.mismatch());
        };
        let originals = self.normalize_project(&root, &inspection.files)?;
        let writing_transfer::Decision::Root { relative_path } =
            self.transfer(writing_transfer::Input::SelectRoot {
                candidates: candidates.clone(),
                requested: requested.map(str::to_owned),
            })?
        else {
            return Err(self.mismatch());
        };
        if !candidates.contains(&relative_path) {
            return Err(self.mismatch());
        }
        legacy::validate_relative_path(&relative_path).map_err(|cause| self.file_error(cause))?;
        let manuscript = self.register_import(&root, &inspection.files, &relative_path, outcome)?;
        self.adopt_with_originals(outcome, Some(&originals))?;
        self.regenerate(false, outcome)?;
        Ok(ManuscriptTransferResult {
            manuscript,
            file_count: inspection.files.len(),
            root_candidates: candidates,
        })
    }

    fn insert_import(&self, title: &str, outcome: &mut SaveOutcome) -> NativeResult<()> {
        let id = &self.operation.manuscript_id;
        let decided = self.operation.decide(Input::PrepareCreation {
            manuscript_id: id.clone(),
            draft: ManuscriptDraft {
                project_id: None,
                title: title.into(),
                root_path: None,
            },
        })?;
        self.remember(&decided);
        let Decision::Creation { plan } = decided.output.decision else {
            return Err(self.mismatch());
        };
        if plan.manuscript_id != *id
            || plan.scope
                != (ProjectScope::Standalone {
                    project_id: STANDALONE_PROJECT_ID.into(),
                })
        {
            return Err(self.mismatch());
        }
        let mut db = self
            .operation
            .store
            .connection()
            .map_err(|cause| self.error(cause.into()))?;
        guarded_transaction::run(&mut db, |db| {
            let result = (|| {
                db.execute(
                    self.sql.get("project-standalone-upsert.v1.sql"),
                    [STANDALONE_PROJECT_ID],
                )
                .map_err(|cause| self.error(cause.into()))?;
                let metadata = match &plan.metadata {
                    MetadataDecision::Ready { metadata }
                        if metadata.title == title.trim() && metadata.root_path == "main.tex" =>
                    {
                        metadata
                    }
                    MetadataDecision::Rejected { reason } => {
                        return Err(self.error(reason.clone().into()));
                    }
                    _ => return Err(self.mismatch()),
                };
                db.execute(
                    self.sql.get("project-insert.v1.sql"),
                    rusqlite::params![
                        id,
                        STANDALONE_PROJECT_ID,
                        metadata.title,
                        metadata.root_path
                    ],
                )
                .map_err(|cause| self.error(cause.into()))?;
                Ok(())
            })();
            let unknown = !db.is_autocommit();
            if unknown {
                *outcome = SaveOutcome::Unknown;
            } else if result.is_ok() {
                *outcome = SaveOutcome::Committed {
                    item_id: id.clone(),
                };
            }
            (result, unknown)
        })
    }

    fn normalize_project(
        &self,
        root: &Path,
        files: &[String],
    ) -> NativeResult<Vec<(String, String)>> {
        let kinds = self.operation.decide(Input::PrepareAgentFileChanges {
            relative_paths: files.to_vec(),
        })?;
        self.remember(&kinds);
        let Decision::AgentFileChangesPrepared { policies } = kinds.output.decision else {
            return Err(self.mismatch());
        };
        if policies
            .iter()
            .map(|policy| &policy.relative_path)
            .ne(files.iter())
        {
            return Err(self.mismatch());
        }
        let sources = policies
            .iter()
            .filter(|policy| policy.file_kind == "bib")
            .map(|policy| {
                fs::read_to_string(root.join(&policy.relative_path))
                    .map(|text| (policy.relative_path.clone(), text))
                    .map_err(|cause| self.error(cause.into()))
            })
            .collect::<NativeResult<Vec<_>>>()?;
        if sources.is_empty() {
            return Ok(sources);
        }
        let AdoptionDecision::ResourcesNormalized { plan } =
            self.adoption_decision(AdoptionInput::NormalizeResources {
                sources: sources.clone(),
                occupied: vec![],
            })?
        else {
            return Err(self.mismatch());
        };
        if plan.mapping.is_empty() {
            return Ok(sources);
        }
        let mut changes = Vec::new();
        for policy in policies.iter().filter(|policy| policy.file_kind == "tex") {
            let source = fs::read_to_string(root.join(&policy.relative_path))
                .map_err(|cause| self.error(cause.into()))?;
            let AdoptionDecision::ResourceSource { text } =
                self.adoption_decision(AdoptionInput::ResourceSource {
                    source,
                    mapping: plan.mapping.clone(),
                    existing: false,
                })?
            else {
                return Err(self.mismatch());
            };
            changes.push((policy.relative_path.clone(), text));
        }
        changes.extend(plan.files);
        for (path, text) in changes {
            let relative =
                legacy::validate_relative_path(&path).map_err(|cause| self.file_error(cause))?;
            fs::write(root.join(relative), text).map_err(|cause| self.error(cause.into()))?;
        }
        Ok(sources)
    }

    fn register_import(
        &self,
        root: &Path,
        files: &[String],
        relative_path: &str,
        outcome: &mut SaveOutcome,
    ) -> NativeResult<Manuscript> {
        let kinds = self.operation.decide(Input::PrepareAgentFileChanges {
            relative_paths: files.to_vec(),
        })?;
        self.remember(&kinds);
        let Decision::AgentFileChangesPrepared { policies } = kinds.output.decision else {
            return Err(self.mismatch());
        };
        if policies
            .iter()
            .map(|policy| &policy.relative_path)
            .ne(files.iter())
        {
            return Err(self.mismatch());
        }
        let (manuscript, captured) = self.read(|db, reader| self.project(db, reader))?;
        {
            let mut db = self
                .operation
                .store
                .connection()
                .map_err(|cause| self.error(cause.into()))?;
            guarded_transaction::run(&mut db, |db| {
                let result: NativeResult<()> = (|| {
                    self.operation
                        .resources
                        .projects
                        .recheck(db, &self.operation.manuscript_id, &captured)
                        .map_err(data_failure)?;
                    for policy in &policies {
                        let bytes = fs::read(root.join(&policy.relative_path))
                            .map_err(|cause| self.error(cause.into()))?;
                        self.operation
                            .resources
                            .index(
                                db,
                                &self.operation.manuscript_id,
                                &policy.relative_path,
                                &policy.file_kind,
                                bytes.len(),
                                &legacy::sha256(&bytes),
                            )
                            .map_err(data_failure)?;
                    }
                    db.execute(
                        self.sql.get("project-update.v1.sql"),
                        rusqlite::params![
                            self.operation.manuscript_id,
                            manuscript.title,
                            relative_path
                        ],
                    )
                    .map_err(|cause| self.error(cause.into()))?;
                    Ok(())
                })();
                let unknown = !db.is_autocommit();
                if unknown {
                    *outcome = SaveOutcome::Unknown;
                }
                (result, unknown)
            })?;
        }
        self.project_by_id(&self.operation.manuscript_id)
    }

    fn cleanup_import(&self) -> NativeResult<()> {
        let root = legacy::manuscript_source_directory(
            self.operation.store.manuscript_directory(),
            &self.operation.manuscript_id,
        )
        .map_err(|cause| self.file_error(cause))?;
        let directory = root
            .parent()
            .ok_or_else(|| self.error(WritingError::InvalidPath))?;
        if directory.exists() {
            fs::remove_dir_all(directory).map_err(|cause| self.error(cause.into()))?;
        }
        let mut db = self
            .operation
            .store
            .connection()
            .map_err(|cause| self.error(cause.into()))?;
        guarded_transaction::run(&mut db, |db| {
            let result = db
                .execute(
                    self.sql.get("project-delete.v1.sql"),
                    [&self.operation.manuscript_id],
                )
                .map(|_| ())
                .map_err(|cause| self.error(cause.into()));
            (result, !db.is_autocommit())
        })
    }
}
