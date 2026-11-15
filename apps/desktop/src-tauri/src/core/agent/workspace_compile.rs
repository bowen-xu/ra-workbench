//! Statically selected writing operations for an already-authorized work copy.
//! The host retains the store, lease, sandbox, process registry and cleanup.
//! These functions grant no process execution or database access to the provider.
use crate::agent::writing_compile_contracts::{
    CompileDiagnostic, LatexToolchainInfo, LatexWorkspaceCompileRecipe,
    LatexWorkspaceCompileRequest,
};
use research_agent_core::runner::RunError;
use std::path::Path;

type Detect = fn(String, &Path) -> LatexToolchainInfo;
type Recipe = fn(
    &LatexToolchainInfo,
    &LatexWorkspaceCompileRequest<'_>,
) -> Result<LatexWorkspaceCompileRecipe, RunError>;
type Diagnostics = fn(&str) -> Vec<CompileDiagnostic>;

#[derive(Clone, Copy)]
pub(crate) struct WritingWorkspaceCompileSource {
    pub detect: Detect,
    pub recipe: Recipe,
    pub diagnostics: Diagnostics,
}
