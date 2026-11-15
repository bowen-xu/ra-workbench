//! Native-only control channel. One client, one queued request, no detached tasks.
//! The host must poll finish_controlled and retain its ownership through shutdown.
use super::{ProcessError, ProcessExit, RetainedOutput, TerminalSize};
use tokio::sync::{mpsc, oneshot};

/// Cumulative retained bytes, not a delta and not evidence of completed reaping.
#[derive(Debug)]
pub struct ProcessSnapshot {
    pub exit: Option<ProcessExit>,
    pub stdout: RetainedOutput,
    pub stderr: RetainedOutput,
    pub terminal: Option<RetainedOutput>,
}
pub struct ProcessControl {
    sender: mpsc::Sender<Request>,
}
pub struct ProcessCommands {
    pub(super) receiver: mpsc::Receiver<Request>,
}
pub(super) enum Request {
    Snapshot(oneshot::Sender<ProcessSnapshot>),
    Interrupt(oneshot::Sender<Result<(), ProcessError>>),
    Resize(TerminalSize, oneshot::Sender<Result<(), ProcessError>>),
}
fn closed() -> ProcessError {
    ProcessError::Io(std::io::ErrorKind::BrokenPipe)
}
/// Dropping either endpoint never means the process has been cancelled/reaped.
/// Dropping a request future after enqueue does not retract its side effect.
/// No Clone: a client cannot build an unbounded concurrent snapshot backlog.
pub fn process_control() -> (ProcessControl, ProcessCommands) {
    let (sender, receiver) = mpsc::channel(1);
    (ProcessControl { sender }, ProcessCommands { receiver })
}
impl ProcessControl {
    pub async fn snapshot(&mut self) -> Result<ProcessSnapshot, ProcessError> {
        let (sender, receiver) = oneshot::channel();
        self.sender
            .send(Request::Snapshot(sender))
            .await
            .map_err(|_| closed())?;
        receiver.await.map_err(|_| closed())
    }
    /// Sends SIGINT to the owned command group; use PTY input VINTR for terminal
    /// foreground-job semantics. Success means signal delivery, not process exit.
    pub async fn interrupt(&mut self) -> Result<(), ProcessError> {
        let (sender, receiver) = oneshot::channel();
        self.sender
            .send(Request::Interrupt(sender))
            .await
            .map_err(|_| closed())?;
        receiver.await.map_err(|_| closed())?
    }
    /// Pipe execution returns Unsupported. Terminal sizes retain native validation.
    pub async fn resize(&mut self, size: TerminalSize) -> Result<(), ProcessError> {
        let (sender, receiver) = oneshot::channel();
        self.sender
            .send(Request::Resize(size, sender))
            .await
            .map_err(|_| closed())?;
        receiver.await.map_err(|_| closed())?
    }
}
