//! MCP stdio adapter built on the shared Codex-derived PipeProcess.
//! The host must construct ProcessSpec from an approved extension descriptor;
//! model input is never allowed to choose the program or environment.

use research_agent_core::mcp::{self, FrameDecoder, Request, Response};
use serde_json::Value;
use std::collections::VecDeque;
use std::time::Duration;
use tokio::time::timeout;

use super::process::{OutputStream, PipeInput, PipeProcess, ProcessError, ProcessSpec};

const MAX_STDERR_BYTES: usize = 16 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("MCP process could not start or write")]
    Process(#[from] ProcessError),
    #[error("MCP protocol rejected the response")]
    Protocol(#[from] mcp::Error),
    #[error("MCP response timed out")]
    Timeout,
    #[error("MCP process closed before responding")]
    Closed,
    #[error("MCP stderr exceeded its diagnostic budget")]
    StderrBudget,
}

pub struct StdioClient {
    process: PipeProcess,
    input: PipeInput,
    decoder: FrameDecoder,
    pending: VecDeque<Vec<u8>>,
    stderr: Vec<u8>,
    failed: bool,
}

// Dropping an in-flight call leaves an unframed late response in the pipe.
// Reuse would attribute it to another request, so cancel terminates the client
// under its existing process owner just like timeout/protocol failure.
struct RequestGuard<'a> {
    client: &'a mut StdioClient,
    completed: bool,
}
impl Drop for RequestGuard<'_> {
    fn drop(&mut self) {
        if !self.completed {
            self.client.poison();
        }
    }
}

impl StdioClient {
    pub(crate) fn is_failed(&self) -> bool {
        self.failed
    }

    fn poison(&mut self) {
        self.failed = true;
        // A poisoned stream cannot be safely reused. Terminate the process
        // group immediately rather than retaining a blocked server until the
        // next runtime refresh or owner drop.
        let _ = self.process.terminate();
    }

    pub fn spawn(spec: ProcessSpec) -> Result<Self, Error> {
        let (process, input) = PipeProcess::spawn(spec)?;
        Ok(Self {
            process,
            input,
            decoder: FrameDecoder::new(),
            pending: VecDeque::new(),
            stderr: Vec::new(),
            failed: false,
        })
    }

    pub async fn request(&mut self, request: &Request, wait: Duration) -> Result<Response, Error> {
        if self.failed {
            return Err(Error::Closed);
        }
        let mut guard = RequestGuard {
            client: self,
            completed: false,
        };
        // The configured request deadline includes stdin backpressure. A
        // server that stops reading must not keep its tool/turn owner forever.
        let result = match timeout(wait, guard.client.request_inner(request)).await {
            Ok(result) => result,
            Err(_) => {
                guard.client.poison();
                Err(Error::Timeout)
            }
        };
        guard.completed = true;
        result
    }

    async fn request_inner(&mut self, request: &Request) -> Result<Response, Error> {
        let mut bytes = mcp::encode_request(request)?;
        bytes.push(b'\n');
        let mut written = 0;
        while written < bytes.len() {
            let count = match self.input.write(&bytes[written..]).await {
                Ok(count) => count,
                Err(error) => {
                    self.poison();
                    return Err(error.into());
                }
            };
            if count == 0 {
                self.poison();
                return Err(Error::Closed);
            }
            written += count;
        }
        let frame = match self.next_frame(request.id).await {
            Ok(frame) => frame,
            Err(error) => {
                self.poison();
                return Err(error);
            }
        };
        match mcp::decode_response(&frame, request.id) {
            Ok(response) => Ok(response),
            Err(error) => {
                self.poison();
                Err(error.into())
            }
        }
    }

    async fn next_frame(&mut self, expected_id: u64) -> Result<Vec<u8>, Error> {
        loop {
            while let Some(frame) = self.pending.pop_front() {
                if is_notification(&frame)? {
                    continue;
                }
                mcp::decode_response(&frame, expected_id)?;
                return Ok(frame);
            }
            let Some(chunk) = self.process.next_output().await? else {
                return Err(Error::Closed);
            };
            match chunk.stream {
                OutputStream::Stdout => {
                    self.pending.extend(self.decoder.push(&chunk.bytes)?);
                }
                OutputStream::Stderr => {
                    self.stderr.extend_from_slice(&chunk.bytes);
                    if self.stderr.len() > MAX_STDERR_BYTES {
                        return Err(Error::StderrBudget);
                    }
                }
            }
        }
    }

    pub fn stderr(&self) -> &[u8] {
        &self.stderr
    }
}

