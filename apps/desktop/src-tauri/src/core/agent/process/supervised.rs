//! Bounded lifecycle around pipe and PTY transports. The host still owns admission,
//! sandboxing, workspace ownership and the policy selecting these limits.
use super::{
    OutputStream, PipeInput, PipeProcess, ProcessError, ProcessExit, ProcessSpec, PtyInput,
    PtyProcess, RetainedOutput, TerminalSize,
    control::{ProcessCommands, ProcessSnapshot, Request},
    output::HeadTailBuffer,
    transport::{Transport, TransportOutput},
};
use std::{future::Future, time::Duration};
use tokio::time::{Instant, sleep_until};

pub struct ProcessLimits {
    /// Combined stdout/stderr storage budget, split equally to prevent one
    /// stream from hiding the other's diagnostics. Unused capacity is not shared.
    /// A PTY assigns this entire budget to its combined terminal stream.
    pub retained_bytes: usize,
    pub execution_timeout: Duration,
    pub drain_timeout: Duration,
}
impl ProcessLimits {
    fn validate(&self) -> Result<(), ProcessError> {
        if self.retained_bytes > 16 * 1024 * 1024
            || self.execution_timeout.is_zero()
            || self.execution_timeout > Duration::from_secs(24 * 60 * 60)
            || self.drain_timeout.is_zero()
            || self.drain_timeout > Duration::from_secs(60)
        {
            return Err(ProcessError::Invalid);
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    Completed,
    Cancelled,
    Deadline,
    IoFailure,
}
pub struct ProcessOutcome {
    pub reason: StopReason,
    pub exit: Option<ProcessExit>,
    /// True only after wait/reap and EOF on every output; a kill request is not proof.
    pub drained: bool,
    pub stdout: RetainedOutput,
    pub stderr: RetainedOutput,
    /// Some only for PTY execution; stdout/stderr are then empty. Never split a
    /// terminal's combined stream into invented separate stdout/stderr streams.
    pub terminal: Option<RetainedOutput>,
    pub error: Option<ProcessError>,
    /// Separate from process I/O/exit; a host must not accept changes on failure.
    pub resource_error: Option<ProcessError>,
}
pub struct SupervisedProcess {
    retention: Option<Box<dyn super::ProcessRetention>>,
    process: Transport,
    deadline: Instant,
    drain_timeout: Duration,
    stdout: HeadTailBuffer,
    stderr: HeadTailBuffer,
    terminal: Option<HeadTailBuffer>,
}
impl SupervisedProcess {
    /// Native-only. A validated budget does not grant permission to execute.
    pub fn spawn(
        spec: ProcessSpec,
        limits: ProcessLimits,
    ) -> Result<(Self, PipeInput), ProcessError> {
        limits.validate()?;
        let deadline = Instant::now() + limits.execution_timeout;
        let (process, input) = PipeProcess::spawn(spec)?;
        Ok((
            Self {
                retention: None,
                process: Transport::Pipe(process),
                deadline,
                drain_timeout: limits.drain_timeout,
                stdout: HeadTailBuffer::new(limits.retained_bytes / 2),
                stderr: HeadTailBuffer::new(limits.retained_bytes - limits.retained_bytes / 2),
                terminal: None,
            },
            input,
        ))
    }
    /// Same lifetime and limits as a pipe, with terminal input/EOF semantics.
    pub fn spawn_terminal(
        spec: ProcessSpec,
        size: TerminalSize,
        limits: ProcessLimits,
    ) -> Result<(Self, PtyInput), ProcessError> {
        limits.validate()?;
        let deadline = Instant::now() + limits.execution_timeout;
        let (process, input) = PtyProcess::spawn(spec, size)?;
        Ok((
            Self {
                retention: None,
                process: Transport::Terminal(process),
                deadline,
                drain_timeout: limits.drain_timeout,
                stdout: HeadTailBuffer::new(0),
                stderr: HeadTailBuffer::new(0),
                terminal: Some(HeadTailBuffer::new(limits.retained_bytes)),
            },
            input,
        ))
    }
    /// Consume ownership through completion. Cancellation of this future drops
    /// the process owner and requests termination; callers needing explicit reap
    /// evidence must resolve `cancel` instead of aborting this future.
    /// Stdin is independent: the caller must write/close it concurrently.
    pub async fn finish(self, cancel: impl Future<Output = ()>) -> ProcessOutcome {
        self.finish_retained(cancel, None).await
    }
    /// Same supervision loop with bounded native control requests. Poll this
    /// future concurrently with the client. Closing the client does not cancel.
    pub async fn finish_controlled(
        self,
        cancel: impl Future<Output = ()>,
        commands: ProcessCommands,
    ) -> ProcessOutcome {
        self.finish_retained(cancel, Some(commands)).await
    }
    pub(crate) fn retain(mut self, retention: Box<dyn super::ProcessRetention>) -> Self {
        self.retention = Some(retention);
        self
    }
    async fn finish_retained(
        mut self,
        cancel: impl Future<Output = ()>,
        commands: Option<ProcessCommands>,
    ) -> ProcessOutcome {
        let retention = self.retention.take();
        let mut outcome = self.run(cancel, commands).await;
        if let Some(retention) = retention {
            outcome = tokio::task::spawn_blocking(move || {
                outcome.resource_error = retention.release(&outcome).err();
                outcome
            })
            .await
            .unwrap_or_else(|_| ProcessOutcome {
                reason: StopReason::IoFailure,
                exit: None,
                drained: false,
                stdout: HeadTailBuffer::new(0).finish(),
                stderr: HeadTailBuffer::new(0).finish(),
                terminal: None,
                error: None,
                resource_error: Some(ProcessError::ResourceRelease),
            });
        }
        outcome
    }
    async fn run(
        mut self,
        cancel: impl Future<Output = ()>,
        mut commands: Option<ProcessCommands>,
    ) -> ProcessOutcome {
        tokio::pin!(cancel);
        let mut reason = StopReason::Completed;
        let mut stopping = false;
        let mut error = None;
        let mut drained = false;
        let mut deadline = self.deadline;
        loop {
            // Tokio timers have a coarser tick than Instant. Check elapsed time
            // before polling ready output so a short drain budget is respected.
            if Instant::now() >= deadline {
                if stopping {
                    break;
                }
                reason = StopReason::Deadline;
                stopping = true;
                deadline = Instant::now() + self.drain_timeout;
                if let Err(failure) = self.process.terminate() {
                    error = Some(failure);
                }
                continue;
            }
            tokio::select! {
                _ = &mut cancel, if !stopping => {
                    reason = StopReason::Cancelled;
                    stopping = true;
                    deadline = Instant::now() + self.drain_timeout;
                    if let Err(failure) = self.process.terminate() { error = Some(failure); }
                }
                _ = sleep_until(deadline) => {
                    if stopping { break; }
                    reason = StopReason::Deadline;
                    stopping = true;
                    deadline = Instant::now() + self.drain_timeout;
                    if let Err(failure) = self.process.terminate() { error = Some(failure); }
                }
                request = async {
                    match &mut commands {
                        Some(commands) => commands.receiver.recv().await,
                        None => std::future::pending().await,
                    }
                }, if !stopping => {
                    match request {
                        Some(Request::Snapshot(reply)) => {
                            if !reply.is_closed() {
                                let _ = reply.send(ProcessSnapshot {
                                    exit: self.process.exit(),
                                    stdout: self.stdout.snapshot(),
                                    stderr: self.stderr.snapshot(),
                                    terminal: self.terminal.as_ref().map(HeadTailBuffer::snapshot),
                                });
                            }
                        }
                        Some(Request::Interrupt(reply)) => { let _ = reply.send(self.process.interrupt()); }
                        Some(Request::Resize(size, reply)) => { let _ = reply.send(self.process.resize(size)); }
                        None => commands = None,
                    }
                }
                output = self.process.next_output() => {
                    match output {
                        Ok(Some(output)) => match output {
                            TransportOutput::Pipe(chunk) => match chunk.stream {
                                OutputStream::Stdout => self.stdout.push(&chunk.bytes),
                                OutputStream::Stderr => self.stderr.push(&chunk.bytes),
                            },
                            TransportOutput::Terminal(bytes) => {
                                // Constructors pair a terminal transport with
                                // this buffer; a mismatch is an internal error.
                                let Some(terminal) = &mut self.terminal else {
                                    reason = StopReason::IoFailure;
                                    error = Some(ProcessError::Invalid);
                                    let _ = self.process.terminate();
                                    break;
                                };
                                terminal.push(&bytes);
                            }
                        },
                        Ok(None) => { drained = true; break; }
                        Err(failure) => {
                            if !stopping { reason = StopReason::IoFailure; }
                            error = Some(failure);
                            let _ = self.process.terminate();
                            break;
                        }
                    }
                }
            }
        }
        ProcessOutcome {
            reason,
            exit: self.process.exit(),
            drained,
            stdout: self.stdout.finish(),
            stderr: self.stderr.finish(),
            terminal: self.terminal.map(HeadTailBuffer::finish),
            error,
            resource_error: None,
        }
    }
}

#[cfg(test)]
#[path = "supervised/terminal_tests.rs"]
mod terminal_tests;
#[cfg(test)]
#[path = "supervised/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "supervised/control_tests.rs"]
mod control_tests;
