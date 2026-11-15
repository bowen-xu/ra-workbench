//! Authorized skill reads through the same pinned directory adapter as AGENTS.
use super::{ParsedSkill, SkillParseError, parse_document};
use crate::agent::instructions::{
    InstructionReadError, InstructionRootIdentity, directory::Directory,
};
use research_agent_core::skills::{LoadedSkill, SkillDescriptor, SkillError, accept_loaded_skill};
use std::{
    io::Read,
    path::{Component, Path},
};

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum SkillReadError {
    #[error("skill source unavailable or unauthorized")]
    Read(#[from] InstructionReadError),
    #[error("skill source no longer exists")]
    NotFound,
    #[error("invalid skill document")]
    Parse(#[from] SkillParseError),
    #[error("skill selection is invalid or stale")]
    Selection(#[from] SkillError),
}

/// `relative` identifies a skill folder under a catalog root, ending in SKILL.md.
/// Authorization runs on the opened root before any child lookup. The caller must
/// bind environment/root to its trust policy, never accept model-provided roots.
pub(super) fn open_skill_directory(
    root: &Path,
    relative: &Path,
    environment: &str,
    authorize: impl FnOnce(&InstructionRootIdentity<'_>) -> Result<(), InstructionReadError>,
) -> Result<Directory, SkillReadError> {
    let components: Vec<_> = relative.components().collect();
    if !root.is_absolute()
        || root
            .canonicalize()
            .map_err(|_| InstructionReadError::Unavailable)?
            != root
        || components.len() < 2
        || components.len() > 64
        || components
            .iter()
            .any(|part| !matches!(part, Component::Normal(_)))
        || relative.file_name().is_none_or(|name| name != "SKILL.md")
        || environment.trim().is_empty()
        || environment.len() > 1024
        || environment.chars().any(char::is_control)
    {
        return Err(InstructionReadError::InvalidScope.into());
    }
    relative
        .to_str()
        .filter(|value| value.len() <= 1024)
        .ok_or(InstructionReadError::InvalidScope)?;
    components[components.len() - 2]
        .as_os_str()
        .to_str()
        .ok_or(InstructionReadError::InvalidScope)?;
    let mut directory = Directory::root(root)?;
    let (device, inode) = directory.identity()?;
    authorize(&InstructionRootIdentity {
        canonical_path: root,
        device,
        inode,
    })?;
    for part in &components[..components.len() - 1] {
        directory = directory
            .optional_child(Path::new(part.as_os_str()))?
            .ok_or(SkillReadError::NotFound)?;
    }
    Ok(directory)
}

fn read_document(
    root: &Path,
    relative: &Path,
    environment: &str,
    authorize: impl FnOnce(&InstructionRootIdentity<'_>) -> Result<(), InstructionReadError>,
) -> Result<ParsedSkill, SkillReadError> {
    let directory = open_skill_directory(root, relative, environment, authorize)?;
    let original = read_from_directory(&directory, relative, environment)?;
    if environment == "application"
        && let Some(text) = crate::system_documents::skill_override(
            root,
            relative
                .to_str()
                .ok_or(InstructionReadError::InvalidScope)?,
        )
        .map_err(|_| InstructionReadError::Unavailable)?
    {
        let name = relative
            .parent()
            .and_then(Path::file_name)
            .and_then(|v| v.to_str())
            .ok_or(InstructionReadError::InvalidScope)?;
        let mut parsed = parse_document(
            environment,
            relative.to_str().unwrap(),
            name,
            text.as_bytes(),
        )?;
        parsed.descriptor.allow_implicit_invocation = original.descriptor.allow_implicit_invocation;
        return Ok(parsed);
    }
    Ok(original)
}

pub(super) fn read_from_directory(
    directory: &Directory,
    relative: &Path,
    environment: &str,
) -> Result<ParsedSkill, SkillReadError> {
    let source = relative
        .to_str()
        .filter(|value| value.len() <= 1024)
        .ok_or(InstructionReadError::InvalidScope)?;
    let default_name = relative
        .parent()
        .and_then(Path::file_name)
        .and_then(|value| value.to_str())
        .ok_or(InstructionReadError::InvalidScope)?;
    let file = directory
        .config_file("SKILL.md")?
        .ok_or(SkillReadError::NotFound)?;
    const LIMIT: u64 = 1024 * 1024;
    if file
        .metadata()
        .map_err(|_| InstructionReadError::Unavailable)?
        .len()
        > LIMIT
    {
        return Err(InstructionReadError::Budget.into());
    }
    let mut bytes = vec![];
    file.take(LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| InstructionReadError::Unavailable)?;
    if bytes.len() as u64 > LIMIT {
        return Err(InstructionReadError::Budget.into());
    }
    let mut parsed = parse_document(environment, source, default_name, &bytes)?;
    if let Some(metadata) = directory.optional_child(Path::new("agents"))?
        && let Some(file) = metadata.config_file("openai.yaml")?
    {
        let mut policy = Vec::new();
        file.take(65537)
            .read_to_end(&mut policy)
            .map_err(|_| InstructionReadError::Unavailable)?;
        super::policy::apply(&mut parsed, &policy)?;
    }
    Ok(parsed)
}

/// Catalog discovery retains metadata only; the full source hash includes body.
/// No body or executable declaration is returned to the model at discovery time.
pub fn read_descriptor(
    root: &Path,
    relative: &Path,
    environment: &str,
    authorize: impl FnOnce(&InstructionRootIdentity<'_>) -> Result<(), InstructionReadError>,
) -> Result<SkillDescriptor, SkillReadError> {
    Ok(read_document(root, relative, environment, authorize)?.descriptor)
}

/// Selected descriptor must come from the host's frozen catalog. Reauthorization
/// and rehashing happen on every demand load, so revoked or edited skills fail.
pub fn load_selected(
    root: &Path,
    selected: &SkillDescriptor,
    explicit: bool,
    max_bytes: usize,
    authorize: impl FnOnce(&InstructionRootIdentity<'_>) -> Result<(), InstructionReadError>,
) -> Result<LoadedSkill, SkillReadError> {
    selected.validate()?;
    if !explicit && !selected.allow_implicit_invocation {
        return Err(SkillError::ExplicitRequired.into());
    }
    if max_bytes > 1024 * 1024 {
        return Err(SkillError::Budget.into());
    }
    let parsed = read_document(
        root,
        Path::new(&selected.identity.source_id),
        &selected.identity.environment_id,
        authorize,
    )?;
    Ok(accept_loaded_skill(
        selected,
        parsed.loaded,
        explicit,
        max_bytes,
    )?)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::{fs, os::unix::fs::symlink};
    fn write(root: &Path, body: &str) {
        fs::create_dir_all(root.join("proofread")).unwrap();
        fs::write(
            root.join("proofread/SKILL.md"),
            format!("---\ndescription: Review prose\n---\n{body}"),
        )
        .unwrap();
    }
    #[test]
    fn loads_selected_version_and_rejects_edit_revocation_and_missing_source() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        write(&root, "Original body");
        let selected =
            read_descriptor(
                &root,
                Path::new("proofread/SKILL.md"),
                "project",
                |_| Ok(()),
            )
            .unwrap();
        assert_eq!(selected.name, "proofread");
        assert_eq!(
            load_selected(&root, &selected, true, 4096, |_| Ok(()))
                .unwrap()
                .body,
            "Original body"
        );
        assert_eq!(
            load_selected(&root, &selected, true, 4096, |_| Err(
                InstructionReadError::Unauthorized
            )),
            Err(SkillReadError::Read(InstructionReadError::Unauthorized))
        );
        write(&root, "Changed body");
        assert_eq!(
            load_selected(&root, &selected, true, 4096, |_| Ok(())),
            Err(SkillReadError::Selection(SkillError::Stale))
        );
        fs::remove_file(root.join("proofread/SKILL.md")).unwrap();
        assert_eq!(
            load_selected(&root, &selected, true, 4096, |_| Ok(())),
            Err(SkillReadError::NotFound)
        );
    }
    #[test]
    fn sidecar_is_required_by_frozen_version_and_cannot_be_a_link() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        write(&root, "Original body");
        fs::create_dir(root.join("proofread/agents")).unwrap();
        let sidecar = root.join("proofread/agents/openai.yaml");
        fs::write(&sidecar, "policy: {allow_implicit_invocation: false}").unwrap();
        let selected =
            read_descriptor(
                &root,
                Path::new("proofread/SKILL.md"),
                "project",
                |_| Ok(()),
            )
            .unwrap();
        assert!(!selected.allow_implicit_invocation);
        assert_eq!(
            load_selected(&root, &selected, false, 4096, |_| Ok(())),
            Err(SkillReadError::Selection(SkillError::ExplicitRequired))
        );
        assert_eq!(
            load_selected(&root, &selected, true, 4096, |_| Ok(()))
                .unwrap()
                .body,
            "Original body"
        );
        fs::remove_file(&sidecar).unwrap();
        assert_eq!(
            load_selected(&root, &selected, true, 4096, |_| Ok(())),
            Err(SkillReadError::Selection(SkillError::Stale))
        );
        symlink(root.join("proofread/SKILL.md"), &sidecar).unwrap();
        assert!(
            read_descriptor(
                &root,
                Path::new("proofread/SKILL.md"),
                "project",
                |_| Ok(())
            )
            .is_err()
        );
    }
    #[test]
    fn explicit_policy_is_checked_before_reading_source() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        write(&root, "Original body");
        let mut selected = read_descriptor(
            &root,
            Path::new("proofread/SKILL.md"),
            "project",
            |_| Ok(()),
        )
        .unwrap();
        selected.allow_implicit_invocation = false;
        fs::remove_dir_all(root.join("proofread")).unwrap();
        assert_eq!(
            load_selected(&root, &selected, false, 4096, |_| {
                panic!("implicit policy must reject before source authorization")
            }),
            Err(SkillReadError::Selection(SkillError::ExplicitRequired))
        );
        assert_eq!(
            load_selected(&root, &selected, true, 4096, |_| Ok(())),
            Err(SkillReadError::NotFound)
        );
    }
    #[test]
    fn pins_root_and_rejects_links_fifo_and_parent_traversal() {
        let temp = tempfile::tempdir().unwrap();
        let container = temp.path().canonicalize().unwrap();
        let root = container.join("catalog");
        write(&root, "Original");
        let selected = read_descriptor(&root, Path::new("proofread/SKILL.md"), "project", |_| {
            fs::rename(&root, container.join("original")).unwrap();
            write(&root, "Replacement");
            Ok(())
        })
        .unwrap();
        assert_eq!(
            load_selected(&container.join("original"), &selected, true, 4096, |_| Ok(
                ()
            ))
            .unwrap()
            .body,
            "Original"
        );
        assert!(
            read_descriptor(
                &root,
                Path::new("../original/proofread/SKILL.md"),
                "project",
                |_| Ok(())
            )
            .is_err()
        );
        fs::remove_file(root.join("proofread/SKILL.md")).unwrap();
        symlink(
            container.join("original/proofread/SKILL.md"),
            root.join("proofread/SKILL.md"),
        )
        .unwrap();
        assert!(
            read_descriptor(
                &root,
                Path::new("proofread/SKILL.md"),
                "project",
                |_| Ok(())
            )
            .is_err()
        );
        fs::remove_file(root.join("proofread/SKILL.md")).unwrap();
        nix::unistd::mkfifo(
            &root.join("proofread/SKILL.md"),
            nix::sys::stat::Mode::S_IRUSR,
        )
        .unwrap();
        assert!(
            read_descriptor(
                &root,
                Path::new("proofread/SKILL.md"),
                "project",
                |_| Ok(())
            )
            .is_err()
        );
    }
}
