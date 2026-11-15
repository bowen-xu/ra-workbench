//! Original bounded process and process-group cancellation owner.
use std::{
    io::Read,
    process::{Child, Command, ExitStatus, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use crate::agent::writing_error::WritingError;

const MAX_STREAM_BYTES: usize = 2 * 1024 * 1024;
const NO_OUTPUT_TIMEOUT: Duration = Duration::from_secs(30);

pub(crate) use research_workbench_native_sdk::writing_compile::ProcessState;

pub(crate) struct ProcessOutcome {
    pub(crate) state: ProcessState,
    pub(crate) status: ExitStatus,
    pub(crate) stdout: String,
    pub(crate) stderr: String,
}

pub(crate) fn run(
    command: &mut Command,
    cancellation: Arc<AtomicBool>,
    timeout: Duration,
) -> Result<ProcessOutcome, WritingError> {
    run_with_timeouts(command, cancellation, timeout, NO_OUTPUT_TIMEOUT)
}

fn run_with_timeouts(
    command: &mut Command,
    cancellation: Arc<AtomicBool>,
    timeout: Duration,
    no_output_timeout: Duration,
) -> Result<ProcessOutcome, WritingError> {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command
        .spawn()
        .map_err(|error| WritingError::Process(error.to_string()))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| WritingError::Process("无法读取编译输出".to_owned()))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| WritingError::Process("无法读取编译错误输出".to_owned()))?;
    let activity = Arc::new(AtomicU64::new(0));
    let stdout_activity = Arc::clone(&activity);
    let stderr_activity = Arc::clone(&activity);
    let stdout_reader = thread::spawn(move || read_limited(stdout, stdout_activity));
    let stderr_reader = thread::spawn(move || read_limited(stderr, stderr_activity));
    let deadline = Instant::now() + timeout;
    let mut last_output = Instant::now();
    let mut activity_count = 0;

    let (state, status) = loop {
        let next_activity_count = activity.load(Ordering::Acquire);
        if next_activity_count != activity_count {
            activity_count = next_activity_count;
            last_output = Instant::now();
        }
        if cancellation.load(Ordering::Acquire) {
            terminate_process_tree(&mut child);
            break (ProcessState::Cancelled, child.wait()?);
        }
        if Instant::now() >= deadline || last_output.elapsed() >= no_output_timeout {
            terminate_process_tree(&mut child);
            break (ProcessState::TimedOut, child.wait()?);
        }
        if let Some(status) = child.try_wait()? {
            break (ProcessState::Completed, status);
        }
        thread::sleep(Duration::from_millis(40));
    };

    let stdout = stdout_reader
        .join()
        .map_err(|_| WritingError::Process("编译 stdout reader 异常退出".to_owned()))??;
    let stderr = stderr_reader
        .join()
        .map_err(|_| WritingError::Process("编译 stderr reader 异常退出".to_owned()))??;
    Ok(ProcessOutcome {
        state,
        status,
        stdout,
        stderr,
    })
}

fn read_limited(mut reader: impl Read, activity: Arc<AtomicU64>) -> Result<String, WritingError> {
    let mut kept = Vec::new();
    let mut buffer = [0_u8; 16 * 1024];
    let mut truncated = false;
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        activity.fetch_add(1, Ordering::Release);
        let remaining = MAX_STREAM_BYTES.saturating_sub(kept.len());
        kept.extend_from_slice(&buffer[..count.min(remaining)]);
        truncated |= count > remaining;
    }
    if truncated {
        kept.extend_from_slice(b"\n[output truncated at 2 MiB]\n");
    }
    Ok(String::from_utf8_lossy(&kept).into_owned())
}

#[cfg(unix)]
fn terminate_process_tree(child: &mut Child) {
    use nix::{
        sys::signal::{Signal, killpg},
        unistd::Pid,
    };

    let process_group = Pid::from_raw(child.id() as i32);
    let _ = killpg(process_group, Signal::SIGTERM);
    let deadline = Instant::now() + Duration::from_millis(500);
    while Instant::now() < deadline {
        if child.try_wait().ok().flatten().is_some() {
            return;
        }
        thread::sleep(Duration::from_millis(20));
    }
    let _ = killpg(process_group, Signal::SIGKILL);
}

