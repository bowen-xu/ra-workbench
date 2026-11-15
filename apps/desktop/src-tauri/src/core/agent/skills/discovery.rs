//! Bounded, deterministic discovery within one host-authorized skill root.
//! Follows Codex's discovery/load split; directory links are never followed.
use super::files::{SkillReadError, read_from_directory};
use crate::agent::instructions::{
    InstructionReadError, InstructionRootIdentity, directory::Directory,
};
use research_agent_core::skills::SkillDescriptor;
use std::path::Path;

#[derive(Debug)]
pub struct SkillDiscovery {
    pub skills: Vec<SkillDescriptor>,
    /// Relative source paths with structured failures; never raw source content.
    pub rejected: Vec<(String, SkillReadError)>,
}

/// Scan a registered root, not an arbitrary model-supplied path. Hidden folders
/// are excluded. Limits fail the discovery as a whole, never silently present a
/// partial catalog as complete. `max_depth = 1` selects direct child skill folders.
pub fn discover(
    root: &Path,
    environment: &str,
    max_depth: usize,
    authorize: impl FnOnce(&InstructionRootIdentity<'_>) -> Result<(), InstructionReadError>,
) -> Result<SkillDiscovery, SkillReadError> {
    discover_under(root, Path::new(""), environment, max_depth, authorize)
}

/// Discover a configured subdirectory while authorizing the containing workspace.
/// Missing skill folders are empty; links and malformed boundaries remain errors.
pub fn discover_under(
    root: &Path,
    relative: &Path,
    environment: &str,
    max_depth: usize,
    authorize: impl FnOnce(&InstructionRootIdentity<'_>) -> Result<(), InstructionReadError>,
) -> Result<SkillDiscovery, SkillReadError> {
    if relative.components().count() > 16
        || relative
            .components()
            .any(|part| !matches!(part, std::path::Component::Normal(_)))
    {
        return Err(InstructionReadError::InvalidScope.into());
    }
    if !root.is_absolute()
        || root
            .canonicalize()
            .map_err(|_| InstructionReadError::Unavailable)?
            != root
        || !(1..=8).contains(&max_depth)
        || environment.trim().is_empty()
        || environment.len() > 1024
        || environment.chars().any(char::is_control)
    {
        return Err(InstructionReadError::InvalidScope.into());
    }
    let mut directory = Directory::root(root)?;
    let (device, inode) = directory.identity()?;
    authorize(&InstructionRootIdentity {
        canonical_path: root,
        device,
        inode,
    })?;
    let mut result = SkillDiscovery {
        skills: vec![],
        rejected: vec![],
    };
    for part in relative.components() {
        let Some(child) = directory.optional_child(Path::new(part.as_os_str()))? else {
            return Ok(result);
        };
        directory = child;
    }
    let mut entries = 20_000;
    let mut directories = 512;
    visit(
        &directory,
        relative,
        environment,
        max_depth,
        &mut entries,
        &mut directories,
        &mut result,
    )?;
    Ok(result)
}

fn visit(
    directory: &Directory,
    relative: &Path,
    environment: &str,
    depth: usize,
    entries: &mut usize,
    directories: &mut usize,
    result: &mut SkillDiscovery,
) -> Result<(), SkillReadError> {
    for name in directory.child_directories(entries)? {
        let name = name.to_str().ok_or(InstructionReadError::InvalidScope)?;
        if name.starts_with('.') {
            continue;
        }
        *directories = directories
            .checked_sub(1)
            .ok_or(InstructionReadError::Budget)?;
        let folder = relative.join(name);
        let source = folder.join("SKILL.md");
        let source_id = source
            .to_str()
            .filter(|value| value.len() <= 1024)
            .ok_or(InstructionReadError::InvalidScope)?;
        let child = directory
            .optional_child(Path::new(name))?
            .ok_or(SkillReadError::NotFound)?;
        match read_from_directory(&child, &source, environment) {
            Ok(parsed) => {
                if result.skills.len() == 256
                    || result
                        .skills
                        .iter()
                        .map(|skill| skill.description.len())
                        .sum::<usize>()
                        + parsed.descriptor.description.len()
                        > 1024 * 1024
                {
                    return Err(InstructionReadError::Budget.into());
                }
                result.skills.push(parsed.descriptor);
            }
            Err(SkillReadError::NotFound) => {}
            Err(error) => result.rejected.push((source_id.to_owned(), error)),
        }
        if depth > 1 {
            visit(
                &child,
                &folder,
                environment,
                depth - 1,
                entries,
                directories,
                result,
            )?;
        }
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::{fs, os::unix::fs::symlink};
    fn skill(root: &Path, folder: &str) {
        fs::create_dir_all(root.join(folder)).unwrap();
        fs::write(
            root.join(folder).join("SKILL.md"),
            "---\ndescription: Review\n---\nBody",
        )
        .unwrap();
    }
    #[test]
    fn finds_sorted_nested_metadata_with_explicit_rejections_and_no_links() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        skill(&root, "zeta");
        skill(&root, "alpha/nested");
        skill(&root, ".hidden");
        skill(&root, "invalid");
        fs::write(root.join("invalid/SKILL.md"), "invalid YAML document").unwrap();
        symlink(root.join("zeta"), root.join("linked")).unwrap();
        let result = discover(&root, "project", 3, |_| Ok(())).unwrap();
        assert_eq!(
            result
                .skills
                .iter()
                .map(|s| s.identity.source_id.as_str())
                .collect::<Vec<_>>(),
            vec!["alpha/nested/SKILL.md", "zeta/SKILL.md"]
        );
        assert_eq!(result.rejected.len(), 1);
        assert_eq!(result.rejected[0].0, "invalid/SKILL.md");
        assert_eq!(
            discover(&root, "project", 1, |_| Ok(()))
                .unwrap()
                .skills
                .len(),
            1
        );
    }
    #[test]
    fn pins_authorized_root_and_rejects_unbounded_catalog() {
        let temp = tempfile::tempdir().unwrap();
        let parent = temp.path().canonicalize().unwrap();
        let root = parent.join("root");
        skill(&root, "original");
        let result = discover(&root, "project", 1, |_| {
            fs::rename(&root, parent.join("moved")).unwrap();
            skill(&root, "replacement");
            Ok(())
        })
        .unwrap();
        assert_eq!(result.skills[0].name, "original");
        assert!(matches!(
            discover(&root, "project", 1, |_| Err(
                InstructionReadError::Unauthorized
            )),
            Err(SkillReadError::Read(InstructionReadError::Unauthorized))
        ));
        for index in 0..256 {
            skill(&root, &format!("skill-{index}"));
        }
        assert!(matches!(
            discover(&root, "project", 1, |_| Ok(())),
            Err(SkillReadError::Read(InstructionReadError::Budget))
        ));
    }
}
