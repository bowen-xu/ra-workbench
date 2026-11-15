//! Adapted from Codex utils/pty/src/pty.rs::{set_cloexec,close_inherited_fds_except}
//! at e363b08c9175ac1cbe5893615dd2cb9ddf95043b. Copyright 2025 OpenAI. Apache-2.0;
//! see packages/agent-core/third-party/codex/{LICENSE,NOTICE}.
//! No allocation, directory enumeration or mutex acquisition after fork.
use std::io;

pub(super) struct DescriptorPlan {
    #[cfg(target_os = "macos")]
    upper: i32,
}
impl DescriptorPlan {
    pub fn prepare() -> io::Result<Self> {
        #[cfg(target_os = "macos")]
        {
            let mut limit: nix::libc::c_int = 0;
            let mut size = std::mem::size_of_val(&limit);
            // SAFETY: sysctl writes exactly into a live integer, with its size
            // supplied. This runs in the parent, before entering pre_exec.
            let status = unsafe {
                nix::libc::sysctlbyname(
                    c"kern.maxfilesperproc".as_ptr(),
                    (&mut limit as *mut i32).cast(),
                    &mut size,
                    std::ptr::null_mut(),
                    0,
                )
            };
            if status != 0 {
                return Err(io::Error::last_os_error());
            }
            if size != std::mem::size_of_val(&limit) || limit < 3 {
                return Err(io::Error::from(io::ErrorKind::InvalidData));
            }
            // The kernel ceiling covers descriptors opened by other threads
            // before fork, unlike a snapshot of just currently open descriptors.
            // Also cover existing descriptors if an administrator lowered it.
            for entry in std::fs::read_dir("/dev/fd")? {
                let entry = entry?;
                let name = entry.file_name();
                let fd: i32 = name
                    .to_str()
                    .and_then(|name| name.parse().ok())
                    .ok_or(io::ErrorKind::InvalidData)?;
                limit = limit.max(fd.checked_add(1).ok_or(io::ErrorKind::InvalidData)?);
            }
            // Do not silently scan only a prefix on an unusually configured OS.
            if limit > 1_048_576 {
                return Err(io::ErrorKind::Unsupported.into());
            }
            Ok(Self { upper: limit })
        }
        #[cfg(target_os = "linux")]
        {
            Ok(Self {})
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            Err(io::ErrorKind::Unsupported.into())
        }
    }
    /// Must run in the child's pre_exec hook. Marking instead of closing keeps
    /// std's exec-error pipe usable if a later setup step or exec fails.
    pub fn apply(&self) -> io::Result<()> {
        #[cfg(target_os = "macos")]
        for fd in 3..self.upper {
            // SAFETY: fcntl is async-signal-safe; it accesses the child's copied
            // descriptor table only. No owned parent handle is borrowed/closed.
            let flags = unsafe { nix::libc::fcntl(fd, nix::libc::F_GETFD) };
            if flags < 0 {
                let error = io::Error::last_os_error();
                if error.raw_os_error() == Some(nix::libc::EBADF) {
                    continue;
                }
                return Err(error);
            }
            // SAFETY: same invariant; preserving flags also preserves the Rust
            // error pipe's existing CLOEXEC bit until exec succeeds.
            if unsafe { nix::libc::fcntl(fd, nix::libc::F_SETFD, flags | nix::libc::FD_CLOEXEC) }
                < 0
            {
                return Err(io::Error::last_os_error());
            }
        }
        #[cfg(target_os = "linux")]
        {
            // SAFETY: direct kernel syscall with scalar arguments; CLOEXEC does
            // not close std's error pipe before exec. No unsupported fallback.
            if unsafe {
                nix::libc::syscall(
                    nix::libc::SYS_close_range,
                    3u32,
                    u32::MAX,
                    nix::libc::CLOSE_RANGE_CLOEXEC,
                )
            } < 0
            {
                return Err(io::Error::last_os_error());
            }
        }
        Ok(())
    }
}
