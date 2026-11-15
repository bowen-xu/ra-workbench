//! Native PTY allocation adapted from Codex utils/pty/src/pty.rs at
//! e363b08c9175ac1cbe5893615dd2cb9ddf95043b. Copyright 2025 OpenAI. Apache-2.0;
//! see packages/agent-core/third-party/codex/{LICENSE,NOTICE}.
use super::{ProcessError, TerminalSize};
use std::{
    fs::File,
    io::{Read, Write},
    os::fd::{AsRawFd, FromRawFd},
    sync::Arc,
};
use tokio::io::unix::AsyncFd;

pub(super) fn open(size: TerminalSize) -> Result<(Arc<AsyncFd<File>>, File), ProcessError> {
    size.validate()?;
    let mut master = -1;
    let mut slave = -1;
    let mut size = size.native();
    // SAFETY: live output integers and winsize are supplied; null termios/name
    // request defaults. On success each returned FD is transferred once below.
    if unsafe {
        nix::libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut size,
        )
    } != 0
    {
        return Err(std::io::Error::last_os_error().into());
    }
    // SAFETY: successful openpty returned two distinct, newly owned FDs.
    let (master, slave) = unsafe { (File::from_raw_fd(master), File::from_raw_fd(slave)) };
    for file in [&master, &slave] {
        nix::fcntl::fcntl(
            file,
            nix::fcntl::FcntlArg::F_SETFD(nix::fcntl::FdFlag::FD_CLOEXEC),
        )
        .map_err(|error| std::io::Error::from_raw_os_error(error as i32))?;
    }
    let flags = nix::fcntl::fcntl(&master, nix::fcntl::FcntlArg::F_GETFL)
        .map_err(|error| std::io::Error::from_raw_os_error(error as i32))?;
    nix::fcntl::fcntl(
        &master,
        nix::fcntl::FcntlArg::F_SETFL(
            nix::fcntl::OFlag::from_bits_truncate(flags) | nix::fcntl::OFlag::O_NONBLOCK,
        ),
    )
    .map_err(|error| std::io::Error::from_raw_os_error(error as i32))?;
    Ok((Arc::new(AsyncFd::new(master)?), slave))
}
pub(super) async fn read(master: &AsyncFd<File>, bytes: &mut [u8]) -> Result<usize, ProcessError> {
    loop {
        let mut ready = master.readable().await?;
        match ready.try_io(|fd| {
            let mut file = fd.get_ref();
            file.read(bytes)
        }) {
            Ok(Ok(count)) => return Ok(count),
            // Linux reports EIO when the last slave closes; macOS reports EOF.
            Ok(Err(error)) if error.raw_os_error() == Some(nix::libc::EIO) => return Ok(0),
            Ok(Err(error)) => return Err(error.into()),
            Err(_) => {}
        }
    }
}
pub struct PtyInput(pub(super) Option<Arc<AsyncFd<File>>>);
impl PtyInput {
    /// Cancel-safe single write; callers advance by the returned byte count.
    pub async fn write(&mut self, bytes: &[u8]) -> Result<usize, ProcessError> {
        let master = self
            .0
            .as_ref()
            .ok_or(ProcessError::Io(std::io::ErrorKind::BrokenPipe))?;
        loop {
            let mut ready = master.writable().await?;
            if let Ok(result) = ready.try_io(|fd| {
                let mut file = fd.get_ref();
                file.write(bytes)
            }) {
                return result.map_err(Into::into);
            }
        }
    }
    /// Release this input handle. This is NOT terminal EOF: output still owns
    /// the master. Send the terminal's VEOF byte or terminate the process.
    pub fn close(&mut self) {
        self.0.take();
    }
}
pub(super) fn resize(master: &AsyncFd<File>, size: TerminalSize) -> Result<(), ProcessError> {
    size.validate()?;
    let native = size.native();
    // SAFETY: ioctl receives a live master FD and a pointer to a valid winsize.
    if unsafe { nix::libc::ioctl(master.get_ref().as_raw_fd(), nix::libc::TIOCSWINSZ, &native) } < 0
    {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}
