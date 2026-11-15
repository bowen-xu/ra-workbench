//! Original file operations and their explicit creation/save lock boundaries.
//! Native owns domain decisions; Core retains scoped files and the original lock.
pub(crate) mod agent_index;
pub(crate) mod bibliography;
pub(crate) mod bibliography_adoption;
pub(crate) mod bibliography_entries;
pub(crate) mod bibliography_rekey;
mod bibliography_transaction;
mod catalog;
pub(crate) mod catalog_preparation;
pub(crate) mod catalog_query;
pub(crate) mod changes;
pub(crate) mod checkpoints;
pub(crate) mod commands;
pub(crate) mod deletion;
pub(crate) mod directories;
pub(crate) mod note_assets;
mod recovery;
pub(super) mod resources;
pub(crate) mod save;
pub(crate) mod transfer;
use super::{data_failure, decided_failure, failure, project_reply};
use crate::{
    runtime_plugins::{
        RuntimePluginCatalog,
        literature_catalog::save::SaveOutcome,
        native::{DecidedProject, NativePreviewService, NativeProfile, NativeResult, package},
    },
    scoped_text_files::TextFileError,
    storage::AppStore,
};
use research_workbench_native_sdk::writing_file::FileChangeRequest;
use research_workbench_native_sdk::writing_project::{
    Decision, Input, ManuscriptFile, ManuscriptFileContent, SaveFileRequest,
};
use resources::FileResources;
use std::sync::Arc;

pub(super) enum FileRequest {
    NoteAsset {
        call: note_assets::Call,
    },
    ExportSource {
        destination: String,
        rename_on_conflict: bool,
    },
    Bibliography {
        call: bibliography::Call,
    },
    Checkpoint {
        call: checkpoints::CheckpointCall,
    },
    DeleteProject {
        manager: crate::compile_activity::CompileManager,
    },
    Directory {
        call: directories::DirectoryCall,
    },
    List,
    Read {
        relative_path: String,
    },
    Save {
        request: SaveFileRequest,
    },
    Create {
        request: SaveFileRequest,
    },
    Change {
        request: FileChangeRequest,
    },
}

#[derive(Clone, Copy)]
enum WriteMode {
    Save,
    Create,
}

enum FileAction {
    NoteAsset {
        call: note_assets::Call,
    },
    ExportSource {
        destination: String,
        rename_on_conflict: bool,
    },
    Bibliography {
        call: bibliography::Call,
        resources: Box<bibliography::Sql>,
    },
    Checkpoint {
        call: checkpoints::CheckpointCall,
        resources: Box<checkpoints::Resources>,
    },
    DeleteProject {
        manager: crate::compile_activity::CompileManager,
        resources: Box<deletion::DeletionResources>,
    },
    Directory {
        call: directories::DirectoryCall,
        resources: Box<directories::DirectoryResources>,
    },
    List,
    Read {
        relative_path: String,
    },
    Write {
        request: SaveFileRequest,
        mode: WriteMode,
        catalog: Box<catalog::CatalogResources>,
    },
    Change {
        request: FileChangeRequest,
        resources: Box<changes::ChangeResources>,
    },
}

#[derive(Debug)]
pub(super) enum FileResult {
    NoteAsset(research_workbench_native_sdk::writing_note_asset::NoteAsset),
    AssetBytes(Vec<u8>),
    ProjectImported(research_workbench_native_sdk::writing_project::ManuscriptTransferResult),
    Exported(research_workbench_native_sdk::writing_project::ManuscriptExportResult),
    ResourcesImported(
        research_workbench_native_sdk::writing_project::ManuscriptResourceImportResult,
    ),
    CitationCandidates(Vec<research_workbench_native_sdk::writing_bibliography::CitationCandidate>),
    CitationBinding(research_workbench_native_sdk::writing_project::CitationBinding),
    BibliographyChanged,
    RemovedBibliographyEntries(usize),
    CitationKeyMigration(
        Box<research_workbench_native_sdk::writing_bibliography::rekey::Migration>,
    ),
    BibliographyStatus(research_workbench_native_sdk::writing_project::BibliographyStatus),
    Bibliography(
        Vec<research_workbench_native_sdk::writing_bibliography::ProjectBibliographyEntry>,
    ),
    Checkpoint(Box<research_workbench_native_sdk::writing_checkpoint::CheckpointRecord>),
    Checkpoints(Vec<research_workbench_native_sdk::writing_checkpoint::CheckpointRecord>),
    Directories(Vec<research_workbench_native_sdk::writing_project::ManuscriptDirectory>),
    Directory(research_workbench_native_sdk::writing_project::ManuscriptDirectory),
    List(Vec<ManuscriptFile>),
    Read(ManuscriptFileContent),
    Project(research_workbench_native_sdk::writing_project::Manuscript),
    Deleted,
}

