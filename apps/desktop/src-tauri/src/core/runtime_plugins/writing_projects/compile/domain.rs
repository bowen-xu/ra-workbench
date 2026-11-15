//! The selected writing instance prepares finite values outside database guards.
use super::resources::Sql;
use crate::storage::AppStore;
use crate::{
    agent::writing_error::WritingError,
    runtime_plugins::{
        native::{NativeHostError, NativeProfile, NativeResult, package},
        writing_projects::{context::Binding, failure, sql_error},
    },
};
use research_workbench_native_sdk::{
    writing_compile::{CompileRun, Decision, Input, Rejection},
    writing_project,
};
use std::cell::RefCell;

pub(super) struct Domain {
    pub(super) binding: Binding,
    runtime: tokio::runtime::Handle,
    last: RefCell<Option<(String, String)>>,
}

impl Domain {
    pub(super) fn new(binding: Binding, runtime: tokio::runtime::Handle) -> Self {
        Self {
            binding,
            runtime,
            last: RefCell::new(None),
        }
    }

    pub(super) fn resources(&self) -> NativeResult<Sql> {
        let package = package::accept(
            &self.binding.catalog,
            &self.binding.package_key,
            NativeProfile::WritingProjects,
        )?;
        Sql::load(&self.binding.catalog, &package)
            .map_err(|error| failure(error.code, error.message, &self.binding.package_key))
    }

    pub(super) fn call(&self, input: Input) -> NativeResult<Decision> {
        match self.decide(writing_project::Input::Compile {
            input: Box::new(input),
        })? {
            writing_project::Decision::Compile { decision } => match *decision {
                Decision::Rejected { reason } => Err(self.error(match reason {
                    Rejection::InvalidPath => WritingError::InvalidPath,
                    Rejection::SynctexNoResult => WritingError::SynctexNoResult,
                    Rejection::InvalidEngine => WritingError::InvalidEngine,
                    Rejection::ToolchainUnavailable => WritingError::ToolchainUnavailable,
                })),
                decision => Ok(decision),
            },
            _ => Err(self.mismatch()),
        }
    }

    pub(super) fn decide(
        &self,
        input: writing_project::Input,
    ) -> NativeResult<writing_project::Decision> {
        let decided = self.runtime.block_on(async {
            self.binding
                .service
                .project_decision(self.binding.package_key.clone(), input)
                .await
        })?;
        *self.last.borrow_mut() = Some((decided.instance_id, decided.request_id));
        Ok(decided.output.decision)
    }

    pub(super) fn regenerate(
        &self,
        store: AppStore,
        manuscript_id: String,
        operation: crate::runtime_plugins::native::ProjectOperation,
    ) -> NativeResult<()> {
        use crate::runtime_plugins::writing_projects::files::{
            FileRequest, FileResult, bibliography::Call, operate_files_with_operation,
        };
        match self.runtime.block_on(operate_files_with_operation(
            store,
            self.binding.service.clone(),
            self.binding.catalog.clone(),
            self.binding.package_key.clone(),
            manuscript_id,
            FileRequest::Bibliography {
                call: Call::Regenerate,
            },
            Some(operation),
        ))? {
            FileResult::BibliographyStatus(_) => Ok(()),
            _ => Err(self.mismatch()),
        }
    }

    pub(super) fn operation(
        &self,
    ) -> NativeResult<crate::runtime_plugins::native::ProjectOperation> {
        self.runtime.block_on(
            self.binding
                .service
                .project_operation(&self.binding.package_key),
        )
    }

    pub(super) fn run(&self, run: CompileRun) -> NativeResult<CompileRun> {
        match self.call(Input::ProjectRun { run })? {
            Decision::Run { run } => Ok(run),
            _ => Err(self.mismatch()),
        }
    }

    pub(super) fn error(&self, cause: WritingError) -> Box<NativeHostError> {
        let mut error = match cause {
            WritingError::Database(cause) => {
                sql_error(&self.binding.package_key, "compile", &cause)
            }
            cause => failure(cause.code(), cause.to_string(), &self.binding.package_key),
        };
        if let Some((instance, request)) = self.last.borrow().as_ref() {
            error.instance_id = Some(instance.clone());
            error.request_id = Some(request.clone());
        }
        error
    }

    pub(super) fn mismatch(&self) -> Box<NativeHostError> {
        let mut error = failure(
            "plugin_host_response_mismatch",
            "写作编译结果与本次阶段不匹配。",
            &self.binding.package_key,
        );
        if let Some((instance, request)) = self.last.borrow().as_ref() {
            error.instance_id = Some(instance.clone());
            error.request_id = Some(request.clone());
        }
        error
    }
}
