//! Native process transport, not an authorization or sandbox boundary. Hosts must
//! prepare an authorized, sandboxed command before exposing this to agent tools.
//! Adapted from OpenAI Codex utils/pty/src/{pipe,process,process_group}.rs at
//! e363b08c9175ac1cbe5893615dd2cb9ddf95043b. Copyright 2025 OpenAI. Apache-2.0;
//! see packages/agent-core/third-party/codex/{LICENSE,NOTICE}.
//! Changes: caller-driven cancel-safe reads, explicit exit signal, no detached
//! reader tasks, broadcast loss, inherited environment, or non-Unix fallback.
use std::{collections::BTreeMap, ffi::OsString, path::PathBuf};

/// Fixed owned-process operations; never supplied by a command or plugin.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcessIoOperation {
    GroupInterrupt,
    GroupTerminate,
    ChildStartKill,
    ChildWait,
    GroupCleanupAfterWait,
}

impl std::fmt::Display for ProcessIoOperation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::GroupInterrupt => "group_interrupt",
            Self::GroupTerminate => "group_terminate",
            Self::ChildStartKill => "child_start_kill",
            Self::ChildWait => "child_wait",
            Self::GroupCleanupAfterWait => "group_cleanup_after_wait",
        })
    }
}

/// Fixed safe metadata for the two attempts in macOS termination recovery.
#[cfg(target_os = "macos")]
#[derive(Debug, thiserror::Error)]
#[error("process I/O failed (operation={operation}, kind={kind:?}, raw_os_error={raw_os_error:?})")]
pub struct ProcessIoFailure {
    operation: ProcessIoOperation,
    kind: std::io::ErrorKind,
    raw_os_error: Option<i32>,
}

#[cfg(target_os = "macos")]
impl ProcessIoFailure {
    fn new(operation: ProcessIoOperation, error: std::io::Error) -> Self {
        Self {
            operation,
            kind: error.kind(),
            raw_os_error: error.raw_os_error(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ProcessError {
    #[error("invalid process specification")]
    Invalid,
    #[error("process resources could not be released safely")]
    ResourceRelease,
    #[error("process backend is not supported on this platform")]
    Unsupported,
    #[error("process I/O failed ({0:?})")]
    Io(std::io::ErrorKind),
    #[error(
        "process I/O failed (operation={operation}, kind={kind:?}, raw_os_error={raw_os_error:?})"
    )]
    OperationIo {
        operation: ProcessIoOperation,
        kind: std::io::ErrorKind,
        raw_os_error: Option<i32>,
    },
    #[cfg(target_os = "macos")]
    #[error("{initial}; termination recovery failed ({recovery})")]
    TerminationRecovery {
        initial: ProcessIoFailure,
        recovery: ProcessIoFailure,
    },
}

impl ProcessError {
    #[cfg(unix)]
    fn operation_io(operation: ProcessIoOperation, error: std::io::Error) -> Self {
        // Store only fixed operation metadata and native numeric diagnostics.
        // The original io::Error text may contain paths or command arguments.
        Self::OperationIo {
            operation,
            kind: error.kind(),
            raw_os_error: error.raw_os_error(),
        }
    }

    #[cfg(target_os = "macos")]
    fn termination_recovery(
        initial: std::io::Error,
        recovery_operation: ProcessIoOperation,
        recovery: std::io::Error,
    ) -> Self {
        Self::TerminationRecovery {
            initial: ProcessIoFailure::new(ProcessIoOperation::GroupTerminate, initial),
            recovery: ProcessIoFailure::new(recovery_operation, recovery),
        }
    }
}
impl From<std::io::Error> for ProcessError {
    fn from(error: std::io::Error) -> Self {
        // OS errors can include command paths or argument contents. Keep the
        // native transport error typed and free of the original command text.
        Self::Io(error.kind())
    }
}

#[cfg(all(test, unix))]
#[path = "process/error_tests.rs"]
mod error_tests;

/// Native-only input. Never deserialize this directly from model/IPC arguments.
/// Environment values are explicitly supplied by the host, never inherited.
pub struct ProcessSpec {
    pub program: PathBuf,
    pub arguments: Vec<OsString>,
    pub directory: PathBuf,
    pub environment: BTreeMap<OsString, OsString>,
}
impl ProcessSpec {
    fn validate(&self) -> Result<(), ProcessError> {
        if !self.program.is_absolute() || !self.directory.is_absolute() {
            return Err(ProcessError::Invalid);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputStream {
    Stdout,
    Stderr,
}
#[derive(Debug, PartialEq, Eq)]
pub struct OutputChunk {
    pub stream: OutputStream,
    pub bytes: Vec<u8>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcessExit {
    pub code: Option<i32>,
    pub signal: Option<i32>,
}

/// Single-owner pipe input. `close` sends EOF by releasing the OS handle;
/// AsyncWrite shutdown alone does not close a child pipe on every platform.
pub struct PipeInput(Option<tokio::process::ChildStdin>);
impl PipeInput {
    /// One cancel-safe write; callers retain the returned count rather than
    /// replaying an entire partially written input when resuming.
    pub async fn write(&mut self, bytes: &[u8]) -> Result<usize, ProcessError> {
        use tokio::io::AsyncWriteExt;
        self.0
            .as_mut()
            .ok_or(ProcessError::Io(std::io::ErrorKind::BrokenPipe))?
            .write(bytes)
            .await
            .map_err(Into::into)
    }
    pub fn close(&mut self) {
        self.0.take();
    }
}

#[cfg(unix)]
#[path = "process/descriptors.rs"]
mod descriptors;
#[path = "process/output.rs"]
mod output;
#[cfg(unix)]
#[path = "process/owner.rs"]
mod owner;
#[cfg(unix)]
#[path = "process/pty.rs"]
mod pty;
#[cfg(unix)]
#[path = "process/pty_io.rs"]
mod pty_io;
#[cfg(unix)]
pub use pty::{PtyProcess, TerminalSize};
#[cfg(unix)]
pub use pty_io::PtyInput;
#[cfg(unix)]
#[path = "process/pipe.rs"]
mod pipe;
pub use output::RetainedOutput;
#[cfg(unix)]
#[path = "process/supervised.rs"]
mod supervised;
#[cfg(unix)]
#[path = "process/transport.rs"]
mod transport;
#[cfg(unix)]
pub use pipe::PipeProcess;
#[cfg(unix)]
pub use supervised::{ProcessLimits, ProcessOutcome, StopReason, SupervisedProcess};
#[cfg(target_os = "macos")]
#[path = "process/seatbelt.rs"]
mod seatbelt;
#[cfg(target_os = "macos")]
pub use seatbelt::{OfflineSandboxPaths, prepare_offline_sandbox};

#[cfg(not(unix))]
pub struct PipeProcess;
#[cfg(not(unix))]
impl PipeProcess {
    pub fn spawn(spec: ProcessSpec) -> Result<(Self, PipeInput), ProcessError> {
        spec.validate()?;
        Err(ProcessError::Unsupported)
    }
}

#[cfg(unix)]
#[path = "process/control.rs"]
mod control;
#[cfg(unix)]
pub use control::{ProcessCommands, ProcessControl, ProcessSnapshot, process_control};

#[cfg(unix)]
#[path = "process/session.rs"]
mod session;
#[cfg(unix)]
pub use session::{ProcessPoll, ProcessSession};

#[cfg(unix)]
pub(crate) use session::ProcessRetention;

#[cfg(unix)]
#[path = "process/registry.rs"]
mod registry;
#[cfg(unix)]
pub use registry::{ProcessRegistry, ProcessRegistryError};
