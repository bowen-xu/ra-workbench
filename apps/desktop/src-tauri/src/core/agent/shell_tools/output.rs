use super::*;
use crate::agent::process::{RetainedOutput, StopReason};
use serde_json::{Value, json};

fn stream(output: &RetainedOutput) -> Value {
    if output.omitted_bytes == 0 {
        let bytes = [output.head.as_slice(), output.tail.as_slice()].concat();
        return json!({"head":String::from_utf8_lossy(&bytes),"tail":"","omittedBytes":0,
            "lossyUtf8":std::str::from_utf8(&bytes).is_err()});
    }
    // Byte truncation may split UTF-8. Explicitly report replacement, and keep
    // head/tail separate so the omitted middle is never presented as contiguous.
    json!({"head":String::from_utf8_lossy(&output.head),
        "tail":String::from_utf8_lossy(&output.tail), "omittedBytes":output.omitted_bytes,
        "lossyUtf8": std::str::from_utf8(&output.head).is_err() || std::str::from_utf8(&output.tail).is_err()})
}

pub(super) fn render(id: Uuid, poll: &ProcessPoll) -> Result<ToolOutput, RunError> {
    let (state, exit, stdout, stderr, terminal, error) = match poll {
        ProcessPoll::Running(value) => (
            "running",
            value.exit,
            &value.stdout,
            &value.stderr,
            &value.terminal,
            false,
        ),
        ProcessPoll::Finished(value) => {
            let state = if !value.drained || value.resource_error.is_some() {
                "recovery_required"
            } else {
                match value.reason {
                    StopReason::Completed => "finished",
                    StopReason::Cancelled => "cancelled",
                    StopReason::Deadline => "timed_out",
                    StopReason::IoFailure => "io_failed",
                }
            };
            (
                state,
                value.exit,
                &value.stdout,
                &value.stderr,
                &value.terminal,
                state != "finished"
                    || value.error.is_some()
                    || value.exit.is_none_or(|exit| exit.code != Some(0)),
            )
        }
    };
    let content = json!({"processId":id,"state":state,"cumulative":true,
        "exitCode":exit.and_then(|exit|exit.code),"signal":exit.and_then(|exit|exit.signal),
        "stdout":stream(stdout),"stderr":stream(stderr),"terminal":terminal.as_ref().map(stream)})
    .to_string();
    // Host must set a smaller retained-byte budget so JSON escaping fits within
    // the runner budget. Failure retains the handle; no output is silently cut.
    if content.len() > 256 * 1024 {
        return Err(RunError::Budget);
    }
    Ok(ToolOutput {
        content,
        is_error: error,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn joins_contiguous_utf8_but_preserves_real_omissions_and_invalid_bytes() {
        let bytes = "中文".as_bytes();
        let output = RetainedOutput {
            head: bytes[..2].to_vec(),
            tail: bytes[2..].to_vec(),
            omitted_bytes: 0,
        };
        assert_eq!(stream(&output)["head"], "中文");
        assert_eq!(stream(&output)["lossyUtf8"], false);
        let output = RetainedOutput {
            omitted_bytes: 8,
            ..output
        };
        assert_eq!(stream(&output)["omittedBytes"], 8);
        assert_eq!(stream(&output)["lossyUtf8"], true);
    }
}
