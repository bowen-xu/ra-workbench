//! Original full bibliography list in the owned file operation.
mod adoption;
pub(crate) mod binding;
mod decisions;
mod library;
pub(crate) mod mutation;
pub(crate) mod project_transfer;
mod proof;
mod publication;
mod read;
pub(crate) mod recovery;
pub(crate) mod rekey;
pub(crate) mod resource_import;
mod resources;
pub(crate) mod search;
pub(crate) mod sync;
use super::super::{data_failure, failure, sql_error};
use super::{FileOperation, FileRequest, FileResult, bibliography_entries};
use crate::{
    agent::writing_error::WritingError,
    ai::structured_resource_validation::{Failure, FailureCause, Flow},
    runtime_plugins::{
        RuntimePluginCatalog,
        literature_catalog::save::SaveOutcome,
        native::{NativeHostError, NativePreviewService, NativeResult},
    },
    storage::AppStore,
};
use decisions::Reader;
use research_workbench_native_sdk::{
    writing_bibliography::adoption::{AdoptionDecision, AdoptionInput, PublicationChoice},
    writing_bibliography::*,
    writing_project::{Decision, Input, Manuscript, ManuscriptFile},
};
pub(super) use resources::Sql;
use rusqlite::Connection;
use std::cell::RefCell;
use std::sync::Arc;
pub(crate) enum Call {
    ImportProject {
        title: String,
        source_path: String,
        requested_root: Option<String>,
    },
    DuplicateProject {
        source_id: String,
    },
    ImportResources {
        destination_directory: String,
        source_paths: Vec<String>,
    },
    Regenerate,
    List {
        include_unused_automatic: bool,
    },
    Restore,
    Sync {
        entries: Vec<BibliographyEntryInput>,
    },
    Rekey {
        assignments: Vec<research_workbench_native_sdk::writing_bibliography::rekey::Assignment>,
    },
    Update {
        entry_id: String,
        expected: String,
        draft: serde_json::Value,
    },
    Link {
        entry_id: String,
        item_id: String,
    },
    RemoveUnused,
    Bind {
        entry: BibliographyEntryInput,
    },
    Search {
        query: String,
        scope: String,
    },
}

enum Need {
    Native(Box<Input>),
    LibraryResources,
    Library(Box<research_workbench_native_sdk::catalog::CatalogInput>),
    Recapture,
}
type Read<T> = Result<T, Flow<WritingError, Need>>;
fn original<E: Into<WritingError>>(cause: E) -> Flow<WritingError, Need> {
    Flow::original(cause.into())
}

struct Runtime<'a> {
    operation: &'a FileOperation,
    sql: &'a Sql,
    last: RefCell<Option<(String, String)>>,
    library: RefCell<Option<crate::runtime_plugins::literature_bibliography::Resources>>,
}

