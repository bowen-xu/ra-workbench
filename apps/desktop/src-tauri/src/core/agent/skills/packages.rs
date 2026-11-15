//! Read-only skill packages outside the writable execution tree. The original
//! skills remain private; only registered, exact-version packages are copied.
use crate::{
    agent::{instructions::InstructionReadError, skills::files::load_selected},
    storage::AppStore,
};
use research_agent_core::{runner::RunError, skills::SkillIdentity, submission::SubmissionRecord};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
};
pub(crate) fn relative(identity: &SkillIdentity) -> String {
    format!(
        "../skills/{:x}",
        Sha256::digest(
            format!(
                "{}/{}/{}",
                identity.environment_id, identity.source_id, identity.revision
            )
            .as_bytes()
        )
    )
}
pub(crate) fn freeze(
    store: &AppStore,
    record: &SubmissionRecord,
    work: &Path,
) -> Result<Vec<PathBuf>, RunError> {
    let base = work.parent().ok_or(RunError::ToolRejected)?.join("skills");
    if base.exists() {
        if fs::symlink_metadata(&base)
            .map_err(|_| RunError::ToolRejected)?
            .file_type()
            .is_symlink()
        {
            return Err(RunError::ToolRejected);
        }
    } else {
        fs::create_dir(&base).map_err(|_| RunError::ToolRejected)?;
    }
    let mut remaining = 128 * 1024 * 1024;
    for skill in &record.input.skills {
        let identity = &skill.descriptor.identity;
        let root = if identity.environment_id == "application" {
            store.agent_configuration_directory().to_owned()
        } else if let Some(id) = identity.environment_id.strip_prefix("manuscript:") {
            let id = uuid::Uuid::parse_str(id).map_err(|_| RunError::ToolRejected)?;
            store
                .manuscript_directory()
                .join(id.to_string())
                .join("source")
        } else {
            return Err(RunError::ToolRejected);
        };
        let root = root.canonicalize().map_err(|_| RunError::ToolRejected)?;
        load_selected(&root, &skill.descriptor, true, 1024 * 1024, |opened| {
            if identity.environment_id == "application" && opened.canonical_path == root {
                return Ok(());
            }
            let id = uuid::Uuid::parse_str(
                identity
                    .environment_id
                    .strip_prefix("manuscript:")
                    .ok_or(InstructionReadError::Unauthorized)?,
            )
            .map_err(|_| InstructionReadError::Unauthorized)?;
            let db = store
                .connection()
                .map_err(|_| InstructionReadError::Unavailable)?;
            crate::agent::repository::instruction_trust::is_trusted(&db, id, opened)
                .map_err(|_| InstructionReadError::Unavailable)?
                .then_some(())
                .ok_or(InstructionReadError::Unauthorized)
        })
        .map_err(|_| RunError::ToolRejected)?;
        let destination = work.join(relative(identity));
        if destination.exists() {
            return Err(RunError::ToolRejected);
        } // never silently reuse mutable contents
        let source = root
            .join(&identity.source_id)
            .parent()
            .ok_or(RunError::ToolRejected)?
            .to_owned();
        super::import::copy_package_with_budget(&source, &destination, &mut remaining)
            .map_err(|_| RunError::ToolRejected)?;
    }
    Ok(vec![base])
}
