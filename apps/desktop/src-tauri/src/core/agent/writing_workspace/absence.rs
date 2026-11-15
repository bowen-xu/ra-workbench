//! Original no-follow inventory-absence check, owned by Core file I/O.
use crate::agent::instructions::directory::Directory;
use crate::agent::writing_context::WritingContextError as Error;
use std::path::{Component, Path};
use uuid::Uuid;

/// Registered inventory absence alone is not enough for creating a new file.
/// Check each existing parent without following links; any occupied target,
/// including a directory, symlink or unregistered file, is a conflict.
pub fn check_absent(root: &Path, manuscript: Uuid, path: &str) -> Result<(), Error> {
    let relative = crate::scoped_text_files::legacy::validate_relative_path(path)
        .map_err(|_| Error::InvalidInput)?;
    let source = crate::scoped_text_files::legacy::manuscript_source_directory(
        root,
        &manuscript.to_string(),
    )
    .map_err(|_| Error::Unavailable)?;
    let mut directory = Directory::root(&source).map_err(|_| Error::Unavailable)?;
    let device = directory.identity().map_err(|_| Error::Unavailable)?.0;
    let mut components = relative.components().peekable();
    while let Some(Component::Normal(name)) = components.next() {
        if components.peek().is_none() {
            return match nix::sys::stat::fstatat(
                &directory,
                Path::new(name),
                nix::fcntl::AtFlags::AT_SYMLINK_NOFOLLOW,
            ) {
                Err(nix::errno::Errno::ENOENT) => Ok(()),
                Ok(_) => Err(Error::Conflict),
                Err(_) => Err(Error::Unavailable),
            };
        }
        match directory
            .optional_child(Path::new(name))
            .map_err(|_| Error::Conflict)?
        {
            None => return Ok(()),
            Some(child) => {
                if child.identity().map_err(|_| Error::Unavailable)?.0 != device {
                    return Err(Error::Conflict);
                }
                directory = child;
            }
        }
    }
    Err(Error::InvalidInput)
}
