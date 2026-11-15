//! Native running session over the migrated process supervisor. No model/IPC
//! authorization is implied; the application registry must own these sessions.
use super::{
    PipeInput, ProcessControl, ProcessError, ProcessLimits, ProcessOutcome, ProcessSnapshot,
    ProcessSpec, PtyInput, SupervisedProcess, TerminalSize, process_control,
};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{sync::oneshot, task::JoinHandle, time::timeout};

pub enum ProcessPoll {
    /// Cumulative bounded output, never a destructive read or an exit assertion.
    Running(ProcessSnapshot),
    Finished(Arc<ProcessOutcome>),
}
enum Input {
    Pipe(PipeInput),
    Terminal(PtyInput),
}
/// Native resource finalization runs on a blocking worker after actual supervision.
/// A dropped retention object must preserve any unresolved persistent intent.
pub(crate) trait ProcessRetention: Send + 'static {
    fn release(self: Box<Self>, outcome: &ProcessOutcome) -> Result<(), ProcessError>;
}
enum Driver {
    Running(JoinHandle<Result<ProcessOutcome, ProcessError>>),
    Finished(Arc<ProcessOutcome>),
    Failed,
}
/// Exactly one supervisor task keeps draining between host calls. Explicit
/// shutdown must be awaited before releasing a workspace or accepting changes.
/// Drop requests cancellation but cannot attest reaping; the runtime must remain
/// alive for bounded cleanup. No public abort handle is exposed.
pub struct ProcessSession {
    driver: Driver,
    input: Input,
    control: ProcessControl,
    cancel: ProcessCancellation,
}
/// Cancellation does not require the session's input/poll mutex.
#[derive(Clone)]
pub(crate) struct ProcessCancellation(Arc<Mutex<Option<oneshot::Sender<()>>>>);
impl ProcessCancellation {
    pub fn cancel(&self) {
        // Only taking a sender occurs under this lock; no user code/invariants
        // can be partially modified if a previous holder panicked.
        if let Some(sender) = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
        {
            let _ = sender.send(());
        }
    }
}
fn stopped() -> ProcessError {
    ProcessError::Io(std::io::ErrorKind::BrokenPipe)
}
impl ProcessSession {
    pub fn spawn(spec: ProcessSpec, limits: ProcessLimits) -> Result<Self, ProcessError> {
        let (process, input) = SupervisedProcess::spawn(spec, limits)?;
        Ok(Self::start(process, Input::Pipe(input), None))
    }
    pub fn spawn_terminal(
        spec: ProcessSpec,
        size: TerminalSize,
        limits: ProcessLimits,
    ) -> Result<Self, ProcessError> {
        let (process, input) = SupervisedProcess::spawn_terminal(spec, size, limits)?;
        Ok(Self::start(process, Input::Terminal(input), None))
    }
    pub(crate) fn spawn_retained(
        spec: ProcessSpec,
        limits: ProcessLimits,
        terminal: Option<TerminalSize>,
        retention: Box<dyn ProcessRetention>,
    ) -> Result<Self, ProcessError> {
        let (process, input) = if let Some(size) = terminal {
            let (process, input) = SupervisedProcess::spawn_terminal(spec, size, limits)?;
            (process, Input::Terminal(input))
        } else {
            let (process, input) = SupervisedProcess::spawn(spec, limits)?;
            (process, Input::Pipe(input))
        };
        Ok(Self::start(process, input, Some(retention)))
    }
    fn start(
        process: SupervisedProcess,
        input: Input,
        retention: Option<Box<dyn ProcessRetention>>,
    ) -> Self {
        let (control, commands) = process_control();
        let (cancel, receiver) = oneshot::channel();
        let driver = tokio::spawn(async move {
            let outcome = process
                .finish_controlled(
                    async {
                        let _ = receiver.await;
                    },
                    commands,
                )
                .await;
            match retention {
                None => Ok(outcome),
                Some(retention) => tokio::task::spawn_blocking(move || {
                    let mut outcome = outcome;
                    outcome.resource_error = retention.release(&outcome).err();
                    outcome
                })
                .await
                .map_err(|_| ProcessError::ResourceRelease),
            }
        });
        Self {
            driver: Driver::Running(driver),
            input,
            control,
            cancel: ProcessCancellation(Arc::new(Mutex::new(Some(cancel)))),
        }
    }
    /// One cancel-safe write, returning the accepted byte count. Callers must
    /// retain partial progress; this method never retries an entire input.
    pub async fn write(&mut self, bytes: &[u8]) -> Result<usize, ProcessError> {
        if !matches!(&self.driver, Driver::Running(task) if !task.is_finished()) {
            return Err(stopped());
        }
        match &mut self.input {
            Input::Pipe(input) => input.write(bytes).await,
            Input::Terminal(input) => input.write(bytes).await,
        }
    }
    /// Pipe: send EOF. PTY: release the writer reference, not terminal EOF.
    pub fn close_input(&mut self) {
        match &mut self.input {
            Input::Pipe(input) => input.close(),
            Input::Terminal(input) => input.close(),
        }
    }
    pub async fn interrupt(&mut self) -> Result<(), ProcessError> {
        self.control.interrupt().await
    }
    pub async fn resize(&mut self, size: TerminalSize) -> Result<(), ProcessError> {
        self.control.resize(size).await
    }
    /// Wait up to the requested yield interval (0..=60 s), then return a current
    /// snapshot. Observation cancellation never cancels/restarts the command.
    /// If completion closes the snapshot channel, join the final outcome rather
    /// than returning a stale-handle error. A snapshot describes its capture time.
    pub async fn poll(&mut self, interval: Duration) -> Result<ProcessPoll, ProcessError> {
        if interval > Duration::from_secs(60) {
            return Err(ProcessError::Invalid);
        }
        if let Ok(result) = timeout(interval, self.finish()).await {
            return result.map(ProcessPoll::Finished);
        }
        let result = match &mut self.driver {
            Driver::Running(task) => tokio::select! {
                result = task => Some(result),
                snapshot = self.control.snapshot() => {
                    if let Ok(snapshot) = snapshot { return Ok(ProcessPoll::Running(snapshot)); }
                    None
                }
            },
            _ => None,
        };
        if let Some(result) = result {
            self.record(result);
        }
        self.finish().await.map(ProcessPoll::Finished)
    }
    fn record(
        &mut self,
        result: Result<Result<ProcessOutcome, ProcessError>, tokio::task::JoinError>,
    ) {
        self.driver = match result {
            Ok(Ok(outcome)) => Driver::Finished(Arc::new(outcome)),
            _ => Driver::Failed,
        };
        self.cancel.cancel();
        self.close_input();
    }
    /// Cancellation-safe join, cached for repeated observations. Failed driver
    /// execution never produces a fabricated exit code or drained=true result.
    pub async fn finish(&mut self) -> Result<Arc<ProcessOutcome>, ProcessError> {
        if let Driver::Running(task) = &mut self.driver {
            let result = task.await;
            self.record(result);
        }
        match &self.driver {
            Driver::Finished(outcome) => Ok(Arc::clone(outcome)),
            _ => Err(ProcessError::Io(std::io::ErrorKind::Other)),
        }
    }
    pub(crate) fn cancellation(&self) -> ProcessCancellation {
        self.cancel.clone()
    }
    pub(crate) fn safely_finished(&self) -> bool {
        matches!(&self.driver, Driver::Finished(result) if result.drained && result.resource_error.is_none())
    }
    pub fn cancel(&mut self) {
        self.cancel.cancel();
    }
    /// Requests termination and waits for the supervisor's bounded drain/reap.
    /// Callers must inspect drained; successful API return alone is not proof.
    pub async fn shutdown(&mut self) -> Result<Arc<ProcessOutcome>, ProcessError> {
        self.cancel();
        self.finish().await
    }
}
impl Drop for ProcessSession {
    fn drop(&mut self) {
        self.cancel();
    }
}

#[cfg(test)]
#[path = "session/tests.rs"]
mod tests;
