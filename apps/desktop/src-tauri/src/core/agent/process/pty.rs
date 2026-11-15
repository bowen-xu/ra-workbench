//! PTY transport adapted from Codex utils/pty/src/pty.rs at
//! e363b08c9175ac1cbe5893615dd2cb9ddf95043b. Copyright 2025 OpenAI. Apache-2.0;
//! see packages/agent-core/third-party/codex/{LICENSE,NOTICE}.
//! Shared process owner, AsyncFd readiness, no detached reader/writer tasks.
use super::{
    ProcessError, ProcessExit, ProcessSpec, PtyInput,
    owner::{self, ProcessOwner},
    pty_io,
};
use std::{fs::File, process::Stdio, sync::Arc};
use tokio::io::unix::AsyncFd;

#[derive(Clone, Copy, Debug)]
pub struct TerminalSize {
    pub rows: u16,
    pub columns: u16,
}
impl TerminalSize {
    pub(super) fn validate(self) -> Result<(), ProcessError> {
        if self.rows == 0 || self.columns == 0 || self.rows > 1000 || self.columns > 1000 {
            return Err(ProcessError::Invalid);
        }
        Ok(())
    }
    pub(super) fn native(self) -> nix::libc::winsize {
        nix::libc::winsize {
            ws_row: self.rows,
            ws_col: self.columns,
            ws_xpixel: 0,
            ws_ypixel: 0,
        }
    }
}
pub struct PtyProcess {
    owner: ProcessOwner,
    master: Arc<AsyncFd<File>>,
    eof: bool,
}
impl PtyProcess {
    /// Native-only; host must authorize and sandbox the specification first.
    /// Must be called within a Tokio runtime with the I/O driver enabled.
    pub fn spawn(spec: ProcessSpec, size: TerminalSize) -> Result<(Self, PtyInput), ProcessError> {
        size.validate()?;
        let mut command = owner::command(spec)?;
        let (master, slave) = pty_io::open(size)?;
        command
            .stdin(Stdio::from(slave.try_clone()?))
            .stdout(Stdio::from(slave.try_clone()?))
            .stderr(Stdio::from(slave.try_clone()?));
        // SAFETY: after the shared hook establishes a session and isolates FDs,
        // only async-signal-safe signal/ioctl syscalls run. Stdio has been duped
        // to the PTY slave by Command; no borrowed parent resource is touched.
        unsafe {
            command.pre_exec(|| {
                for signal in [
                    nix::libc::SIGCHLD,
                    nix::libc::SIGHUP,
                    nix::libc::SIGINT,
                    nix::libc::SIGQUIT,
                    nix::libc::SIGTERM,
                    nix::libc::SIGALRM,
                ] {
                    if nix::libc::signal(signal, nix::libc::SIG_DFL) == nix::libc::SIG_ERR {
                        return Err(std::io::Error::last_os_error());
                    }
                }
                let mut empty = std::mem::zeroed();
                if nix::libc::sigemptyset(&mut empty) != 0
                    || nix::libc::sigprocmask(nix::libc::SIG_SETMASK, &empty, std::ptr::null_mut())
                        != 0
                    || nix::libc::ioctl(0, nix::libc::TIOCSCTTY as _, 0) < 0
                {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let child = command.spawn()?;
        drop(command); // release the parent's cloned slave FDs before reading EOF
        drop(slave);
        let input = PtyInput(Some(master.clone()));
        Ok((
            Self {
                owner: ProcessOwner::new(child)?,
                master,
                eof: false,
            },
            input,
        ))
    }
    pub fn resize(&self, size: TerminalSize) -> Result<(), ProcessError> {
        pty_io::resize(&self.master, size)
    }
    /// Signals the command's group. For normal terminal Ctrl-C semantics on a
    /// foreground job, write the VINTR byte through PtyInput instead.
    pub fn interrupt(&self) -> Result<(), ProcessError> {
        self.owner.interrupt()
    }
    pub fn terminate(&mut self) -> Result<(), ProcessError> {
        self.owner.terminate()
    }
    pub fn exit(&self) -> Option<ProcessExit> {
        self.owner.exit()
    }
    /// Combined terminal output; stdout/stderr cannot be separated in a PTY.
    /// None requires terminal EOF, root reap and settled group cleanup.
    /// Cancel-safe, max 8 KiB.
    pub async fn next_output(&mut self) -> Result<Option<Vec<u8>>, ProcessError> {
        let mut bytes = [0; 8192];
        loop {
            if self.eof && self.owner.is_settled() {
                return Ok(None);
            }
            tokio::select! {
                result = self.owner.wait(), if !self.owner.is_settled() => { result?; }
                result = pty_io::read(&self.master, &mut bytes), if !self.eof => {
                    let count = result?;
                    if count == 0 { self.eof = true; }
                    else { return Ok(Some(bytes[..count].to_vec())); }
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "pty/tests.rs"]
mod tests;
