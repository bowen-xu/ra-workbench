//! Complete package copies reuse the native handle-relative workspace I/O.
use super::Error;
use crate::agent::{instructions::directory::Directory, workspace::storage};
use std::{fs, io::Read, path::Path};
const MAX_BYTES: u64 = 128 * 1024 * 1024;
pub(crate) fn checked_child(root: &Path, relative: &Path) -> Result<std::path::PathBuf, Error> {
    let root = root.canonicalize().map_err(|_| Error::Unavailable)?;
    let mut directory = Directory::root(&root).map_err(|_| Error::Invalid)?;
    let mut path = root;
    for component in relative.components() {
        let std::path::Component::Normal(name) = component else {
            return Err(Error::Invalid);
        };
        let name = name.to_str().ok_or(Error::Invalid)?;
        directory = match directory
            .optional_child(Path::new(name))
            .map_err(|_| Error::Invalid)?
        {
            Some(child) => child,
            None => storage::create_directory(&directory, name).map_err(|_| Error::Unavailable)?,
        };
        path.push(name);
    }
    Ok(path)
}
pub(crate) fn copy_package(from: &Path, to: &Path) -> Result<(), Error> {
    let mut remaining = MAX_BYTES;
    copy_package_with_budget(from, to, &mut remaining)
}
pub(crate) fn copy_package_with_budget(
    from: &Path,
    to: &Path,
    remaining: &mut u64,
) -> Result<(), Error> {
    if fs::symlink_metadata(from)
        .map_err(|_| Error::Unavailable)?
        .file_type()
        .is_symlink()
    {
        return Err(Error::Invalid);
    }
    let root = from.canonicalize().map_err(|_| Error::Unavailable)?;
    let source = Directory::root(&root).map_err(|_| Error::Invalid)?;
    fs::create_dir(to).map_err(|_| Error::Unavailable)?;
    let target = Directory::root(&to.canonicalize().map_err(|_| Error::Unavailable)?)
        .map_err(|_| Error::Invalid)?;
    let mut count = 20_000;
    visit(&source, &target, 0, &mut count, remaining)
}
fn visit(
    source: &Directory,
    target: &Directory,
    depth: usize,
    count: &mut usize,
    remaining: &mut u64,
) -> Result<(), Error> {
    if depth > 16 {
        return Err(Error::Invalid);
    }
    for (name, kind) in storage::entries(source, count).map_err(|_| Error::Invalid)? {
        if name == ".git" {
            continue;
        }
        let name = name.to_str().ok_or(Error::Invalid)?;
        match kind {
            storage::Kind::Directory => {
                let child = source.child(Path::new(name)).map_err(|_| Error::Invalid)?;
                let output =
                    storage::create_directory(target, name).map_err(|_| Error::Unavailable)?;
                visit(&child, &output, depth + 1, count, remaining)?;
            }
            storage::Kind::File => {
                let file = source
                    .config_file(name)
                    .map_err(|_| Error::Invalid)?
                    .ok_or(Error::Conflict)?;
                let metadata = file.metadata().map_err(|_| Error::Unavailable)?;
                if metadata.len() > *remaining {
                    return Err(Error::Invalid);
                }
                let mut bytes = Vec::new();
                file.take(*remaining + 1)
                    .read_to_end(&mut bytes)
                    .map_err(|_| Error::Unavailable)?;
                if bytes.len() as u64 > *remaining {
                    return Err(Error::Invalid);
                }
                if bytes.len() as u64 != metadata.len() {
                    return Err(Error::Conflict);
                }
                *remaining -= bytes.len() as u64;
                let output = storage::create_file(target, name, &bytes, false)
                    .map_err(|_| Error::Unavailable)?;
                output
                    .set_permissions(metadata.permissions())
                    .map_err(|_| Error::Unavailable)?;
            }
            storage::Kind::Other => return Err(Error::Invalid),
        }
    }
    Ok(())
}
