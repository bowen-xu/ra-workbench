//! Selected writing rules for authorized Agent compilation and retained-result queries.
#[cfg(target_os = "macos")]
use super::workspace_recipe;
use super::{domain::Domain, query, read};
use crate::{
    agent::writing_compile_contracts::LatexToolchainInfo,
    runtime_plugins::{
        native::{NativeResult, ProjectOperation},
        writing_projects::context::Binding,
    },
    storage::AppStore,
};
use research_workbench_native_sdk::writing_compile::{Decision, Input};

#[cfg(target_os = "macos")]
use crate::agent::writing_compile_contracts::{
    LatexWorkspaceCompileRecipe, LatexWorkspaceCompileRequest,
};
#[cfg(target_os = "macos")]
use research_workbench_native_sdk::writing_compile::{CompileDiagnostic, CompileOptions};

pub(crate) struct AgentCompileSource {
    domain: Domain,
    _operation: ProjectOperation,
}

impl AgentCompileSource {
    pub(crate) fn forward(
        &self,
        request: &crate::agent::compile_synctex::AgentCompileSynctexQuery<'_>,
        relative_path: &str,
        line: u32,
        column: u32,
    ) -> NativeResult<Vec<crate::agent::compile_synctex::SynctexPdfLocation>> {
        if !matches!(
            self.domain.call(Input::ValidateForward {
                run_id: None,
                relative_path: relative_path.into(),
                line
            })?,
            Decision::Accepted
        ) {
            return Err(self.domain.mismatch());
        }
        query::forward(&self.domain, request, relative_path, line, column)
    }

    pub(crate) fn inverse(
        &self,
        request: &crate::agent::compile_synctex::AgentCompileSynctexQuery<'_>,
        page: u32,
        x: f64,
        y: f64,
    ) -> NativeResult<Vec<crate::agent::compile_synctex::SynctexSourceLocation>> {
        if !matches!(
            self.domain.call(Input::ValidateInverse {
                run_id: None,
                page,
                x_bits: x.to_bits(),
                y_bits: y.to_bits()
            })?,
            Decision::Accepted
        ) {
            return Err(self.domain.mismatch());
        }
        query::inverse(&self.domain, request, page, x, y)
    }
    pub(crate) fn new(binding: Binding, runtime: tokio::runtime::Handle) -> NativeResult<Self> {
        let domain = Domain::new(binding, runtime);
        let operation = domain.operation()?;
        Ok(Self {
            domain,
            _operation: operation,
        })
    }

    pub(crate) fn detect(&self, store: &AppStore) -> NativeResult<LatexToolchainInfo> {
        read::detect(store, &self.domain)
    }

    #[cfg(target_os = "macos")]
    pub(crate) fn recipe(
        &self,
        toolchain: &LatexToolchainInfo,
        request: &LatexWorkspaceCompileRequest<'_>,
    ) -> NativeResult<LatexWorkspaceCompileRecipe> {
        if !matches!(
            self.domain.call(Input::ValidateToolchain {
                engine: request.engine.into(),
                toolchain: toolchain.clone()
            })?,
            Decision::Accepted
        ) {
            return Err(self.domain.mismatch());
        }
        let program =
            workspace_recipe::program(toolchain).map_err(|cause| self.domain.error(cause))?;
        let argument = match self.domain.call(Input::EngineArgument {
            engine: request.engine.into(),
        })? {
            Decision::EngineArgument { argument } => argument,
            _ => return Err(self.domain.mismatch()),
        };
        let flags = match self.domain.call(Input::Flags {
            options: CompileOptions {
                draft: request.draft,
                stop_on_first_error: request.stop_on_first_error,
                from_scratch: request.from_scratch,
            },
        })? {
            Decision::Flags { flags } => flags,
            _ => return Err(self.domain.mismatch()),
        };
        workspace_recipe::assemble(program, toolchain, request, argument, &flags)
            .map_err(|cause| self.domain.error(cause))
    }

    #[cfg(target_os = "macos")]
    pub(crate) fn diagnostics(&self, output: &str) -> NativeResult<Vec<CompileDiagnostic>> {
        match self.domain.call(Input::Diagnostics {
            raw_output: output.into(),
        })? {
            Decision::Diagnostics { items } => Ok(items),
            _ => Err(self.domain.mismatch()),
        }
    }
}