impl FileOperation {
    pub(super) fn bibliography(
        &self,
        sql: &Sql,
        call: &Call,
        outcome: &mut SaveOutcome,
    ) -> NativeResult<FileResult> {
        let runtime = Runtime {
            operation: self,
            sql,
            last: RefCell::new(None),
            library: RefCell::new(None),
        };
        let include_unused = match call {
            Call::ImportProject {
                title,
                source_path,
                requested_root,
            } => {
                return runtime
                    .import_project(title, source_path, requested_root.as_deref(), outcome)
                    .map(FileResult::ProjectImported);
            }
            Call::DuplicateProject { source_id } => {
                return runtime
                    .duplicate_project(source_id, outcome)
                    .map(FileResult::ProjectImported);
            }
            Call::ImportResources {
                destination_directory,
                source_paths,
            } => {
                return runtime
                    .import_resources(destination_directory, source_paths, outcome)
                    .map(FileResult::ResourcesImported);
            }
            Call::Search { query, scope } => {
                return runtime
                    .search(query, scope)
                    .map(FileResult::CitationCandidates);
            }
            Call::Bind { entry } => {
                return runtime
                    .bind(entry, outcome)
                    .map(FileResult::CitationBinding);
            }
            Call::Update {
                entry_id,
                expected,
                draft,
            } => {
                runtime.update(entry_id, expected, draft, outcome)?;
                return Ok(FileResult::BibliographyChanged);
            }
            Call::Link { entry_id, item_id } => {
                runtime.link(entry_id, item_id, outcome)?;
                return Ok(FileResult::BibliographyChanged);
            }
            Call::RemoveUnused => {
                return runtime
                    .remove_unused(outcome)
                    .map(FileResult::RemovedBibliographyEntries);
            }
            Call::Rekey { assignments } => {
                return runtime
                    .migrate(assignments, outcome)
                    .map(|migration| FileResult::CitationKeyMigration(Box::new(migration)));
            }
            Call::Sync { entries } => {
                return runtime
                    .sync(entries, outcome)
                    .map(FileResult::BibliographyStatus);
            }
            Call::Restore => return runtime.restore(outcome).map(FileResult::BibliographyStatus),
            Call::Regenerate => {
                return runtime
                    .regenerate(false, outcome)
                    .map(FileResult::BibliographyStatus);
            }
            Call::List {
                include_unused_automatic,
            } => *include_unused_automatic,
        };
        runtime.adopt(outcome)?;
        let choice = runtime.adoption_decision(AdoptionInput::Publication {
            always: false,
            proof_current: None,
        })?;
        let choice = if matches!(
            choice,
            AdoptionDecision::PublicationChosen {
                result: PublicationChoice::NeedProof
            }
        ) {
            let current = runtime.fingerprint()?.is_some_and(|proof| {
                self.store
                    .publication_proofs()
                    .current(&self.manuscript_id, &proof)
            });
            runtime.adoption_decision(AdoptionInput::Publication {
                always: false,
                proof_current: Some(current),
            })?
        } else {
            choice
        };
        match choice {
            AdoptionDecision::PublicationChosen {
                result: PublicationChoice::Publish,
            } => runtime.publish(outcome)?,
            AdoptionDecision::PublicationChosen {
                result: PublicationChoice::Verify,
            } => runtime.verify(outcome)?,
            _ => return Err(runtime.mismatch()),
        }
        let entries = runtime.entries()?;
        if include_unused {
            return Ok(FileResult::Bibliography(entries));
        }
        let retained = runtime.retained()?;
        Ok(FileResult::Bibliography(
            entries
                .into_iter()
                .filter(|entry| {
                    !entry.id.starts_with("binding:") || retained.contains(&entry.citation_key)
                })
                .collect(),
        ))
    }
}

