//! App-owned default skills, embedded independently of the developer's skill
//! directory. Seed complete packages once, then reuse ordinary catalog, policy,
//! versioned reads and removal. No second skill registry or execution owner.
use super::import::{Error, managed};
use crate::agent::instructions::directory::Directory;
use std::{fs, io::Write, path::Path};

const NAME: &str = "edit-notes";
const RECEIPT: &str = ".bundled-edit-notes.initialized";
const SKILL: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../agent-resources/skills/edit-notes/SKILL.md"
));
const PROTOCOL: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../agent-resources/skills/edit-notes/references/text-view.md"
));

const STRUCTURED_SKILL: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../agent-resources/skills/edit-structured-resources/SKILL.md"
));
const PORTRAIT_SKILL: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../agent-resources/skills/generate-paper-portrait/SKILL.md"
));
const STRUCTURED_REFERENCES: &[(&str, &str)] = &[
    (
        "portrait.md",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../agent-resources/skills/edit-structured-resources/references/portrait.md"
        )),
    ),
    (
        "reading.md",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../agent-resources/skills/edit-structured-resources/references/reading.md"
        )),
    ),
    (
        "templates.md",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../agent-resources/skills/edit-structured-resources/references/templates.md"
        )),
    ),
];

/// Initialize only a previously absent default. An existing package is user
/// owned, including older versions. The durable receipt preserves later removal
/// across restarts; enablement is never written here. New bundled defaults need
/// their own receipt. Upgrading an installed skill remains an explicit import.
pub(crate) fn install_defaults(configuration_directory: &Path) -> Result<(), Error> {
    let parent = managed(configuration_directory)?;
    // Reuse the same no-follow root boundary as catalog/import reads.
    let directory = Directory::root(&parent).map_err(|_| Error::Invalid)?;
    let mut options = fs::OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK);
    }
    let lock_path = parent.join(".bundled-skills.lock");
    // Non-Unix OpenOptions has no O_NOFOLLOW; reject existing links before open.
    if fs::symlink_metadata(&lock_path).is_ok_and(|metadata| !metadata.is_file()) {
        return Err(Error::Invalid);
    }
    let lock = options.open(lock_path).map_err(|_| Error::Unavailable)?;
    if !lock.metadata().map_err(|_| Error::Unavailable)?.is_file() {
        return Err(Error::Invalid);
    }
    fs2::FileExt::lock_exclusive(&lock).map_err(|_| Error::Unavailable)?;
    install_package(
        configuration_directory,
        &parent,
        &directory,
        NAME,
        RECEIPT,
        SKILL,
        &[("text-view.md", PROTOCOL)],
    )?;
    install_package(
        configuration_directory,
        &parent,
        &directory,
        "edit-structured-resources",
        ".bundled-edit-structured-resources.initialized",
        STRUCTURED_SKILL,
        STRUCTURED_REFERENCES,
    )?;
    install_package(
        configuration_directory,
        &parent,
        &directory,
        "generate-paper-portrait",
        ".bundled-generate-paper-portrait.initialized",
        PORTRAIT_SKILL,
        &[],
    )
}

fn install_package(
    configuration_directory: &Path,
    parent: &Path,
    directory: &Directory,
    name: &str,
    receipt: &str,
    skill: &str,
    references: &[(&str, &str)],
) -> Result<(), Error> {
    if directory
        .config_file(receipt)
        .map_err(|_| Error::Invalid)?
        .is_some()
    {
        return Ok(());
    }
    let same_name_exists = super::application::discover(configuration_directory)
        .map_err(|_| Error::Invalid)?
        .skills
        .iter()
        .any(|skill| skill.name == name);
    let destination = parent.join(name);
    match fs::symlink_metadata(&destination) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
        Ok(_) => return Err(Error::Invalid),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if same_name_exists {
                return record_initialized(parent, directory, receipt);
            }
            // The private staging tree stays hidden from catalog discovery until
            // the whole skill and its resources are ready. Never publish half a
            // package or merge embedded files into an existing user package.
            let staging = tempfile::Builder::new()
                .prefix(".bundled-")
                .tempdir_in(parent)
                .map_err(|_| Error::Unavailable)?;
            let package = staging.path().join(name);
            fs::create_dir_all(package.join("references")).map_err(|_| Error::Unavailable)?;
            write_synced(&package.join("SKILL.md"), skill)?;
            for (filename, text) in references {
                write_synced(&package.join("references").join(filename), text)?;
            }
            super::parse_document(
                "application",
                &format!(".agents/skills/{name}/SKILL.md"),
                name,
                skill.as_bytes(),
            )
            .map_err(|_| Error::Invalid)?;
            #[cfg(unix)]
            for path in [package.join("references"), package.clone()] {
                let opened = Directory::root(&path).map_err(|_| Error::Invalid)?;
                nix::unistd::fsync(&opened).map_err(|_| Error::Unavailable)?;
            }
            fs::rename(&package, &destination).map_err(|_| Error::Unavailable)?;
        }
        Err(_) => return Err(Error::Unavailable),
    }
    record_initialized(parent, directory, receipt)
}

fn record_initialized(
    parent: &Path,
    directory: &Directory,
    receipt_name: &str,
) -> Result<(), Error> {
    let mut receipt = tempfile::NamedTempFile::new_in(parent).map_err(|_| Error::Unavailable)?;
    receipt
        .write_all(b"initialized-v1\n")
        .map_err(|_| Error::Unavailable)?;
    receipt
        .as_file()
        .sync_all()
        .map_err(|_| Error::Unavailable)?;
    receipt
        .persist_noclobber(parent.join(receipt_name))
        .map_err(|_| Error::Unavailable)?;
    #[cfg(unix)]
    nix::unistd::fsync(directory).map_err(|_| Error::Unavailable)?;
    Ok(())
}

fn write_synced(path: &Path, text: &str) -> Result<(), Error> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|_| Error::Unavailable)?;
    file.write_all(text.as_bytes())
        .map_err(|_| Error::Unavailable)?;
    file.sync_all().map_err(|_| Error::Unavailable)
}

#[cfg(test)]
#[path = "bundled/tests.rs"]
mod tests;
