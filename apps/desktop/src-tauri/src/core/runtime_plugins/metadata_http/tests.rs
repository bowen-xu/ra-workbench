//! Synthetic loopback HTTP only. Run server and clients in the same namespace.
use super::*;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    task::JoinHandle,
    time::timeout,
};

const WAIT: Duration = Duration::from_secs(15);

async fn server(response: Vec<u8>) -> (Url, JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = Url::parse(&format!(
        "http://{}/fixture",
        listener.local_addr().unwrap()
    ))
    .unwrap();
    let task = tokio::spawn(async move {
        timeout(WAIT, async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            loop {
                let byte = socket.read_u8().await.unwrap();
                request.push(byte);
                if request.ends_with(b"\r\n\r\n") {
                    break;
                }
                assert!(request.len() < 8192);
            }
            socket.write_all(&response).await.unwrap();
            socket.shutdown().await.unwrap();
            String::from_utf8(request).unwrap()
        })
        .await
        .unwrap()
    });
    (url, task)
}

async fn read(response: &[u8], limit: usize) -> Result<DoiHttpBodyV1, DoiHttpErrorV1> {
    let (url, task) = server(response.to_vec()).await;
    let result = timeout(
        WAIT,
        MetadataHttp::new()
            .unwrap()
            .get(url, "application/json", limit),
    )
    .await
    .unwrap();
    task.await.unwrap();
    result
}

#[tokio::test]
async fn retains_original_get_headers_content_type_and_body() {
    let (url, task) = server(b"HTTP/1.1 200 OK\r\nContent-Type: application/json; charset=UTF-8\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}".to_vec()).await;
    let result = MetadataHttp::new()
        .unwrap()
        .get(url, "application/json", 2)
        .await
        .unwrap();
    let request = task.await.unwrap();
    assert!(request.starts_with("GET /fixture HTTP/1.1\r\n"));
    let headers = request.to_ascii_lowercase();
    assert!(headers.contains("accept: application/json\r\n"));
    assert!(
        headers.contains("user-agent: researchassistant/0.1 (local desktop research tool)\r\n")
    );
    assert_eq!(result.body, "{}");
    assert_eq!(result.content_type, "application/json; charset=UTF-8");
    assert!(!result.final_https);
}

#[tokio::test]
async fn not_found_and_http_status_precede_length_or_body_failures() {
    assert_eq!(
        read(
            b"HTTP/1.1 404 Not Found\r\nContent-Length: 999\r\nConnection: close\r\n\r\n",
            1
        )
        .await,
        Err(DoiHttpErrorV1::NotFound {}),
    );
    let error = read(
        b"HTTP/1.1 503 Unavailable\r\nContent-Length: 999\r\nConnection: close\r\n\r\n",
        1,
    )
    .await
    .unwrap_err();
    assert!(matches!(error, DoiHttpErrorV1::Status {status:503, cause} if cause.contains("503")));
}

#[tokio::test]
async fn declared_and_actual_length_limits_keep_the_original_order() {
    assert_eq!(
        read(
            b"HTTP/1.1 200 OK\r\nContent-Length: 999\r\nConnection: close\r\n\r\n",
            1
        )
        .await,
        Err(DoiHttpErrorV1::TooLarge { status: 200 }),
    );
    assert_eq!(
        read(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n3\r\nabc\r\n0\r\n\r\n", 2).await,
        Err(DoiHttpErrorV1::TooLarge { status: 200 }),
    );
}

#[tokio::test]
async fn truncated_body_precedes_utf8_and_invalid_utf8_is_not_replaced() {
    let error = read(
        b"HTTP/1.1 200 OK\r\nContent-Length: 20\r\nConnection: close\r\n\r\n\xff",
        20,
    )
    .await
    .unwrap_err();
    assert!(matches!(error, DoiHttpErrorV1::Read {status:200, cause} if !cause.is_empty()));
    let error = read(
        b"HTTP/1.1 200 OK\r\nContent-Length: 1\r\nConnection: close\r\n\r\n\xff",
        20,
    )
    .await
    .unwrap_err();
    assert!(
        matches!(error, DoiHttpErrorV1::Utf8 {status:200, cause} if cause.contains("invalid utf-8"))
    );
}

#[tokio::test]
async fn unredirected_3xx_is_accepted_and_invalid_content_type_becomes_empty() {
    let result = read(b"HTTP/1.1 302 Found\r\nContent-Type: \xff\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}", 2).await.unwrap();
    assert_eq!(result.body, "{}");
    assert!(result.content_type.is_empty());
}

/// A fresh child has only synthetic proxy settings; no parent proxy value or
/// credential is read, copied, printed or sent to the fixture.
#[tokio::test]
async fn inherits_the_owning_process_proxy_environment() {
    const MARKER: &str = "RESEARCH_DOI_HTTP_PROXY_TEST_CHILD";
    const TARGET: &str = "RESEARCH_DOI_HTTP_PROXY_TEST_TARGET";
    if std::env::var_os(MARKER).is_some() {
        let response = MetadataHttp::new()
            .unwrap()
            .get(
                Url::parse(&std::env::var(TARGET).unwrap()).unwrap(),
                "application/json",
                2,
            )
            .await
            .unwrap();
        assert_eq!(response.body, "{}");
        return;
    }
    for bypass in [false, true] {
        let (url, task) =
            server(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}".to_vec())
                .await;
        let executable = std::env::current_exe().unwrap();
        let proxy = if bypass {
            "http://127.0.0.1:9".to_owned()
        } else {
            format!("http://{}:{}", url.host_str().unwrap(), url.port().unwrap())
        };
        let target = if bypass {
            url.to_string()
        } else {
            "http://doi-proxy-fixture.invalid/resolve".to_owned()
        };
        let output = tokio::task::spawn_blocking(move || {
            std::process::Command::new(executable)
            .args([
                "--exact",
                "runtime_plugins::metadata_http::tests::inherits_the_owning_process_proxy_environment",
                "--nocapture",
            ])
            .env_clear()
            .env(MARKER, "1")
            .env(TARGET, target)
            .env("HTTP_PROXY", proxy)
            .env("NO_PROXY", if bypass { "127.0.0.1" } else { "" })
            .output()
            .unwrap()
        });
        let (request, output) = timeout(WAIT, async { tokio::join!(task, output) })
            .await
            .unwrap();
        let request = request.unwrap();
        let output = output.unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        if bypass {
            assert!(request.starts_with("GET /fixture HTTP/1.1\r\n"));
        } else {
            assert!(
                request.starts_with("GET http://doi-proxy-fixture.invalid/resolve HTTP/1.1\r\n")
            );
        }
    }
}