pub(super) async fn operate_files(
    store: AppStore,
    service: Arc<NativePreviewService>,
    catalog: Arc<RuntimePluginCatalog>,
    key: String,
    manuscript_id: String,
    request: FileRequest,
) -> NativeResult<FileResult> {
    operate_files_with_operation(store, service, catalog, key, manuscript_id, request, None).await
}

/// Core may retain the original accepted lease across a compile handoff. Reuse
/// that lease, since reacquiring behind a pending shutdown writer would deadlock.
pub(super) async fn operate_files_with_operation(
    store: AppStore,
    service: Arc<NativePreviewService>,
    catalog: Arc<RuntimePluginCatalog>,
    key: String,
    manuscript_id: String,
    request: FileRequest,
    operation: Option<crate::runtime_plugins::native::ProjectOperation>,
) -> NativeResult<FileResult> {
    let selected = key.clone();
    let id = if matches!(
        &request,
        FileRequest::Checkpoint { .. } | FileRequest::NoteAsset { .. }
    ) {
        manuscript_id
    } else {
        manuscript_id.trim().to_owned()
    };
    let operation_id = id.clone();
    tokio::spawn(async move {
        let _operation = match operation {
            Some(operation) => operation,
            None => service.project_operation(&selected).await?,
        };
        let library = if matches!(&request, FileRequest::Bibliography { .. }) {
            store.native_note_trace().cloned()
        } else {
            None
        };
        let _library_operation = match library.as_ref() {
            Some(binding) => Some(
                binding
                    .service
                    .bibliography_operation(&binding.package_key)
                    .await?,
            ),
            None => None,
        };
        let runtime = tokio::runtime::Handle::current();
        let worker_service = Arc::clone(&service);
        let worker_key = selected.clone();
        tokio::task::spawn_blocking(move || {
            let package = package::accept(&catalog, &worker_key, NativeProfile::WritingProjects)?;
            let resources = FileResources::load(&catalog, &package)
                .map_err(|error| failure(error.code, error.message, &worker_key))?;
            let write_mode = if matches!(&request, FileRequest::Create { .. }) {
                WriteMode::Create
            } else {
                WriteMode::Save
            };
            let action = match request {
                FileRequest::NoteAsset { call } => FileAction::NoteAsset { call },
                FileRequest::ExportSource {
                    destination,
                    rename_on_conflict,
                } => FileAction::ExportSource {
                    destination,
                    rename_on_conflict,
                },
                FileRequest::Bibliography { call } => FileAction::Bibliography {
                    call,
                    resources: Box::new(
                        bibliography::Sql::load(&catalog, &package)
                            .map_err(|error| failure(error.code, error.message, &worker_key))?,
                    ),
                },
                FileRequest::Checkpoint { call } => FileAction::Checkpoint {
                    call,
                    resources: Box::new(
                        checkpoints::Resources::load(&catalog, &package)
                            .map_err(|error| failure(error.code, error.message, &worker_key))?,
                    ),
                },
                FileRequest::DeleteProject { manager } => FileAction::DeleteProject {
                    manager,
                    resources: Box::new(
                        deletion::DeletionResources::load(&catalog, &package)
                            .map_err(|error| failure(error.code, error.message, &worker_key))?,
                    ),
                },
                FileRequest::Directory { call } => FileAction::Directory {
                    call,
                    resources: Box::new(
                        directories::DirectoryResources::load(&catalog, &package, &resources)
                            .map_err(|error| failure(error.code, error.message, &worker_key))?,
                    ),
                },
                FileRequest::Change { request } => FileAction::Change {
                    request,
                    resources: Box::new(
                        changes::ChangeResources::load(&catalog, &package)
                            .map_err(|error| failure(error.code, error.message, &worker_key))?,
                    ),
                },
                FileRequest::List => FileAction::List,
                FileRequest::Read { relative_path } => FileAction::Read { relative_path },
                FileRequest::Save { request } | FileRequest::Create { request } => {
                    FileAction::Write {
                        request,
                        mode: write_mode,
                        catalog: Box::new(
                            catalog::CatalogResources::load(&catalog, &package)
                                .map_err(|error| failure(error.code, error.message, &worker_key))?,
                        ),
                    }
                }
            };
            FileOperation {
                store,
                service: worker_service,
                resources,
                key: worker_key,
                manuscript_id: id,
                action,
                runtime,
            }
            .run()
        })
        .await
        .map_err(|_| {
            let mut error = failure(
                "storage_error",
                "写作文件操作未能结算；请核对原文件和待恢复书目。",
                &selected,
            );
            error.save_outcome = Some(SaveOutcome::Unknown);
            error
        })?
    })
    .await
    .map_err(|_| {
        let mut error = failure(
            "storage_error",
            "写作文件操作未能结算；请核对原文件和待恢复书目。",
            &key,
        );
        error.save_outcome = Some(SaveOutcome::Unknown);
        error.operation_id = Some(operation_id.clone());
        error
    })?
    .map_err(|mut error| {
        error.operation_id = Some(operation_id);
        error
    })
}

