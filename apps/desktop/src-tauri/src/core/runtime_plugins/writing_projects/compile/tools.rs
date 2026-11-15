//! Original bounded probes and executable path adaptation.
use std::{
    collections::HashSet,
    env,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use crate::agent::{
    writing_compile_contracts::{LatexTool, LatexToolchainInfo},
    writing_error::WritingError,
};

const TOOL_NAMES: [&str; 8] = [
    "latexmk",
    "pdflatex",
    "xelatex",
    "lualatex",
    "bibtex",
    "biber",
    "synctex",
    "kpsewhich",
];

pub(crate) fn inspect(runtime_directory: &Path) -> Vec<LatexTool> {
    let _ = std::fs::create_dir_all(runtime_directory);
    TOOL_NAMES
        .iter()
        .map(|name| inspect_tool(name, runtime_directory))
        .collect()
}

pub(crate) fn executable(
    toolchain: &LatexToolchainInfo,
    name: &str,
) -> Result<PathBuf, WritingError> {
    toolchain
        .tools
        .iter()
        .find(|tool| tool.name == name && tool.available)
        .and_then(|tool| tool.path.as_deref())
        .map(PathBuf::from)
        .ok_or(WritingError::ToolchainUnavailable)
}

pub(crate) fn compile_path(toolchain: &LatexToolchainInfo) -> String {
    let mut directories = toolchain
        .tools
        .iter()
        .filter_map(|tool| tool.path.as_deref())
        .filter_map(|path| Path::new(path).parent())
        .map(Path::to_path_buf)
        .collect::<Vec<_>>();
    directories.extend(search_directories());
    deduplicate_paths(directories)
        .into_iter()
        .map(|path| path.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(":")
}

fn inspect_tool(name: &str, runtime_directory: &Path) -> LatexTool {
    let Some(path) = resolve_executable(name) else {
        return LatexTool {
            name: name.to_owned(),
            available: false,
            path: None,
            version: None,
        };
    };
    LatexTool {
        name: name.to_owned(),
        available: true,
        version: read_version(&path, name, runtime_directory),
        path: Some(path.to_string_lossy().into_owned()),
    }
}

fn resolve_executable(name: &str) -> Option<PathBuf> {
    search_directories()
        .into_iter()
        .map(|directory| directory.join(name))
        .find(|path| path.is_file())
}

fn search_directories() -> Vec<PathBuf> {
    let mut directories = vec![
        PathBuf::from("/Library/TeX/texbin"),
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
        PathBuf::from("/usr/bin"),
        PathBuf::from("/bin"),
    ];
    if let Some(path) = env::var_os("PATH") {
        directories.extend(env::split_paths(&path));
    }
    deduplicate_paths(directories)
}

fn deduplicate_paths(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    paths
        .into_iter()
        .filter(|path| seen.insert(path.clone()))
        .collect()
}

fn read_version(path: &Path, name: &str, runtime_directory: &Path) -> Option<String> {
    let argument = match name {
        "latexmk" => "-v",
        _ => "--version",
    };
    let mut child = Command::new(path)
        .arg(argument)
        .current_dir(runtime_directory)
        .env_clear()
        .env("PATH", compile_path_for(path))
        .env("HOME", runtime_directory)
        .env("TMPDIR", runtime_directory)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(20)),
            Ok(None) | Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let output = child.wait_with_output().ok()?;
    let bytes = if output.stdout.is_empty() {
        output.stderr
    } else {
        output.stdout
    };
    String::from_utf8_lossy(&bytes)
        .lines()
        .find(|line| !line.trim().is_empty())
        .map(|line| line.trim().chars().take(180).collect())
}

fn compile_path_for(executable: &Path) -> String {
    let mut directories = executable
        .parent()
        .map(Path::to_path_buf)
        .into_iter()
        .collect::<Vec<_>>();
    directories.extend(search_directories());
    deduplicate_paths(directories)
        .into_iter()
        .map(|path| path.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(":")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compile_path_has_no_duplicate_directories() {
        let value = compile_path_for(Path::new("/usr/bin/latexmk"));
        let parts = value.split(':').collect::<Vec<_>>();
        assert_eq!(
            parts.iter().copied().collect::<HashSet<_>>().len(),
            parts.len()
        );
    }
}
