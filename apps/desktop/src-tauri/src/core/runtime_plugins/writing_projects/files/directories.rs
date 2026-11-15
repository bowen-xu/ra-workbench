//! Original folder operations. Domain planning uses the existing writing host;
//! source ownership, coherent reads, Deferred transactions and rollback stay Core.
pub(crate) mod commands;
mod execute;
mod probe;
mod resources;
use super::super::{data_failure, failure, project_reply, sql_error};
use super::{FileOperation, FileResult};
use crate::{
    data_interface::guarded_transaction,
    runtime_plugins::{
        literature_catalog::save::SaveOutcome,
        native::{DecidedProject, NativeHostError, NativeResult},
    },
    scoped_text_files::{StagedDirectory, TextFileError},
};
use probe::{Observation, Relations};
use research_workbench_native_sdk::{
    writing_directory::{DirectoryDecision, DirectoryInput, DirectoryRequest, DirectoryStage},
    writing_project::{Decision, Input, ManuscriptDirectory},
};
pub(super) use resources::DirectoryResources;
use rusqlite::{DropBehavior, Transaction};

#[derive(Debug)]
pub(crate) enum DirectoryCall {
    List,
    Create { relative_path: String },
    Change { request: DirectoryRequest },
}

struct Origin {
    instance_id: String,
    request_id: String,
}
impl Origin {
    fn attach(&self, mut error: Box<NativeHostError>) -> Box<NativeHostError> {
        error.instance_id = Some(self.instance_id.clone());
        error.request_id = Some(self.request_id.clone());
        error
    }
}
pub(super) enum PhysicalChange {
    Rename,
    Delete { staged: StagedDirectory },
}
pub(super) struct DirectoryRuntime<'a> {
    operation: &'a FileOperation,
    resources: &'a DirectoryResources,
}

pub(super) fn value<'a, T>(stage: &'a DirectoryStage<T>, key: &str) -> NativeResult<&'a T> {
    match stage {
        DirectoryStage::Ready { value } => Ok(value),
        DirectoryStage::Rejected { reason } => Err(failure(reason.code(), reason.to_string(), key)),
        DirectoryStage::Unavailable => Err(failure(
            "plugin_host_response_mismatch",
            "目录准备缺少可消费的实际读取结果。",
            key,
        )),
    }
}

