//! Original fixed Agent recipe and filesystem read-root policy. No process is launched here.
use super::tools;
use crate::agent::{
    writing_compile_contracts::{
        LatexToolchainInfo, LatexWorkspaceCompileRecipe, LatexWorkspaceCompileRequest,
    },
    writing_error::WritingError,
};
use research_workbench_native_sdk::writing_compile::{EngineArgument, Flag};
use std::{
    collections::BTreeSet,
    ffi::OsString,
    path::{Path, PathBuf},
    process::Command,
};

pub(crate) fn program(toolchain: &LatexToolchainInfo) -> Result<PathBuf, WritingError> {
    tools::executable(toolchain, "latexmk")
        .and_then(|path| path.canonicalize().map_err(Into::into))
        .map_err(|_| WritingError::ToolchainUnavailable)
}

pub(crate) fn assemble(
    program: PathBuf,
    toolchain: &LatexToolchainInfo,
    request: &LatexWorkspaceCompileRequest<'_>,
    engine: EngineArgument,
    flags: &[Flag],
) -> Result<LatexWorkspaceCompileRecipe, WritingError> {
    let mut command = Command::new(&program);
    command.args([
        OsString::from("-norc"),
        OsString::from(engine.argument()),
        OsString::from("-interaction=nonstopmode"),
        OsString::from("-file-line-error"),
        OsString::from("-synctex=1"),
        OsString::from("-latexoption=-no-shell-escape"),
        OsString::from(format!(
            "-outdir={}",
            request.output_directory.to_string_lossy()
        )),
        OsString::from(format!(
            "-auxdir={}",
            request.output_directory.to_string_lossy()
        )),
    ]);
    command.args(flags.iter().copied().map(Flag::argument));
    command.arg(request.root_path);
    Ok(LatexWorkspaceCompileRecipe {
        program,
        arguments: command.get_args().map(|a| a.to_os_string()).collect(),
        path: tools::compile_path(toolchain),
        read_only: toolchain_read_roots(toolchain)?,
    })
}

fn toolchain_read_roots(toolchain: &LatexToolchainInfo) -> Result<Vec<PathBuf>, WritingError> {
    let mut roots = BTreeSet::new();
    for path in toolchain
        .tools
        .iter()
        .filter_map(|tool| tool.path.as_deref())
    {
        let path = Path::new(path)
            .canonicalize()
            .map_err(|_| WritingError::ToolchainUnavailable)?;
        if let Some(parent) = path.parent()
            && !parent.starts_with("/usr")
            && !parent.starts_with("/bin")
            && !parent.starts_with("/System")
        {
            roots.insert(parent.to_path_buf());
        }
    }
    // Preserve the original MacTeX public-path grants and their canonical forms.
    for candidate in ["/usr/local/texlive", "/Library", "/usr"] {
        let path = Path::new(candidate);
        if path.exists() {
            roots.insert(
                path.canonicalize()
                    .map_err(|_| WritingError::ToolchainUnavailable)?,
            );
        }
    }
    if roots.len() > 32 {
        return Err(WritingError::ToolchainUnavailable);
    }
    Ok(roots.into_iter().collect())
}