/// MCP servers may emit JSON-RPC notifications (for example
/// `notifications/tools/list_changed`) between responses. They have no `id`
/// and must be consumed without being mistaken for the response to the active
/// request. Invalid notification shapes still fail closed.
fn is_notification(frame: &[u8]) -> Result<bool, Error> {
    let value: Value = serde_json::from_slice(frame).map_err(|_| mcp::Error::InvalidRequest)?;
    if value.get("id").is_some() {
        return Ok(false);
    }
    let Some(method) = value.get("method").and_then(Value::as_str) else {
        return Ok(false);
    };
    if method.trim().is_empty()
        || method.len() > 128
        || method.chars().any(char::is_control)
        || value
            .get("params")
            .is_some_and(|params| !params.is_object())
    {
        return Err(mcp::Error::InvalidRequest.into());
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use research_agent_core::mcp::Request;
    use std::{collections::BTreeMap, ffi::OsString, future::Future, path::PathBuf};

    #[tokio::test]
    async fn reuses_pipe_process_for_bounded_stdio_request() {
        let spec = ProcessSpec {
            program: PathBuf::from("/bin/sh"),
            arguments: vec![
                OsString::from("-c"),
                OsString::from(
                    "IFS= read line; printf '%s\\n' '{\"jsonrpc\":\"2.0\",\"id\":7,\"result\":{\"ok\":true}}'",
                ),
            ],
            directory: std::env::temp_dir(),
            environment: BTreeMap::new(),
        };
        let mut client = StdioClient::spawn(spec).unwrap();
        let response = client
            .request(
                &Request {
                    jsonrpc: "2.0".into(),
                    id: 7,
                    method: "tools/list".into(),
                    params: serde_json::json!({}),
                },
                Duration::from_secs(2),
            )
            .await
            .unwrap();
        assert_eq!(response.id, 7);
        assert!(client.stderr().is_empty());
    }

    #[tokio::test]
    async fn skips_json_rpc_notifications_before_the_expected_response() {
        let spec = ProcessSpec {
            program: PathBuf::from("/bin/sh"),
            arguments: vec![
                OsString::from("-c"),
                OsString::from(
                    "IFS= read line; printf '%s\\n' '{\"jsonrpc\":\"2.0\",\"method\":\"notifications/tools/list_changed\"}'; printf '%s\\n' '{\"jsonrpc\":\"2.0\",\"id\":7,\"result\":{\"ok\":true}}'",
                ),
            ],
            directory: std::env::temp_dir(),
            environment: BTreeMap::new(),
        };
        let mut client = StdioClient::spawn(spec).unwrap();
        let response = client
            .request(
                &Request {
                    jsonrpc: "2.0".into(),
                    id: 7,
                    method: "tools/list".into(),
                    params: serde_json::json!({}),
                },
                Duration::from_secs(2),
            )
            .await
            .unwrap();
        assert_eq!(response.id, 7);
    }

    #[tokio::test]
    async fn timeout_poisoning_prevents_reusing_a_late_response_stream() {
        let spec = ProcessSpec {
            program: PathBuf::from("/bin/sh"),
            arguments: vec![
                OsString::from("-c"),
                OsString::from("IFS= read line; sleep 2"),
            ],
            directory: std::env::temp_dir(),
            environment: BTreeMap::new(),
        };
        let mut client = StdioClient::spawn(spec).unwrap();
        let request = Request {
            jsonrpc: "2.0".into(),
            id: 8,
            method: "tools/list".into(),
            params: serde_json::json!({}),
        };
        assert!(matches!(
            client.request(&request, Duration::from_millis(10)).await,
            Err(Error::Timeout)
        ));
        assert!(matches!(
            client.request(&request, Duration::from_secs(2)).await,
            Err(Error::Closed)
        ));
    }
    #[tokio::test]
    async fn dropping_request_poisoning_prevents_late_response_reuse() {
        let mut client = StdioClient::spawn(ProcessSpec {
            program: PathBuf::from("/bin/sh"),
            arguments: vec![
                OsString::from("-c"),
                OsString::from("while IFS= read line; do :; done"),
            ],
            directory: std::env::temp_dir(),
            environment: BTreeMap::new(),
        })
        .unwrap();
        let request = Request {
            jsonrpc: "2.0".into(),
            id: 1,
            method: "tools/list".into(),
            params: serde_json::json!({}),
        };
        {
            let future = client.request(&request, Duration::from_secs(30));
            tokio::pin!(future);
            tokio::select! { biased;
                _ = &mut future => panic!("fixture must not respond"),
                _ = tokio::task::yield_now() => {},
            }
        }
        assert!(client.failed);
        assert!(matches!(
            client.request(&request, Duration::from_secs(1)).await,
            Err(Error::Closed)
        ));
    }

    #[tokio::test]
    async fn configured_deadline_covers_backpressured_stdin_and_reaps_poisoned_owner() {
        let mut client = StdioClient::spawn(ProcessSpec {
            program: PathBuf::from("/bin/sh"),
            // A deterministic synthetic server: announce readiness, then stop
            // without ever reading stdin. SIGKILL from poison still terminates it.
            arguments: vec![
                OsString::from("-c"),
                OsString::from("printf ready; kill -STOP $$"),
            ],
            directory: std::env::temp_dir(),
            environment: BTreeMap::new(),
        })
        .unwrap();
        let ready = timeout(Duration::from_secs(2), client.process.next_output())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(ready.bytes, b"ready");
        let request = Request {
            jsonrpc: "2.0".into(),
            id: 17,
            method: "tools/call".into(),
            params: serde_json::json!({"name":"search","arguments":{"text":"x".repeat(1024 * 1024)}}),
        };
        assert!(
            mcp::encode_request(&request).is_ok(),
            "large request stays inside the MCP frame budget"
        );
        {
            let mut execution = Box::pin(client.request(&request, Duration::from_millis(100)));
            assert!(
                std::future::poll_fn(|cx| std::task::Poll::Ready(
                    execution.as_mut().poll(cx).is_pending()
                ))
                .await
            );
            assert!(matches!(
                timeout(Duration::from_secs(2), execution).await.unwrap(),
                Err(Error::Timeout)
            ));
        }
        assert!(client.failed);
        assert!(matches!(
            client.request(&request, Duration::from_secs(1)).await,
            Err(Error::Closed)
        ));
        timeout(Duration::from_secs(2), async {
            while client.process.next_output().await.unwrap().is_some() {}
        })
        .await
        .unwrap();
        assert!(
            client.process.exit().is_some(),
            "poisoned process was reaped"
        );
    }
}