impl FileOperation {
    pub(super) fn directory(
        &self,
        call: &DirectoryCall,
        resources: &DirectoryResources,
        outcome: &mut SaveOutcome,
    ) -> NativeResult<FileResult> {
        let runtime = DirectoryRuntime {
            operation: self,
            resources,
        };
        match call {
            DirectoryCall::List | DirectoryCall::Create { .. } => runtime.simple(call, outcome),
            DirectoryCall::Change { request } => runtime.change(request, outcome),
        }
    }
}
impl DirectoryRuntime<'_> {
    fn decide(&self, input: DirectoryInput) -> NativeResult<(DirectoryDecision, Origin)> {
        let DecidedProject {
            output,
            instance_id,
            request_id,
        } = self.operation.decide(Input::Directory {
            input: Box::new(input),
        })?;
        let origin = Origin {
            instance_id,
            request_id,
        };
        match output.decision {
            Decision::Directory { decision } => Ok((*decision, origin)),
            _ => Err(origin.attach(failure(
                "plugin_host_response_mismatch",
                "目录决策种类不匹配。",
                &self.operation.key,
            ))),
        }
    }
    fn simple(&self, call: &DirectoryCall, outcome: &mut SaveOutcome) -> NativeResult<FileResult> {
        let id = &self.operation.manuscript_id;
        let captured = {
            let db = self.operation.store.connection().map_err(|error| {
                failure("storage_error", error.to_string(), &self.operation.key)
            })?;
            self.operation
                .resources
                .projects
                .get(&db, id)
                .map_err(data_failure)?
        };
        let decided = self.operation.decide(Input::GetProject {
            manuscript_id: id.clone(),
            rows: captured.clone(),
        })?;
        let origin = Origin {
            instance_id: decided.instance_id.clone(),
            request_id: decided.request_id.clone(),
        };
        project_reply(&self.operation.key, decided)?;
        let result = (|| {
            let files = self.operation.store.manuscript_files(id);
            match call {
                DirectoryCall::List => files
                    .directories()
                    .map(|paths| {
                        FileResult::Directories(
                            paths
                                .into_iter()
                                .map(|relative_path| ManuscriptDirectory { relative_path })
                                .collect(),
                        )
                    })
                    .map_err(|error| self.operation.file_error(error)),
                DirectoryCall::Create { relative_path } => {
                    let db = self.operation.store.connection().map_err(|error| {
                        failure("storage_error", error.to_string(), &self.operation.key)
                    })?;
                    self.operation
                        .resources
                        .projects
                        .recheck(&db, id, &captured)
                        .map_err(data_failure)?;
                    files
                        .create_directory(relative_path.trim())
                        .map_err(|cause| {
                            if matches!(cause, TextFileError::Io(_)) {
                                *outcome = SaveOutcome::Unknown;
                            }
                            self.operation.file_error(cause)
                        })?;
                    *outcome = SaveOutcome::Committed {
                        item_id: id.clone(),
                    };
                    Ok(FileResult::Directory(ManuscriptDirectory {
                        relative_path: relative_path.trim().into(),
                    }))
                }
                DirectoryCall::Change { .. } => Err(failure(
                    "plugin_host_response_mismatch",
                    "目录变更被送往只读／创建适配。",
                    &self.operation.key,
                )),
            }
        })();
        result.map_err(|error| origin.attach(error))
    }

    fn capture<T>(&self, read: impl FnOnce(&Transaction<'_>) -> T) -> NativeResult<T> {
        let mut guard =
            self.operation.store.connection().map_err(|error| {
                failure("storage_error", error.to_string(), &self.operation.key)
            })?;
        guarded_transaction::run(&mut guard, |db| {
            let mut tx = match db.unchecked_transaction() {
                Ok(tx) => tx,
                Err(cause) => {
                    return (
                        Err(sql_error(&self.operation.key, "capture_begin", &cause)),
                        !db.is_autocommit(),
                    );
                }
            };
            tx.set_drop_behavior(DropBehavior::Ignore);
            let captured = read(&tx);
            match tx.rollback() {
                Ok(()) if db.is_autocommit() => (Ok(captured), false),
                Err(cause) => (
                    Err(sql_error(&self.operation.key, "capture_rollback", &cause)),
                    true,
                ),
                Ok(()) => (
                    Err(failure(
                        "storage_error",
                        "writing_directory_capture_unsettled",
                        &self.operation.key,
                    )),
                    true,
                ),
            }
        })
    }

    fn change(
        &self,
        request: &DirectoryRequest,
        outcome: &mut SaveOutcome,
    ) -> NativeResult<FileResult> {
        let id = &self.operation.manuscript_id;
        let (manuscript, files) = self.capture(|tx| {
            let manuscript = matches!(request, DirectoryRequest::Delete { .. })
                .then(|| Observation::capture(tx, &self.resources.manuscript, id));
            (
                manuscript,
                Observation::capture(tx, &self.resources.files, id),
            )
        })?;
        let (decision, origin) = self.decide(DirectoryInput::Preflight {
            request: request.clone(),
            manuscript: manuscript.as_ref().and_then(Observation::rows),
            files: files.rows(),
        })?;
        let DirectoryDecision::Preflight {
            manuscript: guard,
            files: file_guard,
        } = decision
        else {
            return Err(origin.attach(failure(
                "plugin_host_response_mismatch",
                "目录前置决策阶段不匹配。",
                &self.operation.key,
            )));
        };
        self.check_preflight(manuscript.as_ref(), &files, &guard, &file_guard)
            .map_err(|error| origin.attach(error))?;
        let scoped = self.operation.store.manuscript_files(id);
        let physical = match request {
            DirectoryRequest::Rename {
                from_path, to_path, ..
            } => scoped
                .rename_directory(from_path, to_path)
                .map(|()| PhysicalChange::Rename),
            DirectoryRequest::Delete { relative_path, .. } => scoped
                .stage_directory(relative_path)
                .map(|staged| PhysicalChange::Delete { staged }),
        }
        .map_err(|cause| {
            if matches!(cause, TextFileError::Io(_)) {
                *outcome = SaveOutcome::Unknown;
            }
            origin.attach(self.operation.file_error(cause))
        })?;
        *outcome = SaveOutcome::Unknown;
        let captured = self
            .capture(|tx| self.resources.capture(tx, id))
            .map_err(|mut error| {
                self.recovery_diagnostic(&mut error, &physical, outcome);
                origin.attach(error)
            })?;
        let input = DirectoryInput::Relations {
            request: request.clone(),
            manuscript: captured.manuscript.rows(),
            files: captured.files.rows(),
            drafts: captured.drafts.rows(),
            session: captured.session.rows(),
        };
        let (decision, origin) = match self.decide(input) {
            Ok(result) => result,
            Err(mut error) => {
                self.recover_unpublished(request, &captured, &physical, outcome);
                self.recovery_diagnostic(&mut error, &physical, outcome);
                return Err(error);
            }
        };
        let DirectoryDecision::Relations { plan } = decision else {
            let mut error = origin.attach(failure(
                "plugin_host_response_mismatch",
                "目录关联决策阶段不匹配。",
                &self.operation.key,
            ));
            self.recover_unpublished(request, &captured, &physical, outcome);
            self.recovery_diagnostic(&mut error, &physical, outcome);
            return Err(error);
        };
        let rows = self
            .execute(request, &captured, &plan, &physical, outcome)
            .map_err(|mut error| {
                self.recovery_diagnostic(&mut error, &physical, outcome);
                origin.attach(error)
            })?;
        match physical {
            PhysicalChange::Delete { staged } => {
                let _ = staged.discard();
                Ok(FileResult::Deleted)
            }
            PhysicalChange::Rename => {
                let rows = rows.ok_or_else(|| {
                    failure(
                        "plugin_host_response_mismatch",
                        "目录改名缺少原稿件读回。",
                        &self.operation.key,
                    )
                })?;
                self.operation
                    .decide(Input::GetProject {
                        manuscript_id: id.clone(),
                        rows,
                    })
                    .and_then(|decided| project_reply(&self.operation.key, decided))
                    .map(FileResult::Project)
            }
        }
    }

    fn check_preflight(
        &self,
        manuscript: Option<&Observation>,
        files: &Observation,
        guard: &DirectoryStage<()>,
        file_guard: &DirectoryStage<()>,
    ) -> NativeResult<()> {
        let mut db =
            self.operation.store.connection().map_err(|error| {
                failure("storage_error", error.to_string(), &self.operation.key)
            })?;
        guarded_transaction::run(&mut db, |db| {
            let mut tx = match db.unchecked_transaction() {
                Ok(tx) => tx,
                Err(cause) => {
                    return (
                        Err(sql_error(&self.operation.key, "preflight_begin", &cause)),
                        !db.is_autocommit(),
                    );
                }
            };
            tx.set_drop_behavior(DropBehavior::Ignore);
            let result = (|| {
                if let Some(manuscript) = manuscript {
                    manuscript.require(&tx)?;
                }
                value(guard, &self.operation.key)?;
                files.require(&tx)?;
                value(file_guard, &self.operation.key)?;
                Ok(())
            })();
            let rollback = tx.rollback();
            match rollback {
                Ok(()) if db.is_autocommit() => (result, false),
                Err(cause) => (
                    Err(sql_error(&self.operation.key, "preflight_rollback", &cause)),
                    true,
                ),
                Ok(()) => (
                    Err(failure(
                        "storage_error",
                        "writing_directory_preflight_unsettled",
                        &self.operation.key,
                    )),
                    true,
                ),
            }
        })
    }

    fn restore(
        &self,
        request: &DirectoryRequest,
        physical: &PhysicalChange,
    ) -> Result<(), TextFileError> {
        let files = self
            .operation
            .store
            .manuscript_files(&self.operation.manuscript_id);
        match (request, physical) {
            (
                DirectoryRequest::Rename {
                    from_path, to_path, ..
                },
                PhysicalChange::Rename,
            ) => files.rename_directory(to_path, from_path),
            (DirectoryRequest::Delete { relative_path, .. }, PhysicalChange::Delete { staged }) => {
                files.restore_directory(relative_path, staged)
            }
            _ => Err(TextFileError::InvalidPath),
        }
    }
    fn recovery_diagnostic(
        &self,
        error: &mut NativeHostError,
        physical: &PhysicalChange,
        outcome: &SaveOutcome,
    ) {
        if *outcome == SaveOutcome::Unknown {
            let identity = match physical {
                PhysicalChange::Rename => "source-directory".into(),
                PhysicalChange::Delete { staged } => staged.identity(),
            };
            error.message.push_str(&format!(
                "；writing_directory_recovery_required：请按原稿件与目录核对（{identity}）。"
            ));
        }
    }
    fn recover_unpublished(
        &self,
        request: &DirectoryRequest,
        captured: &Relations,
        physical: &PhysicalChange,
        outcome: &mut SaveOutcome,
    ) {
        // Failed probes carry no successful state proof. Their original caller
        // handles rollback; a host failure here conservatively retains staging.
        if !captured.all_available() {
            return;
        }
        let Ok(mut db) = self.operation.store.connection() else {
            return;
        };
        let result = guarded_transaction::run(&mut db, |db| {
            let mut tx = match db.unchecked_transaction() {
                Ok(tx) => tx,
                Err(_) => return (Err(()), !db.is_autocommit()),
            };
            tx.set_drop_behavior(DropBehavior::Ignore);
            let checked = captured.require_all(&tx);
            let restored = checked.and_then(|()| {
                self.restore(request, physical)
                    .map_err(|cause| self.operation.file_error(cause))
            });
            let rollback = tx.rollback();
            let settled = rollback.is_ok() && db.is_autocommit();
            (
                if settled {
                    restored.map_err(|_| ())
                } else {
                    Err(())
                },
                !settled,
            )
        });
        if result.is_ok() {
            *outcome = SaveOutcome::RolledBack;
        }
    }
}
