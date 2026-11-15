//! All child lookups are relative to open no-follow directory descriptors.
use super::LeaseError;
use crate::agent::instructions::directory::Directory;
use nix::{
    dir::Dir,
    fcntl::{AtFlags, OFlag, openat},
    sys::stat::{Mode, SFlag, fstatat, mkdirat},
    unistd::{UnlinkatFlags, unlinkat},
};
use std::{
    fs::File,
    io::{Read, Write},
    os::{fd::AsFd, unix::ffi::OsStrExt},
    path::Path,
};

pub fn sync(directory: &Directory) -> Result<(), LeaseError> {
    nix::unistd::fsync(directory).map_err(|_| LeaseError::Unavailable)
}
pub fn create_directory(parent: &Directory, name: &str) -> Result<Directory, LeaseError> {
    mkdirat(parent, name, Mode::S_IRWXU).map_err(|error| {
        if error == nix::errno::Errno::EEXIST {
            LeaseError::Conflict
        } else {
            LeaseError::Unavailable
        }
    })?;
    sync(parent)?;
    Ok(parent.child(Path::new(name))?)
}
pub fn create_file(
    parent: &Directory,
    name: &str,
    bytes: &[u8],
    executable: bool,
) -> Result<File, LeaseError> {
    let mode = if executable {
        Mode::S_IRWXU
    } else {
        Mode::S_IRUSR | Mode::S_IWUSR
    };
    let mut file = File::from(
        openat(
            parent,
            name,
            OFlag::O_RDWR | OFlag::O_CREAT | OFlag::O_EXCL | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC,
            mode,
        )
        .map_err(|_| LeaseError::Conflict)?,
    );
    file.write_all(bytes)?;
    file.sync_all()?;
    sync(parent)?;
    Ok(file)
}
pub fn read_file(parent: &Directory, name: &str, max: u64) -> Result<Vec<u8>, LeaseError> {
    let file = parent.config_file(name)?.ok_or(LeaseError::Incomplete)?;
    if file.metadata()?.len() > max {
        return Err(LeaseError::Budget);
    }
    let mut bytes = vec![];
    file.take(max + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > max {
        return Err(LeaseError::Budget);
    }
    Ok(bytes)
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    File,
    Directory,
    Other,
}
pub fn entries(
    parent: &Directory,
    budget: &mut usize,
) -> Result<Vec<(std::ffi::OsString, Kind)>, LeaseError> {
    let opened = Dir::openat(
        parent,
        ".",
        OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC,
        Mode::empty(),
    )
    .map_err(|_| LeaseError::Unavailable)?;
    let mut result = vec![];
    for entry in opened {
        let entry = entry.map_err(|_| LeaseError::Unavailable)?;
        let name = entry.file_name().to_bytes();
        if name == b"." || name == b".." {
            continue;
        }
        *budget = budget.checked_sub(1).ok_or(LeaseError::Budget)?;
        let name = std::ffi::OsStr::from_bytes(name);
        let stat = fstatat(parent, Path::new(name), AtFlags::AT_SYMLINK_NOFOLLOW)
            .map_err(|_| LeaseError::Unavailable)?;
        let kind = match SFlag::from_bits_truncate(stat.st_mode) & SFlag::S_IFMT {
            SFlag::S_IFDIR => Kind::Directory,
            SFlag::S_IFREG => Kind::File,
            _ => Kind::Other,
        };
        result.push((name.to_os_string(), kind));
    }
    result.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(result)
}
pub fn write_path(
    root: &Directory,
    path: &str,
    bytes: &[u8],
    executable: bool,
) -> Result<(), LeaseError> {
    research_agent_core::workspace::validate_path(path)?;
    let parts: Vec<_> = path.split('/').collect();
    let mut directory = root.child(Path::new("."))?;
    for part in &parts[..parts.len() - 1] {
        directory = match directory.optional_child(Path::new(part))? {
            Some(child) => child,
            None => create_directory(&directory, part)?,
        };
    }
    create_file(&directory, parts[parts.len() - 1], bytes, executable)?;
    Ok(())
}

/// Replace one existing regular file below an owned directory. The parent is
/// opened relative to a no-follow directory handle and the target is opened
/// with O_NOFOLLOW, so a model supplied path cannot redirect the write through
/// a symlink. Callers must validate the expected bytes before invoking this.
pub fn replace_file(
    root: &Directory,
    path: &str,
    bytes: &[u8],
    executable: bool,
) -> Result<(), LeaseError> {
    research_agent_core::workspace::validate_path(path)?;
    let parts: Vec<_> = path.split('/').collect();
    let name = parts.last().ok_or(LeaseError::Invalid)?;
    let mut directory = root.child(Path::new("."))?;
    for part in &parts[..parts.len() - 1] {
        directory = directory.child(Path::new(part))?;
        if directory.identity()?.0 != root.identity()?.0 {
            return Err(LeaseError::Invalid);
        }
    }
    let flags = OFlag::O_WRONLY | OFlag::O_TRUNC | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC;
    let file = openat(&directory, *name, flags, Mode::empty()).map_err(|_| LeaseError::Conflict)?;
    let mut file = File::from(file);
    let _ = executable;
    file.write_all(bytes)?;
    file.sync_all()?;
    sync(&directory)
}

pub fn remove_file(root: &Directory, path: &str) -> Result<(), LeaseError> {
    research_agent_core::workspace::validate_path(path)?;
    let parts: Vec<_> = path.split('/').collect();
    let name = parts.last().ok_or(LeaseError::Invalid)?;
    let mut directory = root.child(Path::new("."))?;
    for part in &parts[..parts.len() - 1] {
        directory = directory.child(Path::new(part))?;
        if directory.identity()?.0 != root.identity()?.0 {
            return Err(LeaseError::Invalid);
        }
    }
    unlinkat(directory.as_fd(), *name, UnlinkatFlags::NoRemoveDir)
        .map_err(|_| LeaseError::Conflict)?;
    sync(&directory)
}

pub fn read_path(root: &Directory, path: &str, max: u64) -> Result<Option<Vec<u8>>, LeaseError> {
    research_agent_core::workspace::validate_path(path)?;
    let parts: Vec<_> = path.split('/').collect();
    let name = parts.last().ok_or(LeaseError::Invalid)?;
    let mut directory = root.child(Path::new("."))?;
    for part in &parts[..parts.len() - 1] {
        directory = match directory.optional_child(Path::new(part))? {
            Some(child) => child,
            None => return Ok(None),
        };
        if directory.identity()?.0 != root.identity()?.0 {
            return Err(LeaseError::Invalid);
        }
    }
    let Some(file) = directory.config_file(name)? else {
        return Ok(None);
    };
    if !file.metadata()?.is_file() {
        return Err(LeaseError::Invalid);
    }
    if file.metadata()?.len() > max {
        return Err(LeaseError::Budget);
    }
    let mut bytes = Vec::new();
    file.take(max + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > max {
        return Err(LeaseError::Budget);
    }
    Ok(Some(bytes))
}
pub fn same_child(
    parent: &Directory,
    name: &std::ffi::OsStr,
    child: &Directory,
) -> Result<(), LeaseError> {
    let actual = parent
        .optional_child(Path::new(name))?
        .ok_or(LeaseError::Conflict)?;
    if actual.identity()? != child.identity()? {
        return Err(LeaseError::Conflict);
    }
    Ok(())
}
pub fn remove_tree(
    parent: &Directory,
    name: &std::ffi::OsStr,
    budget: &mut usize,
    depth: usize,
) -> Result<(), LeaseError> {
    if depth > 64 {
        return Err(LeaseError::Budget);
    }
    let child = parent.child(Path::new(name))?;
    // Never descend into a mounted foreign filesystem during collection/cleanup.
    if child.identity()?.0 != parent.identity()?.0 {
        return Err(LeaseError::Invalid);
    }
    for (name, kind) in entries(&child, budget)? {
        if kind == Kind::Directory {
            remove_tree(&child, &name, budget, depth + 1)?;
        } else {
            unlink_file(&child, &name)?;
        }
    }
    same_child(parent, name, &child)?;
    unlinkat(parent.as_fd(), name, UnlinkatFlags::RemoveDir)
        .map_err(|_| LeaseError::Unavailable)?;
    sync(parent)
}
pub fn unlink_file(parent: &Directory, name: &std::ffi::OsStr) -> Result<(), LeaseError> {
    unlinkat(parent.as_fd(), name, UnlinkatFlags::NoRemoveDir)
        .map_err(|_| LeaseError::Unavailable)?;
    Ok(())
}
