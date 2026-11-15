//! Unix pipe transport adapted from the Codex sources named in process.rs.
//! Process group cleanup is lifecycle management, not process-tree containment:
//! a sandbox must separately prevent an untrusted descendant escaping the group.
use super::owner::{self, ProcessOwner};
use super::*;
#[cfg(test)]
use nix::unistd::Pid;
use std::process::Stdio;
use tokio::{
    io::AsyncReadExt,
    process::{ChildStderr, ChildStdout},
};

/// Owns the child and both output pipes. Stdin is returned separately so its
/// writer can run concurrently with output draining and close it to send EOF.
/// No helper task outlives this handle. Drop kills the group; Tokio owns the
/// child's eventual reap on drop. Await output EOF for explicit reap evidence.
pub struct PipeProcess {
    owner: ProcessOwner,
    stdout: ChildStdout,
    stderr: ChildStderr,
    stdout_done: bool,
    stderr_done: bool,
}
impl PipeProcess {
    pub fn spawn(spec: ProcessSpec) -> Result<(Self, PipeInput), ProcessError> {
        let mut command = owner::command(spec)?;
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn()?;
        let stdin = child.stdin.take().ok_or(ProcessError::Invalid)?;
        let stdout = child.stdout.take().ok_or(ProcessError::Invalid)?;
        let stderr = child.stderr.take().ok_or(ProcessError::Invalid)?;
        Ok((
            Self {
                owner: ProcessOwner::new(child)?,
                stdout,
                stderr,
                stdout_done: false,
                stderr_done: false,
            },
            PipeInput(Some(stdin)),
        ))
    }
    pub fn exit(&self) -> Option<ProcessExit> {
        self.owner.exit()
    }
    #[cfg(test)]
    pub(crate) fn process_id(&self) -> u32 {
        self.owner.process_id()
    }

    pub fn interrupt(&self) -> Result<(), ProcessError> {
        self.owner.interrupt()
    }
    /// Request immediate termination. Continue reading to collect final output
    /// and observe reaping; requesting a signal is not itself an exit result.
    pub fn terminate(&mut self) -> Result<(), ProcessError> {
        self.owner.terminate()
    }
    /// A cancelled read future loses no consumed chunk. Each result is at most
    /// 8 KiB and preserves arbitrary bytes and stream identity. None means both
    /// streams reached EOF, the root was reaped and group cleanup is settled.
    /// Hosts must apply their execution/drain deadlines and total output budget.
    pub async fn next_output(&mut self) -> Result<Option<OutputChunk>, ProcessError> {
        let mut stdout_buffer = [0; 8192];
        let mut stderr_buffer = [0; 8192];
        loop {
            if self.stdout_done && self.stderr_done && self.owner.is_settled() {
                return Ok(None);
            }
            tokio::select! {
                status = self.owner.wait(), if !self.owner.is_settled() => { status?; }
                count = self.stdout.read(&mut stdout_buffer), if !self.stdout_done => {
                    let count = count?;
                    if count == 0 { self.stdout_done = true; }
                    else { return Ok(Some(OutputChunk {stream:OutputStream::Stdout, bytes:stdout_buffer[..count].to_vec()})); }
                }
                count = self.stderr.read(&mut stderr_buffer), if !self.stderr_done => {
                    let count = count?;
                    if count == 0 { self.stderr_done = true; }
                    else { return Ok(Some(OutputChunk {stream:OutputStream::Stderr, bytes:stderr_buffer[..count].to_vec()})); }
                }
            }
        }
    }
}
#[cfg(test)]
#[path = "pipe/descriptor_tests.rs"]
mod descriptor_tests;
#[cfg(all(test, target_os = "macos"))]
#[path = "pipe/pending_group_tests.rs"]
mod pending_group_tests;
#[cfg(test)]
#[path = "pipe/tests.rs"]
mod tests;
