//! Opt-in real inference over a generated PDF, never application data or private documents.
use base64::{Engine as _, engine::general_purpose::STANDARD};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::{
    fs,
    path::PathBuf,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{Message, client::IntoClientRequest},
};
struct OwnedChild(Child);
impl Drop for OwnedChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn pdf(pages: usize) -> Vec<u8> {
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        format!(
            "<< /Type /Pages /Count {pages} /Kids [{}] >>",
            (0..pages)
                .map(|i| format!("{} 0 R", 4 + i * 2))
                .collect::<Vec<_>>()
                .join(" ")
        ),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into(),
    ];
    for n in 0..pages {
        objects.push(format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 600 800] /Resources << /Font << /F1 3 0 R >> >> /Contents {} 0 R >>", 5 + n * 2));
        let content = format!(
            "BT /F1 16 Tf 50 700 Td (Synthetic page {}) Tj 0 -40 Td (A generated document for parsing progress and cancellation.) Tj ET",
            n + 1
        );
        objects.push(format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            content.len(),
            content
        ));
    }
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = vec![0];
    for (i, object) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{}\nendobj\n", i + 1, object).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len()).as_bytes());
    for offset in &offsets[1..] {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            offsets.len()
        )
        .as_bytes(),
    );
    out
}
#[tokio::test]
#[ignore = "requires installed FP32 models and loopback access; generates its own PDF"]
async fn actual_page_progress_and_owned_cancellation_keep_the_daemon_reusable() {
    let models =
        PathBuf::from(std::env::var("RESEARCH_ASSISTANT_DOCLING_TEST_MODEL_DIRECTORY").unwrap());
    let root = tempfile::tempdir().unwrap();
    let registry = root.path().join("registry.json");
    let token_file = root.path().join("token");
    let binary = std::env::var_os("RESEARCH_ASSISTANT_DOCLING_TEST_BINARY")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_research-docling-daemon")));
    let mut command = Command::new(binary);
    for (key, _) in std::env::vars_os() {
        if key
            .to_str()
            .is_some_and(|k| k.starts_with("DOCLING_") || k.starts_with("ORT_"))
        {
            command.env_remove(key);
        }
    }
    command
        .env("DOCLING_RS_FP32", "1")
        .env("DOCLING_RS_EP", "cpu")
        .env("DOCLING_RS_MODELS_DIR", &models)
        .env("DOCLING_RS_PDF_WORKERS", "2");
    for (key, file) in [
        ("DOCLING_LAYOUT_ONNX", "layout_heron.onnx"),
        ("DOCLING_OCR_REC_ONNX", "ocr_rec_v6.onnx"),
        ("DOCLING_OCR_DICT", "ocr_rec_v6_dict.txt"),
        ("DOCLING_TABLEFORMER_ENCODER", "tableformer/encoder.onnx"),
        ("DOCLING_TABLEFORMER_DECODER", "tableformer/decoder.onnx"),
        ("DOCLING_TABLEFORMER_BBOX", "tableformer/bbox.onnx"),
    ] {
        command.env(key, models.join(file));
    }
    let _child = OwnedChild(
        command
            .args(["--model-directory"])
            .arg(&models)
            .arg("--registry-path")
            .arg(&registry)
            .arg("--token-path")
            .arg(&token_file)
            .args([
                "--default-port",
                "0",
                "--fallback-start-port",
                "0",
                "--fallback-end-port",
                "0",
                "--idle-shutdown-ms",
                "120000",
            ])
            .current_dir(root.path())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(60);
    while !registry.exists() {
        assert!(Instant::now() < deadline, "readiness timeout");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let record: Value = serde_json::from_slice(&fs::read(&registry).unwrap()).unwrap();
    let token = fs::read_to_string(&token_file).unwrap();
    let base = record["httpBaseUrl"].as_str().unwrap();
    let mut request = record["websocketUrl"]
        .as_str()
        .unwrap()
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("X-Api-Key", token.parse().unwrap());
    let (mut socket, _) = connect_async(request).await.unwrap();
    socket
        .send(Message::Text(
            json!({"type":"hello","clientId":"synthetic-task-test","protocolVersion":2})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    let grant: Value =
        serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
    let lease = grant["leaseId"].as_str().unwrap();
    let headers = [("X-Api-Key", token.as_str()), ("X-Docling-Lease-Id", lease)]
        .into_iter()
        .map(|(k, v)| (k.parse().unwrap(), v.parse().unwrap()))
        .collect();
    let client = reqwest::Client::builder()
        .default_headers(headers)
        .timeout(Duration::from_secs(60))
        .build()
        .unwrap();
    let id = uuid::Uuid::new_v4().to_string();
    let url = format!("{base}/local-service/v1/conversions/{id}");
    let conversion = client
        .post(format!("{base}/v1/convert/source"))
        .header("X-Docling-Request-Id", &id)
        .json(&json!({"sources":[{"kind":"file","base64_string":STANDARD.encode(pdf(20))}]}))
        .send();
    let conversion = tokio::spawn(conversion);
    let mut done = 0;
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        assert!(Instant::now() < deadline, "no actual page progress");
        let response = client.get(&url).send().await.unwrap();
        if response.status().is_success() {
            let progress: Value = response.json().await.unwrap();
            let next = progress["completedPages"].as_u64().unwrap();
            assert!(next >= done);
            done = next;
            if done > 0 {
                assert_eq!(progress["totalPages"], 20);
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(
        client
            .delete(&url)
            .send()
            .await
            .unwrap()
            .status()
            .is_success()
    );
    let response = tokio::time::timeout(Duration::from_secs(20), conversion)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::GONE);
    let status: Value = client
        .get(format!("{base}/local-service/v1/status"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(status["state"], "ready");
    assert_eq!(status["activeRequestCount"], 0);
    // Clearing a cancelled pipeline's hooks permits the next conversion on the same warm daemon.
    let response = client
        .post(format!("{base}/v1/convert/source"))
        .json(&json!({"sources":[{"kind":"file","base64_string":STANDARD.encode(pdf(1))}]}))
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
    println!("generated PDF: observed {done}/20 pages, cancelled, daemon reused successfully");
    socket.close(None).await.unwrap();
}
