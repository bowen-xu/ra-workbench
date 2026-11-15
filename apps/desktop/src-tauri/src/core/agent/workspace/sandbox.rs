//! One shared protected-resource policy for native workspace execution.
use crate::storage::AppStore;
use research_agent_core::runner::RunError;
use std::path::{Path, PathBuf};
pub(crate) fn protected_paths(store: &AppStore, work: &Path) -> Result<Vec<PathBuf>, RunError> {
    let data = store
        .manuscript_directory()
        .parent()
        .ok_or(RunError::ToolRejected)?
        .canonicalize()
        .map_err(|_| RunError::ToolRejected)?;
    let config = store.agent_configuration_directory();
    let config = if config.exists() {
        config.canonicalize()
    } else {
        config
            .parent()
            .ok_or(RunError::ToolRejected)?
            .canonicalize()
            .map(|parent| parent.join(config.file_name().unwrap_or_default()))
    }
    .map_err(|_| RunError::ToolRejected)?;
    protected_paths_for_roots(&data, &config, work)
}

pub(crate) fn protected_paths_for_roots(
    data: &Path,
    config: &Path,
    work: &Path,
) -> Result<Vec<PathBuf>, RunError> {
    // On macOS, Tauri's app data and app config directories commonly resolve
    // to the same Application Support root. Denying that ancestor after
    // granting the work subpath would also deny the compiler's output files.
    // Seatbelt already defaults to deny and only grants `work`, so an
    // ancestor of work does not need a separate deny rule. A config directory
    // inside work would be an unsafe layout and must fail closed.
    let mut protected = if config.starts_with(work) {
        return Err(RunError::ToolRejected);
    } else if work.starts_with(config) {
        Vec::new()
    } else {
        vec![config.to_path_buf()]
    };
    for name in [
        "manuscripts",
        "objects",
        "semantic-indexes",
        "cache",
        "backups",
        "research-assistant.sqlite3",
        "research-assistant.sqlite3-wal",
        "research-assistant.sqlite3-shm",
    ] {
        protected.push(data.join(name));
    }
    Ok(protected)
}