struct FileOperation {
    store: AppStore,
    service: Arc<NativePreviewService>,
    resources: FileResources,
    key: String,
    manuscript_id: String,
    action: FileAction,
    runtime: tokio::runtime::Handle,
}

impl FileOperation {
    fn decide(&self, input: Input) -> NativeResult<DecidedProject> {
        // This runs only in the owned blocking task. The original synchronous
        // bibliography guard stays on that thread, never on an executor thread.
        self.runtime
            .block_on(self.service.project_decision(self.key.clone(), input))
    }

    fn file_error(
        &self,
        error: TextFileError,
    ) -> Box<crate::runtime_plugins::native::NativeHostError> {
        failure(
            error.code(),
            crate::agent::writing_error::WritingError::from(error).to_string(),
            &self.key,
        )
    }

    fn run(&self) -> NativeResult<FileResult> {
        let io_guard = match &self.action {
            FileAction::Bibliography {
                call: bibliography::Call::Search { .. },
                ..
            } => None,
            FileAction::DeleteProject { .. } => None,
            FileAction::ExportSource { .. } => None,
            FileAction::NoteAsset { .. } => None,
            FileAction::Directory { .. } => None,
            FileAction::Change { .. } => None,
            FileAction::Write {
                mode: WriteMode::Create,
                ..
            } => None,
            _ => Some(
                self.store
                    .bibliography_io()
                    .map_err(|error| failure("storage_error", error.to_string(), &self.key))?,
            ),
        };
        let mut outcome = SaveOutcome::NotStarted;
        let result = (|| {
            if io_guard.is_some()
                && !matches!(
                    &self.action,
                    FileAction::Bibliography {
                        call: bibliography::Call::ImportResources { .. }
                            | bibliography::Call::ImportProject { .. }
                            | bibliography::Call::DuplicateProject { .. }
                            | bibliography::Call::Restore
                            | bibliography::Call::Sync { .. }
                            | bibliography::Call::Update { .. }
                            | bibliography::Call::Link { .. },
                        ..
                    }
                )
            {
                self.recover(&mut outcome)?;
            }
            match &self.action {
                FileAction::NoteAsset { call } => self.note_asset(call, &mut outcome),
                FileAction::ExportSource {
                    destination,
                    rename_on_conflict,
                } => self
                    .export_source(destination, *rename_on_conflict)
                    .map(FileResult::Exported),
                FileAction::Bibliography { call, resources } => {
                    self.bibliography(resources, call, &mut outcome)
                }
                FileAction::Checkpoint { call, resources } => {
                    self.checkpoints(call, resources, &mut outcome)
                }
                FileAction::DeleteProject { manager, resources } => {
                    self.delete_project(manager, resources, &mut outcome)
                }
                FileAction::Directory { call, resources } => {
                    self.directory(call, resources, &mut outcome)
                }
                FileAction::Change { request, resources } => {
                    self.change(request, resources, &mut outcome)
                }
                FileAction::Write {
                    request,
                    mode,
                    catalog,
                } => self.write(request, *mode, catalog, &mut outcome),
                _ => self.read(),
            }
        })();
        result.map_err(|mut error| {
            if error.save_outcome.is_none() {
                error.save_outcome = Some(outcome);
            }
            error
        })
    }

