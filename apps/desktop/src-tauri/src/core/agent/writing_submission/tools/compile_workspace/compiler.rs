//! Freeze the original static source or selected writing package for this owned compile.
use super::*;
use crate::{
    agent::writing_compile_contracts::{CompileDiagnostic, LatexWorkspaceCompileRecipe},
    runtime_plugins::{
        native::NativeHostError, writing_projects::compile::workspace::AgentCompileSource,
    },
};

pub(super) enum Compiler {
    Static(WritingWorkspaceCompileSource),
    Runtime(AgentCompileSource),
}

pub(super) enum PreparedDiagnostics {
    Deferred(fn(&str) -> Vec<CompileDiagnostic>),
    Parsed(Vec<CompileDiagnostic>),
}

impl PreparedDiagnostics {
    pub(super) fn resolve(self, raw_output: &str) -> Vec<CompileDiagnostic> {
        match self {
            Self::Deferred(parse) => parse(raw_output),
            Self::Parsed(items) => items,
        }
    }
}

fn native_failure(error: Box<NativeHostError>) -> RunError {
    match serde_json::to_string(&error) {
        Ok(diagnostic) => RunError::ToolRejectionDiagnostic(diagnostic),
        Err(_) => RunError::EventSink,
    }
}

impl Compiler {
    pub(super) fn select(
        owner: &CompileWorkspaceTools,
        runtime: tokio::runtime::Handle,
    ) -> Result<Self, RunError> {
        if let Some(source) = owner.compiler_source {
            return Ok(Self::Static(source));
        }
        let binding = owner
            .native_compiler
            .clone()
            .ok_or(RunError::ToolUnavailable)?;
        AgentCompileSource::new(binding, runtime)
            .map(Self::Runtime)
            .map_err(native_failure)
    }

    pub(super) fn detect(&self, store: &AppStore) -> Result<LatexToolchainInfo, RunError> {
        match self {
            Self::Static(source) => detect_toolchain(store, *source),
            Self::Runtime(source) => source.detect(store).map_err(native_failure),
        }
    }

    pub(super) fn recipe(
        &self,
        toolchain: &LatexToolchainInfo,
        request: &LatexWorkspaceCompileRequest<'_>,
    ) -> Result<LatexWorkspaceCompileRecipe, RunError> {
        match self {
            Self::Static(source) => (source.recipe)(toolchain, request),
            Self::Runtime(source) => source.recipe(toolchain, request).map_err(native_failure),
        }
    }

    // Native parsing can fail, so complete it before Core rechecks authorization
    // and publishes a retained result. The static pure callback retains its
    // original position after publication.
    pub(super) fn prepare_diagnostics(
        &self,
        raw_output: &str,
    ) -> Result<PreparedDiagnostics, RunError> {
        match self {
            Self::Static(source) => Ok(PreparedDiagnostics::Deferred(source.diagnostics)),
            Self::Runtime(source) => source
                .diagnostics(raw_output)
                .map(PreparedDiagnostics::Parsed)
                .map_err(native_failure),
        }
    }
}
