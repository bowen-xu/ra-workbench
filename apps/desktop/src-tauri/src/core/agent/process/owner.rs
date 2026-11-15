//! Shared child/group ownership extracted from pipe.rs. Adapted from Codex
//! utils/pty at e363b08c9175ac1cbe5893615dd2cb9ddf95043b. Copyright 2025 OpenAI.
//! Apache-2.0; see packages/agent-core/third-party/codex/{LICENSE,NOTICE}.
use super::{
    ProcessError, ProcessExit, ProcessIoOperation, ProcessSpec, descriptors::DescriptorPlan,
};
use nix::{
    errno::Errno,
    sys::signal::{Signal, killpg},
    unistd::Pid,
};
use std::{io, os::unix::process::ExitStatusExt};
use tokio::process::{Child, Command};

struct ProcessGroup(Option<Pid>);
impl ProcessGroup {
    fn signal(&self, signal: Signal) -> io::Result<()> {
        let Some(pid) = self.0 else {
            return Ok(());
        };
        group_signal_result(killpg(pid, signal))
    }
    fn kill(&mut self) -> io::Result<()> {
        self.signal(Signal::SIGKILL)?;
        self.0.take();
        Ok(())
    }
}
impl Drop for ProcessGroup {
    fn drop(&mut self) {
        let _ = self.kill();
    }
}

pub(super) struct ProcessOwner {
    group: ProcessGroup,
    child: Child,
    exit: Option<ProcessExit>,
}
impl ProcessOwner {
    pub fn new(child: Child) -> Result<Self, ProcessError> {
        let pid = child
            .id()
            .and_then(|id| i32::try_from(id).ok())
            .filter(|id| *id > 1)
            .ok_or(ProcessError::Invalid)?;
        Ok(Self {
            group: ProcessGroup(Some(Pid::from_raw(pid))),
            child,
            exit: None,
        })
    }
    pub fn exit(&self) -> Option<ProcessExit> {
        self.exit
    }
    #[cfg(test)]
    pub(super) fn process_id(&self) -> u32 {
        self.child.id().expect("owned child has not been reaped")
    }
    pub(super) fn is_settled(&self) -> bool {
        self.exit.is_some() && self.group.0.is_none()
    }
    pub fn interrupt(&self) -> Result<(), ProcessError> {
        self.group
            .signal(Signal::SIGINT)
            .map_err(|error| ProcessError::operation_io(ProcessIoOperation::GroupInterrupt, error))
    }
    pub fn terminate(&mut self) -> Result<(), ProcessError> {
        if let Err(error) = self.group.kill() {
            self.group_termination_failed(error)?;
        }
        if self.exit.is_none() {
            self.child.start_kill().map_err(|error| {
                ProcessError::operation_io(ProcessIoOperation::ChildStartKill, error)
            })?;
        }
        Ok(())
    }

    fn group_termination_failed(&mut self, initial: io::Error) -> Result<(), ProcessError> {
        #[cfg(target_os = "macos")]
        if self.exit.is_none() && initial.raw_os_error() == Some(nix::libc::EPERM) {
            let status = match self.child.try_wait() {
                Ok(Some(status)) => status,
                Ok(None) => {
                    #[cfg(all(test, target_os = "macos"))]
                    eprintln!(
                        "C07_TERMINATION_FAILURE cached_exit=false group_pending={} observed_try_wait=OkNone",
                        self.group.0.is_some()
                    );
                    return Err(ProcessError::operation_io(
                        ProcessIoOperation::GroupTerminate,
                        initial,
                    ));
                }
                Err(error) => {
                    return Err(ProcessError::termination_recovery(
                        initial,
                        ProcessIoOperation::ChildWait,
                        error,
                    ));
                }
            };
            self.exit = Some(ProcessExit {
                code: status.code(),
                signal: status.signal(),
            });
            // Darwin can reject a group containing only unreaped zombies.
            // Retry once after reaping our actual child, never on EPERM alone.
            // The stored group still has the original non-containment boundary.
            return self.group.kill().map_err(|error| {
                ProcessError::termination_recovery(
                    initial,
                    ProcessIoOperation::GroupCleanupAfterWait,
                    error,
                )
            });
        }
        #[cfg(all(test, target_os = "macos"))]
        if initial.raw_os_error() == Some(nix::libc::EPERM) {
            eprintln!(
                "C07_TERMINATION_FAILURE cached_exit={} group_pending={} observed_try_wait=NotAttempted",
                self.exit.is_some(),
                self.group.0.is_some()
            );
        }
        Err(ProcessError::operation_io(
            ProcessIoOperation::GroupTerminate,
            initial,
        ))
    }

    pub async fn wait(&mut self) -> Result<(), ProcessError> {
        if self.exit.is_none() {
            let status = self.child.wait().await.map_err(|error| {
                ProcessError::operation_io(ProcessIoOperation::ChildWait, error)
            })?;
            self.exit = Some(ProcessExit {
                code: status.code(),
                signal: status.signal(),
            });
        }
        // A cached root exit is not evidence that group cleanup succeeded.
        // Retain the pending group on failure so a later wait cannot skip it.
        self.group.kill().map_err(|error| {
            ProcessError::operation_io(ProcessIoOperation::GroupCleanupAfterWait, error)
        })?;
        Ok(())
    }
}

fn group_signal_result(result: Result<(), Errno>) -> io::Result<()> {
    match result {
        Ok(()) | Err(Errno::ESRCH) => Ok(()),
        Err(error) => Err(io::Error::from_raw_os_error(error as i32)),
    }
}

#[cfg(all(test, target_os = "macos"))]
#[path = "owner/macos_tests.rs"]
mod macos_tests;
#[cfg(test)]
#[path = "owner/tests.rs"]
mod tests;
impl Drop for ProcessOwner {
    fn drop(&mut self) {
        let _ = self.group.kill();
        if self.exit.is_none() {
            let _ = self.child.start_kill();
        }
    }
}

pub(super) fn command(spec: ProcessSpec) -> Result<Command, ProcessError> {
    spec.validate()?;
    let descriptors = DescriptorPlan::prepare()?;
    let mut command = Command::new(spec.program);
    command
        .args(spec.arguments)
        .current_dir(spec.directory)
        .env_clear()
        .envs(spec.environment)
        .kill_on_drop(true);
    // SAFETY: child-only descriptor syscalls and setsid. No allocation, locks
    // or runtime state are touched after fork. Errors use std's preserved pipe.
    unsafe {
        command.pre_exec(move || {
            descriptors.apply()?;
            nix::unistd::setsid()
                .map(|_| ())
                .map_err(|error| std::io::Error::from_raw_os_error(error as i32))
        });
    }
    Ok(command)
}
