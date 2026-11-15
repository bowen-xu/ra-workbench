//! Shared original SyncTeX process and scoped-file adapters. Domain parsing stays in writing.
use super::{process, tools as toolchain};
use crate::agent::writing_error::WritingError;
use research_workbench_native_sdk::writing_compile::{LatexToolchainInfo, SynctexSourceLocation};
use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, atomic::AtomicBool},
    time::Duration,
};
const QUERY_TIMEOUT: Duration = Duration::from_secs(10);

pub(crate) struct RunPaths {
    pub(crate) snapshot: PathBuf,
    pub(crate) output_directory: PathBuf,
    pub(crate) pdf: PathBuf,
}

pub(crate) fn ensure_current_input(
    manuscript_root: &Path,
    manuscript_id: &str,
    expected_hash: &str,
) -> Result<(), WritingError> {
    let source = crate::scoped_text_files::legacy::manuscript_source_directory(
        manuscript_root,
        manuscript_id,
    )?;
    if super::snapshot::source_manifest_hash(&source)? != expected_hash {
        return Err(WritingError::SynctexUnavailable);
    }
    Ok(())
}

pub(crate) fn run_paths(
    cache_root: &Path,
    run_id: &str,
    manuscript_id: &str,
    root_path: &str,
) -> Result<RunPaths, WritingError> {
    let root = cache_root.join(manuscript_id).join(run_id);
    let snapshot = root.join("snapshot");
    let output_directory = root.join("output");
    let stem = Path::new(root_path)
        .file_stem()
        .and_then(|value| value.to_str())
        .ok_or(WritingError::InvalidPath)?;
    let pdf = output_directory.join(format!("{stem}.pdf"));
    let synctex = output_directory.join(format!("{stem}.synctex.gz"));
    if !pdf.is_file() || !synctex.is_file() || !snapshot.is_dir() {
        return Err(WritingError::SynctexUnavailable);
    }
    Ok(RunPaths {
        snapshot,
        output_directory,
        pdf,
    })
}

pub(crate) fn run_synctex(
    runtime_home: &Path,
    toolchain_json: &str,
    current_directory: &Path,
    arguments: impl IntoIterator<Item = String>,
) -> Result<String, WritingError> {
    let toolchain_info: LatexToolchainInfo = serde_json::from_str(toolchain_json)?;
    let executable = toolchain::executable(&toolchain_info, "synctex")?;
    let mut command = Command::new(executable);
    command
        .current_dir(current_directory)
        .env_clear()
        .env("PATH", toolchain::compile_path(&toolchain_info))
        .env("HOME", runtime_home)
        .env("TMPDIR", runtime_home)
        .args(arguments);
    let outcome = process::run(
        &mut command,
        Arc::new(AtomicBool::new(false)),
        QUERY_TIMEOUT,
    )?;
    let output = if outcome.stderr.is_empty() {
        outcome.stdout
    } else {
        format!("{}\n{}", outcome.stdout, outcome.stderr)
    };
    if !outcome.status.success() {
        return Err(WritingError::SynctexNoResult);
    }
    Ok(output)
}

fn push_source_location(
    locations: &mut Vec<SynctexSourceLocation>,
    snapshot: &Path,
    input: Option<String>,
    line: Option<u32>,
    column: Option<u32>,
) -> Result<(), WritingError> {
    let (Some(input), Some(line)) = (input, line) else {
        return Ok(());
    };
    let input_path = PathBuf::from(input);
    let absolute = resolve_input_path(snapshot, &input_path)?;
    // SyncTeX commonly reports an otherwise valid path with a `/./` segment.
    // Canonicalize both existing paths before enforcing the snapshot boundary;
    // string cleanup alone could accidentally accept a traversal or symlink.
    let canonical_snapshot = snapshot.canonicalize()?;
    let canonical_input = absolute.canonicalize()?;
    let relative = canonical_input
        .strip_prefix(canonical_snapshot)
        .map_err(|_| WritingError::InvalidPath)?
        .to_str()
        .ok_or(WritingError::InvalidPath)?
        .replace('\\', "/");
    crate::scoped_text_files::legacy::validate_relative_path(&relative)?;
    locations.push(SynctexSourceLocation {
        relative_path: relative,
        line,
        column: column.filter(|value| *value > 0),
    });
    Ok(())
}

/// SyncTeX stores the absolute path of the temporary compile workspace. Once
/// that workspace has expired, the retained result still contains the path but
/// only the identity-bound source snapshot remains. Resolve the longest
/// existing suffix inside that snapshot; never accept a path that cannot be
/// proven to land below the snapshot root.
fn resolve_input_path(snapshot: &Path, input: &Path) -> Result<PathBuf, WritingError> {
    if !input.is_absolute() {
        return Ok(snapshot.join(input));
    }
    if input.is_file() {
        let canonical_snapshot = snapshot.canonicalize()?;
        let canonical_input = input.canonicalize()?;
        if canonical_input.starts_with(&canonical_snapshot) {
            return Ok(input.to_path_buf());
        }
    }
    let components: Vec<_> = input
        .components()
        .filter_map(|component| match component {
            std::path::Component::Normal(value) => Some(value.to_os_string()),
            _ => None,
        })
        .collect();
    for start in 0..components.len() {
        let candidate = components[start..]
            .iter()
            .fold(snapshot.to_path_buf(), |path, component| {
                path.join(component)
            });
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    Err(WritingError::InvalidPath)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn forward_output(
    runtime_home: &Path,
    toolchain_json: &str,
    snapshot: &Path,
    output_directory: &Path,
    pdf: &Path,
    relative_path: &str,
    line: u32,
    column: u32,
) -> Result<String, WritingError> {
    let relative_path = crate::scoped_text_files::legacy::validate_relative_path(relative_path)?;
    let source = snapshot.join(relative_path);
    if !source.is_file() {
        return Err(WritingError::FileNotFound);
    }
    run_synctex(
        runtime_home,
        toolchain_json,
        output_directory,
        [
            "view".to_owned(),
            "-i".to_owned(),
            format!("{line}:{column}:{}", source.to_string_lossy()),
            "-o".to_owned(),
            pdf.to_string_lossy().into_owned(),
        ],
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn inverse_output(
    runtime_home: &Path,
    toolchain_json: &str,
    output_directory: &Path,
    pdf: &Path,
    page: u32,
    x: f64,
    y: f64,
) -> Result<String, WritingError> {
    run_synctex(
        runtime_home,
        toolchain_json,
        output_directory,
        [
            "edit".to_owned(),
            "-o".to_owned(),
            format!("{page}:{x:.3}:{y:.3}:{}", pdf.to_string_lossy()),
        ],
    )
}

pub(crate) struct ResolvedInputs {
    pub(crate) locations: Vec<SynctexSourceLocation>,
    pub(crate) invalid_input: bool,
}

pub(crate) fn resolve_inputs(
    snapshot: &Path,
    items: Vec<research_workbench_native_sdk::writing_compile::synctex::InverseCandidate>,
) -> Result<ResolvedInputs, WritingError> {
    let mut locations = Vec::new();
    let mut invalid_input = false;
    for item in items {
        if let Err(error) = push_source_location(
            &mut locations,
            snapshot,
            Some(item.input),
            Some(item.line),
            item.column,
        ) {
            if matches!(error, WritingError::InvalidPath) {
                invalid_input = true;
            } else {
                return Err(error);
            }
        }
    }
    Ok(ResolvedInputs {
        locations,
        invalid_input,
    })
}
