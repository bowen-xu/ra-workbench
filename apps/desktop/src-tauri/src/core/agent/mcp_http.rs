//! Streamable HTTP MCP adapter.
//!
//! The wire contract follows the Codex-derived MCP transport boundary: one
//! JSON-RPC request per POST, an optional `Mcp-Session-Id`, and either a JSON
//! response or an SSE response containing a JSON `data:` event. URL and
//! credentials are constructed by the trusted native host.

use futures_util::StreamExt;
use reqwest::header::{ACCEPT, CONTENT_TYPE, HeaderMap, HeaderValue};
use research_agent_core::mcp::{self, Request, Response};
use std::time::Duration;
use tokio::time::timeout;

const SESSION_HEADER: &str = "Mcp-Session-Id";
const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("MCP HTTP request failed")]
    Request,
    #[error("MCP HTTP response exceeded its budget")]
    Budget,
    #[error("MCP HTTP response was not valid JSON-RPC")]
    Protocol,
    #[error("MCP HTTP response timed out")]
    Timeout,
}

pub struct Client {
    http: reqwest::Client,
    url: reqwest::Url,
    headers: HeaderMap,
    session_id: Option<HeaderValue>,
}

impl Client {
    pub fn new(url: &str, secret: Option<&str>) -> Result<Self, Error> {
        let url = reqwest::Url::parse(url).map_err(|_| Error::Request)?;
        let mut headers = HeaderMap::new();
        headers.insert(
            ACCEPT,
            HeaderValue::from_static("application/json, text/event-stream"),
        );
        if let Some(secret) = secret {
            let mut value =
                HeaderValue::from_str(&format!("Bearer {secret}")).map_err(|_| Error::Request)?;
            value.set_sensitive(true);
            headers.insert(reqwest::header::AUTHORIZATION, value);
        }
        let http = reqwest::Client::builder()
            // MCP credentials are host-owned and must never follow a server
            // redirect to an unreviewed destination.
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| Error::Request)?;
        Ok(Self {
            http,
            url,
            headers,
            session_id: None,
        })
    }

    pub async fn request(&mut self, request: &Request, wait: Duration) -> Result<Response, Error> {
        let future = async {
            let mut builder = self
                .http
                .post(self.url.clone())
                .headers(self.headers.clone())
                .header(CONTENT_TYPE, "application/json")
                .json(request);
            if let Some(session_id) = &self.session_id {
                builder = builder.header(SESSION_HEADER, session_id.clone());
            }
            let response = builder.send().await.map_err(|_| Error::Request)?;
            if let Some(session_id) = response.headers().get(SESSION_HEADER) {
                self.session_id = Some(session_id.clone());
            }
            if !response.status().is_success() {
                return Err(Error::Request);
            }
            let mut bytes = Vec::new();
            let mut stream = response.bytes_stream();
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(|_| Error::Request)?;
                if bytes.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
                    return Err(Error::Budget);
                }
                bytes.extend_from_slice(&chunk);
            }
            let body = if bytes.starts_with(b"data:")
                || bytes.windows(6).any(|window| window == b"\ndata:")
            {
                bytes
                    .split(|byte| *byte == b'\n')
                    .filter_map(|line| line.strip_prefix(b"data:"))
                    .map(|line| line.trim_ascii())
                    .find(|line| !line.is_empty())
                    .ok_or(Error::Protocol)?
                    .to_vec()
            } else {
                bytes.to_vec()
            };
            mcp::decode_response(&body, request.id).map_err(|_| Error::Protocol)
        };
        timeout(wait, future).await.map_err(|_| Error::Timeout)?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use research_agent_core::mcp::Request;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn posts_json_rpc_and_remembers_session_header() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            for request_index in 0..2 {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                let mut chunk = [0_u8; 1024];
                loop {
                    let count = stream.read(&mut chunk).await.unwrap();
                    if count == 0 {
                        break;
                    }
                    request.extend_from_slice(&chunk[..count]);
                    if request.windows(4).any(|window| window == b"\r\n\r\n") {
                        break;
                    }
                }
                assert!(
                    request
                        .windows(16)
                        .any(|window| window == b"application/json")
                );
                let request_lower = request
                    .iter()
                    .map(u8::to_ascii_lowercase)
                    .collect::<Vec<_>>();
                if request_index == 0 {
                    assert!(
                        !request
                            .windows(b"mcp-session-id".len())
                            .any(|window| window == b"mcp-session-id")
                    );
                } else {
                    assert!(
                        request_lower
                            .windows(b"mcp-session-id".len())
                            .any(|window| window == b"mcp-session-id")
                    );
                }
                let id = request_index + 1;
                let body =
                    format!("{{\"jsonrpc\":\"2.0\",\"id\":{id},\"result\":{{\"ok\":true}}}}");
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nMcp-Session-Id: test-session\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                stream.write_all(response.as_bytes()).await.unwrap();
            }
        });
        let mut client = Client::new(&format!("http://{address}/mcp"), None).unwrap();
        let response = client
            .request(
                &Request {
                    jsonrpc: "2.0".into(),
                    id: 1,
                    method: "initialize".into(),
                    params: serde_json::json!({}),
                },
                Duration::from_secs(2),
            )
            .await
            .unwrap();
        assert_eq!(response.id, 1);
        assert_eq!(client.session_id.as_ref().unwrap(), "test-session");
        let response = client
            .request(
                &Request {
                    jsonrpc: "2.0".into(),
                    id: 2,
                    method: "tools/list".into(),
                    params: serde_json::json!({}),
                },
                Duration::from_secs(2),
            )
            .await
            .unwrap();
        assert_eq!(response.id, 2);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn accepts_streamable_http_sse_data_events() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0_u8; 1024];
            let _ = stream.read(&mut request).await.unwrap();
            let body = b"event: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":7,\"result\":{\"ok\":true}}\n\n";
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            stream.write_all(response.as_bytes()).await.unwrap();
            stream.write_all(body).await.unwrap();
        });
        let mut client = Client::new(&format!("http://{address}/mcp"), None).unwrap();
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
        server.await.unwrap();
    }

    #[tokio::test]
    async fn rejects_responses_over_the_transport_budget() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0_u8; 1024];
            let _ = stream.read(&mut request).await.unwrap();
            let body = vec![b'x'; MAX_RESPONSE_BYTES + 1];
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes()).await;
            let _ = stream.write_all(&body).await;
        });
        let mut client = Client::new(&format!("http://{address}/mcp"), None).unwrap();
        let error = client
            .request(
                &Request {
                    jsonrpc: "2.0".into(),
                    id: 9,
                    method: "tools/list".into(),
                    params: serde_json::json!({}),
                },
                Duration::from_secs(2),
            )
            .await
            .unwrap_err();
        assert!(matches!(error, Error::Budget));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn never_follows_redirects_with_mcp_credentials() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0_u8; 2048];
            let count = stream.read(&mut request).await.unwrap();
            let request = &request[..count];
            assert!(
                request
                    .windows(b"authorization: Bearer secret".len())
                    .any(|window| window.eq_ignore_ascii_case(b"authorization: Bearer secret"))
            );
            let response = format!(
                "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:{}/redirected\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                address.port()
            );
            stream.write_all(response.as_bytes()).await.unwrap();
            tokio::time::timeout(Duration::from_millis(100), listener.accept())
                .await
                .expect_err("redirect must not trigger a second request");
        });
        let mut client = Client::new(&format!("http://{address}/mcp"), Some("secret")).unwrap();
        let error = client
            .request(
                &Request {
                    jsonrpc: "2.0".into(),
                    id: 12,
                    method: "initialize".into(),
                    params: serde_json::json!({}),
                },
                Duration::from_secs(2),
            )
            .await
            .unwrap_err();
        assert!(matches!(error, Error::Request));
        server.await.unwrap();
    }

    #[test]
    fn marks_mcp_authorization_header_sensitive() {
        let client = Client::new("http://127.0.0.1:1/mcp", Some("secret-value")).unwrap();
        assert!(client.headers[reqwest::header::AUTHORIZATION].is_sensitive());
        assert!(!format!("{:?}", client.headers).contains("secret-value"));
    }
}
