//! Package import adapted from Codex core-plugins/marketplace_add/install.rs
//! and the bundled skill-installer (e363b08c9175ac1cbe5893615dd2cb9ddf95043b).
//! Copyright 2025 OpenAI, Apache-2.0. See packages/agent-core/third-party/codex.
//! Application adaptation: HTTPS only, opaque preview handles, complete packages,
//! explicit revision-bound replacement, bounded no-link copies and atomic rename.
use super::{discovery, parse_document};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashSet},
    fs,
    io::Write,
    path::{Component, Path, PathBuf},
    sync::{Mutex, OnceLock},
};
use tempfile::TempDir;
use uuid::Uuid;

#[derive(Clone, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum Source {
    Local {
        directory: PathBuf,
    },
    Git {
        url: String,
        #[serde(rename = "ref")]
        reference: Option<String>,
        subdirectory: Option<String>,
    },
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Candidate {
    pub source_id: String,
    pub name: String,
    pub revision: String,
    pub existing_revision: Option<String>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    pub import_id: Uuid,
    pub candidates: Vec<Candidate>,
    pub resolved_commit: Option<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Selection {
    pub source_id: String,
    pub expected_existing_revision: Option<String>,
}
#[derive(Serialize, Debug, thiserror::Error)]
#[serde(rename_all = "camelCase")]
pub enum Error {
    #[error("技能包无效、含链接或超过预算")]
    Invalid,
    #[error("技能版本已变化，请重新扫描；同名需要显式更新")]
    Conflict,
    #[error("无法读取或安装技能包")]
    Unavailable,
    #[error("Git 获取失败，请检查 HTTPS 地址、ref 与依赖")]
    Git,
}
struct Prepared {
    temporary: TempDir,
    source: Source,
    commit: Option<String>,
    candidates: Vec<Candidate>,
    configuration: PathBuf,
}
static IMPORTS: OnceLock<Mutex<BTreeMap<Uuid, Prepared>>> = OnceLock::new();
fn imports() -> &'static Mutex<BTreeMap<Uuid, Prepared>> {
    IMPORTS.get_or_init(Default::default)
}
pub(crate) fn managed(root: &Path) -> Result<PathBuf, Error> {
    fs::create_dir_all(root).map_err(|_| Error::Unavailable)?;
    let canonical = root.canonicalize().map_err(|_| Error::Unavailable)?;
    if fs::symlink_metadata(root)
        .map_err(|_| Error::Unavailable)?
        .file_type()
        .is_symlink()
    {
        return Err(Error::Invalid);
    }
    let mut directory = canonical;
    for name in [".agents", "skills"] {
        directory = directory.join(name);
        match fs::symlink_metadata(&directory) {
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
            Ok(_) => return Err(Error::Invalid),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir(&directory).map_err(|_| Error::Unavailable)?
            }
            Err(_) => return Err(Error::Unavailable),
        }
    }
    Ok(directory)
}
#[path = "import/package.rs"]
mod package;
pub(crate) use package::{checked_child, copy_package, copy_package_with_budget};
fn relative(value: &str) -> Result<PathBuf, Error> {
    let path = Path::new(value);
    if value.len() > 1024
        || path
            .components()
            .any(|p| !matches!(p, Component::Normal(_)))
        || path.as_os_str().is_empty()
    {
        return Err(Error::Invalid);
    }
    Ok(path.to_owned())
}
async fn git(cwd: &Path, args: &[&str]) -> Result<String, Error> {
    let mut command = tokio::process::Command::new("git");
    command
        .args(args)
        .current_dir(cwd)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .kill_on_drop(true);
    let output = tokio::time::timeout(std::time::Duration::from_secs(120), command.output())
        .await
        .map_err(|_| Error::Git)?
        .map_err(|_| Error::Git)?;
    if !output.status.success() || output.stdout.len() > 1024 * 1024 {
        return Err(Error::Git);
    }
    String::from_utf8(output.stdout).map_err(|_| Error::Git)
}
pub async fn scan(root: PathBuf, source: Source) -> Result<Preview, Error> {
    let parent = managed(&root)?;
    let temporary = tempfile::Builder::new()
        .prefix(".import-")
        .tempdir_in(&parent)
        .map_err(|_| Error::Unavailable)?;
    let mut commit = None;
    let origin = match &source {
        Source::Local { directory } => {
            if !directory.is_absolute() {
                return Err(Error::Invalid);
            }
            directory.clone()
        }
        Source::Git {
            url,
            reference,
            subdirectory,
        } => {
            let parsed = reqwest::Url::parse(url).map_err(|_| Error::Invalid)?;
            if parsed.scheme() != "https"
                || parsed.host_str().is_none()
                || !parsed.username().is_empty()
                || parsed.password().is_some()
                || parsed.query().is_some()
                || parsed.fragment().is_some()
            {
                return Err(Error::Invalid);
            }
            let reference = reference.as_deref().unwrap_or("HEAD");
            if reference.is_empty()
                || reference.len() > 256
                || reference.starts_with('-')
                || reference.chars().any(char::is_control)
            {
                return Err(Error::Invalid);
            }
            git(
                temporary.path(),
                &[
                    "clone",
                    "--filter=blob:none",
                    "--no-checkout",
                    "--",
                    url,
                    "checkout",
                ],
            )
            .await?;
            let checkout = temporary.path().join("checkout");
            if let Some(directory) = subdirectory {
                relative(directory)?;
                git(&checkout, &["sparse-checkout", "set", "--", directory]).await?;
            }
            git(&checkout, &["checkout", reference, "--"]).await?;
            let resolved = git(&checkout, &["rev-parse", "HEAD"])
                .await?
                .trim()
                .to_owned();
            if resolved.len() != 40 || !resolved.chars().all(|c| c.is_ascii_hexdigit()) {
                return Err(Error::Git);
            }
            commit = Some(resolved);
            match subdirectory {
                Some(directory) => checkout.join(relative(directory)?),
                None => checkout,
            }
        }
    };
    let staging = temporary.path().join("packages");
    let target = if origin.join("SKILL.md").is_file() {
        fs::create_dir(&staging).map_err(|_| Error::Unavailable)?;
        let name = origin
            .file_name()
            .and_then(|v| v.to_str())
            .ok_or(Error::Invalid)?;
        relative(name)?;
        let target = staging.join(name);
        copy_package(&origin, &target)?;
        staging.clone()
    } else {
        copy_package(&origin, &staging)?;
        staging.clone()
    };
    let catalog = discovery::discover(
        &target.canonicalize().map_err(|_| Error::Unavailable)?,
        "import",
        8,
        |_| Ok(()),
    )
    .map_err(|_| Error::Invalid)?;
    if catalog.skills.is_empty() || !catalog.rejected.is_empty() {
        return Err(Error::Invalid);
    }
    let existing = super::application::discover(&root).map_err(|_| Error::Unavailable)?;
    let candidates: Vec<Candidate> = catalog
        .skills
        .into_iter()
        .map(|skill| {
            let source_id = format!(".agents/skills/{}", skill.identity.source_id);
            let existing_revision = existing
                .skills
                .iter()
                .find(|s| s.identity.source_id == source_id || s.name == skill.name)
                .map(|s| s.identity.revision.clone());
            Candidate {
                source_id,
                name: skill.name,
                revision: skill.identity.revision,
                existing_revision,
            }
        })
        .collect();
    let id = Uuid::new_v4();
    let response = Preview {
        import_id: id,
        candidates: candidates.clone(),
        resolved_commit: commit.clone(),
    };
    let mut imports = imports().lock().map_err(|_| Error::Unavailable)?;
    if imports.len() >= 8 {
        return Err(Error::Conflict);
    }
    imports.insert(
        id,
        Prepared {
            temporary,
            source,
            commit,
            candidates,
            configuration: root,
        },
    );
    Ok(response)
}
pub fn install(root: &Path, id: Uuid, selections: Vec<Selection>) -> Result<(), Error> {
    let mut imports = imports().lock().map_err(|_| Error::Unavailable)?;
    let preview = imports.get(&id).ok_or(Error::Conflict)?;
    if preview.configuration != root || selections.is_empty() || selections.len() > 256 {
        return Err(Error::Invalid);
    }
    let parent = managed(root)?;
    let mut seen = HashSet::new();
    let catalog = super::application::discover(root).map_err(|_| Error::Unavailable)?;
    let mut operations: Vec<(PathBuf, PathBuf, PathBuf, bool)> = Vec::new();
    for selection in selections {
        if !seen.insert(selection.source_id.clone()) {
            return Err(Error::Invalid);
        }
        let candidate = preview
            .candidates
            .iter()
            .find(|c| c.source_id == selection.source_id)
            .ok_or(Error::Invalid)?;
        let old = catalog
            .skills
            .iter()
            .find(|s| s.identity.source_id == selection.source_id);
        let same_name = catalog.skills.iter().find(|s| s.name == candidate.name);
        let existing = old.or(same_name);
        if existing.map(|s| s.identity.revision.as_str())
            != selection.expected_existing_revision.as_deref()
        {
            return Err(Error::Conflict);
        }
        // An update keeps the existing precise source identity.
        let destination = existing
            .map(|s| s.identity.source_id.as_str())
            .unwrap_or(&selection.source_id);
        let selected = relative(
            selection
                .source_id
                .strip_prefix(".agents/skills/")
                .ok_or(Error::Invalid)?,
        )?;
        let destination = relative(
            destination
                .strip_prefix(".agents/skills/")
                .ok_or(Error::Invalid)?,
        )?;
        let source = preview
            .temporary
            .path()
            .join("packages")
            .join(selected)
            .parent()
            .ok_or(Error::Invalid)?
            .to_owned();
        let target = parent
            .join(destination)
            .parent()
            .ok_or(Error::Invalid)?
            .to_owned();
        let stage = preview
            .temporary
            .path()
            .join(format!("install-{}", operations.len()));
        copy_package(&source, &stage)?;
        let parsed = parse_document(
            "application",
            &selection.source_id,
            &candidate.name,
            &fs::read(stage.join("SKILL.md")).map_err(|_| Error::Unavailable)?,
        )
        .map_err(|_| Error::Invalid)?;
        if parsed.descriptor.identity.revision != candidate.revision {
            return Err(Error::Conflict);
        }
        if operations.iter().any(|(_, old, _, _)| {
            old == &target || target.starts_with(old) || old.starts_with(&target)
        }) {
            return Err(Error::Invalid);
        }
        operations.push((
            stage,
            target,
            preview
                .temporary
                .path()
                .join(format!("backup-{}", operations.len())),
            existing.is_some(),
        ));
    }
    // Prepare and sync provenance before touching any installed package.
    let metadata = serde_json::json!({"source":preview.source,"resolvedCommit":preview.commit,"installed":preview.candidates.iter().filter(|candidate|seen.contains(&candidate.source_id)).collect::<Vec<_>>()});
    let metadata_root = checked_child(root, Path::new("agent/skill-origins"))?;
    let mut file =
        tempfile::NamedTempFile::new_in(&metadata_root).map_err(|_| Error::Unavailable)?;
    file.write_all(
        serde_json::to_string(&metadata)
            .map_err(|_| Error::Invalid)?
            .as_bytes(),
    )
    .map_err(|_| Error::Unavailable)?;
    file.as_file().sync_all().map_err(|_| Error::Unavailable)?;
    for (_, target, _, exists) in &operations {
        let relative_parent = target
            .parent()
            .ok_or(Error::Invalid)?
            .strip_prefix(&parent)
            .map_err(|_| Error::Invalid)?;
        checked_child(&parent, relative_parent)?;
        match fs::symlink_metadata(target) {
            Ok(meta) if *exists && meta.is_dir() && !meta.file_type().is_symlink() => {}
            Err(error) if !exists && error.kind() == std::io::ErrorKind::NotFound => {}
            _ => return Err(Error::Conflict),
        }
    }
    let mut done = 0;
    let result = (|| {
        for (stage, target, backup, exists) in &operations {
            if *exists {
                fs::rename(target, backup).map_err(|_| Error::Unavailable)?;
            }
            if fs::rename(stage, target).is_err() {
                if *exists {
                    let _ = fs::rename(backup, target);
                }
                return Err(Error::Unavailable);
            }
            done += 1;
        }
        fs::File::open(&parent)
            .and_then(|dir| dir.sync_all())
            .map_err(|_| Error::Unavailable)?;
        file.persist(metadata_root.join(format!("{id}.json")))
            .map_err(|_| Error::Unavailable)?;
        Ok(())
    })();
    if let Err(error) = result {
        for (_, target, backup, existed) in operations[..done].iter().rev() {
            fs::remove_dir_all(target).map_err(|_| Error::Unavailable)?;
            if *existed {
                fs::rename(backup, target).map_err(|_| Error::Unavailable)?;
            }
        }
        return Err(error);
    }
    imports.remove(&id);
    Ok(())
}

pub fn remove(root: &Path, source_id: &str, revision: &str) -> Result<(), Error> {
    let _imports = imports().lock().map_err(|_| Error::Unavailable)?;
    let current = super::application::discover(root).map_err(|_| Error::Unavailable)?;
    if !current
        .skills
        .iter()
        .any(|s| s.identity.source_id == source_id && s.identity.revision == revision)
    {
        return Err(Error::Conflict);
    }
    let path = relative(
        source_id
            .strip_prefix(".agents/skills/")
            .ok_or(Error::Invalid)?,
    )?;
    let parent = managed(root)?;
    let package = parent.join(path).parent().ok_or(Error::Invalid)?.to_owned();
    let trash = parent.join(format!(".removed-{}", Uuid::new_v4()));
    fs::rename(package, &trash).map_err(|_| Error::Unavailable)?;
    fs::remove_dir_all(trash).map_err(|_| Error::Unavailable)
}
pub fn cancel(id: Uuid) -> Result<(), Error> {
    imports()
        .lock()
        .map_err(|_| Error::Unavailable)?
        .remove(&id);
    Ok(())
}

#[cfg(test)]
#[path = "import/tests.rs"]
mod tests;