    fn read(&self) -> NativeResult<FileResult> {
        let input = {
            let db = self
                .store
                .connection()
                .map_err(|error| failure("storage_error", error.to_string(), &self.key))?;
            let manuscript = self
                .resources
                .projects
                .get(&db, &self.manuscript_id)
                .map_err(data_failure)?;
            match &self.action {
                FileAction::List => {
                    let files = if manuscript.rows.is_empty() {
                        None
                    } else {
                        Some(
                            self.resources
                                .files(&db, &self.manuscript_id)
                                .map_err(data_failure)?,
                        )
                    };
                    Input::ListFiles {
                        manuscript_id: self.manuscript_id.clone(),
                        manuscript,
                        files,
                    }
                }
                FileAction::Read { .. } => Input::GetProject {
                    manuscript_id: self.manuscript_id.clone(),
                    rows: manuscript,
                },
                FileAction::Write { .. }
                | FileAction::NoteAsset { .. }
                | FileAction::ExportSource { .. }
                | FileAction::Bibliography { .. }
                | FileAction::DeleteProject { .. }
                | FileAction::Change { .. }
                | FileAction::Directory { .. }
                | FileAction::Checkpoint { .. } => {
                    return Err(failure(
                        "plugin_host_response_mismatch",
                        "文件保存被错误送往读取适配。",
                        &self.key,
                    ));
                }
            }
        };
        let decided = self.decide(input)?;
        match &self.action {
            FileAction::List => match decided.output.decision {
                Decision::Files { files } => Ok(FileResult::List(files)),
                Decision::Rejected { reason } => Err(decided_failure(
                    reason.code(),
                    reason.to_string(),
                    &self.key,
                    &decided.instance_id,
                    &decided.request_id,
                )),
                _ => Err(decided_failure(
                    "plugin_host_response_mismatch",
                    "写作文件列表结果种类不匹配。",
                    &self.key,
                    &decided.instance_id,
                    &decided.request_id,
                )),
            },
            FileAction::Read { relative_path } => {
                let instance_id = decided.instance_id.clone();
                let request_id = decided.request_id.clone();
                project_reply(&self.key, decided)?;
                let (content, sha256) = self
                    .store
                    .manuscript_files(&self.manuscript_id)
                    .read(relative_path.trim())
                    .map_err(|error| {
                        let mut error = self.file_error(error);
                        error.instance_id = Some(instance_id.clone());
                        error.request_id = Some(request_id.clone());
                        error
                    })?;
                Ok(FileResult::Read(ManuscriptFileContent {
                    relative_path: relative_path.clone(),
                    content,
                    sha256,
                }))
            }
            FileAction::Write { .. }
            | FileAction::NoteAsset { .. }
            | FileAction::ExportSource { .. }
            | FileAction::Bibliography { .. }
            | FileAction::DeleteProject { .. }
            | FileAction::Change { .. }
            | FileAction::Directory { .. }
            | FileAction::Checkpoint { .. } => Err(failure(
                "plugin_host_response_mismatch",
                "文件保存被错误送往读取适配。",
                &self.key,
            )),
        }
    }
}