#[cfg(not(unix))]
fn terminate_process_tree(child: &mut Child) {
    let _ = child.kill();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_limit_marks_only_excess_and_drains_the_reader() {
        for size in [MAX_STREAM_BYTES - 1, MAX_STREAM_BYTES, MAX_STREAM_BYTES + 1] {
            let bytes = vec![b'x'; size];
            let mut reader = std::io::Cursor::new(bytes);
            let output = read_limited(&mut reader, Arc::new(AtomicU64::new(0)))
                .expect("read bounded output");
            let expected = "x".repeat(size.min(MAX_STREAM_BYTES));
            if size > MAX_STREAM_BYTES {
                assert_eq!(output, format!("{expected}\n[output truncated at 2 MiB]\n"));
            } else {
                assert_eq!(output, expected);
            }
            assert_eq!(reader.position(), size as u64);
        }
    }

    #[test]
    fn captures_completed_process_output() {
        let mut command = Command::new("/bin/sh");
        command.arg("-c").arg("printf 'compile output'");
        let outcome = run(
            &mut command,
            Arc::new(AtomicBool::new(false)),
            Duration::from_secs(2),
        )
        .expect("run process");
        assert_eq!(outcome.state, ProcessState::Completed);
        assert!(outcome.status.success());
        assert_eq!(outcome.stdout, "compile output");
    }

    #[cfg(unix)]
    #[test]
    fn stops_processes_after_the_total_timeout() {
        let mut command = Command::new("/bin/sleep");
        command.arg("30");
        let outcome = run_with_timeouts(
            &mut command,
            Arc::new(AtomicBool::new(false)),
            Duration::from_millis(80),
            Duration::from_secs(2),
        )
        .expect("time out process");
        assert_eq!(outcome.state, ProcessState::TimedOut);
    }

    #[cfg(unix)]
    #[test]
    fn stops_processes_that_stop_producing_output() {
        let mut command = Command::new("/bin/sh");
        command.arg("-c").arg("printf started; exec /bin/sleep 30");
        let outcome = run_with_timeouts(
            &mut command,
            Arc::new(AtomicBool::new(false)),
            Duration::from_secs(2),
            Duration::from_millis(80),
        )
        .expect("time out silent process");
        assert_eq!(outcome.state, ProcessState::TimedOut);
        assert_eq!(outcome.stdout, "started");
    }

    #[cfg(unix)]
    #[test]
    fn truncates_process_output_and_drains_both_pipes() {
        // A fixed payload tests the cap without assuming the child can produce
        // more than 2 MiB inside a short timeout on a loaded test machine.
        let mut command = Command::new("/bin/sh");
        command.arg("-c").arg(
            "dd if=/dev/zero bs=65536 count=33 2>/dev/null; \
             dd if=/dev/zero bs=65536 count=33 >&2 2>/dev/null",
        );
        let outcome = run_with_timeouts(
            &mut command,
            Arc::new(AtomicBool::new(false)),
            Duration::from_secs(30),
            Duration::from_secs(30),
        )
        .expect("run noisy process");
        assert_eq!(outcome.state, ProcessState::Completed);
        assert!(outcome.status.success());
        for output in [&outcome.stdout, &outcome.stderr] {
            assert_eq!(
                output.len(),
                MAX_STREAM_BYTES + "\n[output truncated at 2 MiB]\n".len()
            );
            assert!(output.ends_with("\n[output truncated at 2 MiB]\n"));
        }
    }

    #[cfg(unix)]
    #[test]
    fn honours_cancellation_before_waiting_for_process() {
        let mut command = Command::new("/bin/sh");
        command.arg("-c").arg("exec /bin/sleep 30");
        let cancellation = Arc::new(AtomicBool::new(true));
        let outcome =
            run(&mut command, cancellation, Duration::from_secs(2)).expect("cancel process");
        assert_eq!(outcome.state, ProcessState::Cancelled);
    }

    #[cfg(unix)]
    #[test]
    fn cancellation_terminates_descendant_processes() {
        let root = std::env::temp_dir().join(format!("writing-process-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).expect("create process fixture");
        let marker = root.join("descendant-finished");
        let mut command = Command::new("/bin/sh");
        command
            .arg("-c")
            .arg("(sleep 0.4; printf leaked > \"$1\") & wait")
            .arg("writing-process-test")
            .arg(&marker);
        let cancellation = Arc::new(AtomicBool::new(false));
        let cancellation_trigger = Arc::clone(&cancellation);
        let trigger = thread::spawn(move || {
            thread::sleep(Duration::from_millis(80));
            cancellation_trigger.store(true, Ordering::Release);
        });

        let outcome =
            run(&mut command, cancellation, Duration::from_secs(2)).expect("cancel process tree");
        trigger.join().expect("join cancellation trigger");
        thread::sleep(Duration::from_millis(500));

        assert_eq!(outcome.state, ProcessState::Cancelled);
        assert!(
            !marker.exists(),
            "descendant survived process-group cancellation"
        );
        std::fs::remove_dir_all(root).expect("remove process fixture");
    }
}