impl Runtime<'_> {
    fn mismatch(&self) -> Box<NativeHostError> {
        let mut error = failure(
            "plugin_host_response_mismatch",
            "写作书目结果与实际阶段不匹配。",
            &self.operation.key,
        );
        self.attach(&mut error);
        error
    }
    fn error(&self, cause: WritingError) -> Box<NativeHostError> {
        let mut error = match cause {
            WritingError::Database(cause) => sql_error(&self.operation.key, "bibliography", &cause),
            cause => failure(cause.code(), cause.to_string(), &self.operation.key),
        };
        self.attach(&mut error);
        error
    }
    fn attach(&self, error: &mut Box<NativeHostError>) {
        if let Some((instance, request)) = self.last.borrow().as_ref() {
            error.instance_id = Some(instance.clone());
            error.request_id = Some(request.clone());
        }
    }
    fn file_error(&self, cause: crate::scoped_text_files::TextFileError) -> Box<NativeHostError> {
        let mut error = self.operation.file_error(cause);
        self.attach(&mut error);
        error
    }
    fn remember(&self, decided: &crate::runtime_plugins::native::DecidedProject) {
        *self.last.borrow_mut() = Some((decided.instance_id.clone(), decided.request_id.clone()));
    }
    fn failed(&self, failure: Failure<WritingError>, reader: &Reader) -> Box<NativeHostError> {
        let mut error = match failure.cause {
            FailureCause::SourceDatabase(cause) => {
                let mut error = self.error(failure.error);
                error.query_database_error = Some(cause);
                error
            }
            FailureCause::SourceData(cause) => {
                let library = cause.package_key != self.operation.key;
                let import = matches!(&failure.error, WritingError::BibliographyImport(_));
                let mut error = data_failure(cause);
                if library {
                    error.plugin_id = "literature_library";
                    error.code = failure.error.code().into();
                    if import {
                        error.message = failure.error.to_string();
                    }
                }
                error
            }
            FailureCause::SourceReadUnavailable => self.mismatch(),
            FailureCause::NativeRejection(origin) => {
                let mut error = self.error(failure.error);
                if origin.package_key != self.operation.key {
                    error.plugin_id = "literature_library";
                    error.package_key = Some(origin.package_key);
                }
                error.instance_id = Some(origin.instance_id);
                error.request_id = Some(origin.native_request_id);
                return error;
            }
            FailureCause::Original => self.error(failure.error),
        };
        reader.attach(&mut error);
        error
    }
    fn read<T>(
        &self,
        mut capture: impl FnMut(&Connection, &mut Reader) -> Read<T>,
    ) -> NativeResult<T> {
        let mut reader = Reader::default();
        loop {
            reader.begin();
            let result = {
                let mut db = self.operation.store.connection().map_err(|cause| {
                    failure("storage_error", cause.to_string(), &self.operation.key)
                })?;
                crate::data_interface::guarded_transaction::run(&mut db, |db| {
                    let result = capture(db, &mut reader);
                    (
                        Ok::<_, std::convert::Infallible>(result),
                        !db.is_autocommit(),
                    )
                })
                .expect("read control flow is returned inside the result")
            };
            match result {
                Ok(value) => return Ok(value),
                Err(Flow::Fail(cause)) => return Err(self.failed(cause, &reader)),
                Err(Flow::Need(need)) => self.resume(&mut reader, need)?,
            }
        }
    }
    fn checked_write<T>(
        &self,
        outcome: &mut SaveOutcome,
        mut capture: impl FnMut(&Connection, &mut Reader) -> Read<T>,
        mut apply: impl FnMut(&Connection, T) -> NativeResult<()>,
    ) -> NativeResult<()> {
        self.read(|db, reader| {
            let value = capture(db, reader)?;
            *outcome = SaveOutcome::Unknown;
            Ok(apply(db, value))
        })?
    }
    fn adoption_decision(&self, input: AdoptionInput) -> NativeResult<AdoptionDecision> {
        let decided = self.operation.decide(Input::BibliographyAdoption {
            input: Box::new(input),
        })?;
        self.remember(&decided);
        Reader::adoption_reply(decided, &self.operation.key)
    }
    fn publication_decision(&self, input: PublicationInput) -> NativeResult<PublicationDecision> {
        let decided = self.operation.decide(Input::BibliographyPublication {
            input: Box::new(input),
        })?;
        self.remember(&decided);
        Reader::publication_reply(decided, &self.operation.key)
    }
    fn library_available(&self) -> bool {
        self.operation.store.native_note_trace().is_some()
    }
}

#[tauri::command]
pub(crate) async fn list_runtime_manuscript_bibliography_entries(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<NativePreviewService>>,
    catalog: tauri::State<'_, Arc<RuntimePluginCatalog>>,
    package_key: String,
    manuscript_id: String,
    include_unused_automatic: Option<bool>,
) -> NativeResult<Vec<ProjectBibliographyEntry>> {
    match super::operate_files(
        store.inner().clone(),
        Arc::clone(service.inner()),
        Arc::clone(catalog.inner()),
        package_key.clone(),
        manuscript_id,
        FileRequest::Bibliography {
            call: Call::List {
                include_unused_automatic: include_unused_automatic.unwrap_or(false),
            },
        },
    )
    .await?
    {
        FileResult::Bibliography(entries) => Ok(entries),
        _ => Err(failure(
            "plugin_host_response_mismatch",
            "写作书目读取结果种类不匹配。",
            &package_key,
        )),
    }
}
