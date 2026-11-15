//! Handle-relative reads avoid re-resolving mutable ancestor paths.
use super::InstructionReadError;
use std::{fs::File, path::Path};

#[cfg(unix)]
mod unix {
    use super::*;
    use nix::{
        errno::Errno,
        fcntl::{OFlag, open, openat},
        sys::stat::Mode,
    };
    use std::path::Component;

    pub struct Directory(File);
    impl std::os::fd::AsFd for Directory {
        fn as_fd(&self) -> std::os::fd::BorrowedFd<'_> {
            std::os::fd::AsFd::as_fd(&self.0)
        }
    }

    fn directory_flags() -> OFlag {
        OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC
    }
    fn error(value: Errno) -> InstructionReadError {
        match value {
            Errno::ELOOP | Errno::ENOTDIR => InstructionReadError::InvalidScope,
            _ => InstructionReadError::Unavailable,
        }
    }
    impl Directory {
        pub fn identity(&self) -> Result<(u64, u64), InstructionReadError> {
            use std::os::unix::fs::MetadataExt;
            let metadata = self
                .0
                .metadata()
                .map_err(|_| InstructionReadError::Unavailable)?;
            Ok((metadata.dev(), metadata.ino()))
        }
        pub fn root(path: &Path) -> Result<Self, InstructionReadError> {
            let mut directory = Self(File::from(
                open(Path::new("/"), directory_flags(), Mode::empty()).map_err(error)?,
            ));
            for component in path.components() {
                match component {
                    Component::RootDir => {}
                    Component::Normal(name) => directory = directory.child(Path::new(name))?,
                    _ => return Err(InstructionReadError::InvalidScope),
                }
            }
            Ok(directory)
        }
        pub fn child(&self, name: &Path) -> Result<Self, InstructionReadError> {
            Ok(Self(File::from(
                openat(&self.0, name, directory_flags(), Mode::empty()).map_err(error)?,
            )))
        }
        pub fn optional_child(&self, name: &Path) -> Result<Option<Self>, InstructionReadError> {
            match openat(&self.0, name, directory_flags(), Mode::empty()) {
                Ok(fd) => Ok(Some(Self(File::from(fd)))),
                Err(Errno::ENOENT) => Ok(None),
                Err(value) => Err(error(value)),
            }
        }
        /// Enumerate from this handle; count all entries before filtering so a
        /// directory full of unrelated files cannot bypass the traversal budget.
        pub fn child_directories(
            &self,
            remaining_entries: &mut usize,
        ) -> Result<Vec<std::ffi::OsString>, InstructionReadError> {
            use nix::{
                dir::Dir,
                fcntl::AtFlags,
                sys::stat::{SFlag, fstatat},
            };
            use std::os::unix::ffi::OsStrExt;
            let directory =
                Dir::openat(&self.0, ".", directory_flags(), Mode::empty()).map_err(error)?;
            let mut names = vec![];
            for entry in directory {
                let entry = entry.map_err(error)?;
                let name = entry.file_name().to_bytes();
                if name == b"." || name == b".." {
                    continue;
                }
                *remaining_entries = remaining_entries
                    .checked_sub(1)
                    .ok_or(InstructionReadError::Budget)?;
                let name = std::ffi::OsStr::from_bytes(name);
                let stat = fstatat(&self.0, Path::new(name), AtFlags::AT_SYMLINK_NOFOLLOW)
                    .map_err(error)?;
                if (SFlag::from_bits_truncate(stat.st_mode) & SFlag::S_IFMT) == SFlag::S_IFDIR {
                    names.push(name.to_os_string());
                }
            }
            names.sort();
            Ok(names)
        }
        pub fn file(&self, name: &str) -> Result<Option<File>, InstructionReadError> {
            self.read_file(name, false)
        }
        pub fn config_file(&self, name: &str) -> Result<Option<File>, InstructionReadError> {
            self.read_file(name, true)
        }
        fn read_file(
            &self,
            name: &str,
            strict: bool,
        ) -> Result<Option<File>, InstructionReadError> {
            // NONBLOCK prevents a concurrent replacement with a FIFO from hanging
            // before the opened handle's regular-file metadata can be checked.
            let flags = OFlag::O_RDONLY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC | OFlag::O_NONBLOCK;
            let fd = match openat(&self.0, name, flags, Mode::empty()) {
                Ok(fd) => fd,
                Err(Errno::ENOENT) => return Ok(None),
                Err(value) => return Err(error(value)),
            };
            let file = File::from(fd);
            let metadata = file
                .metadata()
                .map_err(|_| InstructionReadError::Unavailable)?;
            if !metadata.is_file() {
                if strict {
                    return Err(InstructionReadError::InvalidScope);
                }
                return Ok(None);
            }
            Ok(Some(file))
        }
    }
}
#[cfg(unix)]
pub use unix::Directory;

#[cfg(not(unix))]
pub struct Directory;
#[cfg(not(unix))]
impl Directory {
    pub fn identity(&self) -> Result<(u64, u64), InstructionReadError> {
        Err(InstructionReadError::UnsupportedPlatform)
    }
    pub fn root(_: &Path) -> Result<Self, InstructionReadError> {
        Err(InstructionReadError::UnsupportedPlatform)
    }
    pub fn child(&self, _: &Path) -> Result<Self, InstructionReadError> {
        Err(InstructionReadError::UnsupportedPlatform)
    }
    pub fn optional_child(&self, _: &Path) -> Result<Option<Self>, InstructionReadError> {
        Err(InstructionReadError::UnsupportedPlatform)
    }
    pub fn child_directories(
        &self,
        _: &mut usize,
    ) -> Result<Vec<std::ffi::OsString>, InstructionReadError> {
        Err(InstructionReadError::UnsupportedPlatform)
    }
    pub fn file(&self, _: &str) -> Result<Option<File>, InstructionReadError> {
        Err(InstructionReadError::UnsupportedPlatform)
    }
    pub fn config_file(&self, _: &str) -> Result<Option<File>, InstructionReadError> {
        Err(InstructionReadError::UnsupportedPlatform)
    }
}
