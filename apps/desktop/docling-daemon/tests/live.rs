//! Explicit real-model regression; no private documents or application database.
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

#[tokio::test]
#[ignore = "requires verified FP32 resources, explicit public PDF fixtures and loopback access"]
async fn native_pdf_json_and_shared_leases_with_real_fp32_models() {
    let models =
        PathBuf::from(std::env::var("RESEARCH_ASSISTANT_DOCLING_TEST_MODEL_DIRECTORY").unwrap());
    let fixtures =
        PathBuf::from(std::env::var("RESEARCH_ASSISTANT_DOCLING_TEST_FIXTURES").unwrap());
    let report = PathBuf::from(std::env::var("RESEARCH_ASSISTANT_DOCLING_TEST_REPORT").unwrap());
    let root = tempfile::tempdir().unwrap();
    let registry = root.path().join("registry.json");
    let token = root.path().join("token");
    let binary = std::env::var("RESEARCH_ASSISTANT_DOCLING_TEST_BINARY")
        .unwrap_or_else(|_| env!("CARGO_BIN_EXE_research-docling-daemon").into());
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
        .env("DOCLING_RS_PDF_WORKERS", "2")
        .env("DOCLING_LAYOUT_ONNX", models.join("layout_heron.onnx"))
        .env("DOCLING_OCR_REC_ONNX", models.join("ocr_rec_v6.onnx"))
        .env("DOCLING_OCR_DICT", models.join("ocr_rec_v6_dict.txt"))
        .env(
            "DOCLING_TABLEFORMER_ENCODER",
            models.join("tableformer/encoder.onnx"),
        )
        .env(
            "DOCLING_TABLEFORMER_DECODER",
            models.join("tableformer/decoder.onnx"),
        )
        .env(
            "DOCLING_TABLEFORMER_BBOX",
            models.join("tableformer/bbox.onnx"),
        )
        .args([
            "--model-directory",
            models.to_str().unwrap(),
            "--registry-path",
            registry.to_str().unwrap(),
            "--token-path",
            token.to_str().unwrap(),
            "--default-port",
            "0",
            "--fallback-start-port",
            "1",
            "--fallback-end-port",
            "1",
            "--idle-shutdown-ms",
            "1000",
        ])
        .current_dir(root.path())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = OwnedChild(command.spawn().unwrap());
    let deadline = Instant::now() + Duration::from_secs(90);
    while !registry.exists() {
        assert!(Instant::now() < deadline, "native startup timed out");
        assert!(
            child.0.try_wait().unwrap().is_none(),
            "native startup failed"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let record: Value = serde_json::from_slice(&fs::read(&registry).unwrap()).unwrap();
    let secret = fs::read_to_string(&token).unwrap();
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(180))
        .build()
        .unwrap();
    let base = record["httpBaseUrl"].as_str().unwrap();
    assert_eq!(
        client
            .get(format!("{base}/local-service/v1/status"))
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    let mut sockets = Vec::new();
    let mut leases = Vec::new();
    for id in ["client-a", "client-b"] {
        let mut request = record["websocketUrl"]
            .as_str()
            .unwrap()
            .into_client_request()
            .unwrap();
        request
            .headers_mut()
            .insert("X-Api-Key", secret.parse().unwrap());
        let (mut socket, _) = connect_async(request).await.unwrap();
        socket
            .send(Message::Text(
                json!({"type":"hello","protocolVersion":2,"clientId":id})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
        let response: Value =
            serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
        assert_eq!(response["serviceInstanceId"], record["serviceInstanceId"]);
        leases.push(response["leaseId"].as_str().unwrap().to_owned());
        sockets.push(socket);
    }
    let status: Value = client
        .get(format!("{base}/local-service/v1/status"))
        .header("X-Api-Key", &secret)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(status["activeClientCount"], 2);
    let mut heartbeat_tasks = Vec::new();
    for (mut socket, lease) in sockets.into_iter().zip(leases.iter().cloned()) {
        let (stop, mut stopped) = tokio::sync::watch::channel(false);
        let task = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _=tokio::time::sleep(Duration::from_secs(5))=>{socket.send(Message::Text(json!({"type":"heartbeat","leaseId":lease}).to_string().into())).await.unwrap();let _=socket.next().await;},
                    _=stopped.changed()=>{let _=socket.send(Message::Text(json!({"type":"release"}).to_string().into())).await;let _=socket.close(None).await;break;}
                }
            }
        });
        heartbeat_tasks.push((stop, task));
    }
    let mut reports = Vec::new();
    for name in ["ocr_test.pdf", "multi_page.pdf", "scan-cjk.pdf"] {
        let path = fixtures.join(name);
        let bytes = fs::read(&path).unwrap();
        let started = Instant::now();
        let response=client.post(format!("{base}/v1/convert/source")).header("X-Api-Key",&secret).header("X-Docling-Lease-Id",&leases[0])
            .json(&json!({"sources":[{"kind":"file","filename":name,"base64_string":STANDARD.encode(bytes)}],"options":{"from_formats":["pdf"],"to_formats":["json"]}})).send().await.unwrap();
        assert!(response.status().is_success(), "conversion failed: {name}");
        let value: Value = response.json().await.unwrap();
        assert_eq!(value["runtime"]["precision"], "fp32");
        assert_eq!(
            value["runtime"]["engineVersion"],
            research_docling_runtime::ENGINE_VERSION
        );
        let doc = &value["document"]["json_content"];
        let texts = doc["texts"].as_array().unwrap();
        assert!(!texts.is_empty(), "empty text: {name}");
        let mut regions = 0;
        for item in texts {
            assert!(
                item["self_ref"]
                    .as_str()
                    .is_some_and(|s| s.starts_with("#/texts/"))
            );
            for position in item["prov"].as_array().unwrap() {
                assert!(position["page_no"].as_u64().unwrap() > 0);
                assert!(position["bbox"]["l"].is_number());
                assert!(
                    position["charspan"]
                        .as_array()
                        .is_some_and(|span| span.len() == 2)
                );
                regions += 1;
            }
        }
        assert!(regions > 0);
        let table_cells = doc["tables"]
            .as_array()
            .map(|tables| {
                tables
                    .iter()
                    .flat_map(|t| t["data"]["table_cells"].as_array().into_iter().flatten())
                    .filter_map(|c| c["text"].as_str())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let text = texts
            .iter()
            .filter_map(|t| t["text"].as_str())
            .chain(table_cells.iter().copied())
            .collect::<Vec<_>>()
            .join(" ");
        if name == "scan-cjk.pdf" {
            for expected in ["中文", "研究结果", "Control", "Treatment", "12", "24"] {
                assert!(text.contains(expected), "OCR missing {expected}");
            }
            assert!(table_cells.len() >= 6, "scan table not reconstructed");
        }
        fs::write(
            report.with_file_name(format!("{name}.native.json")),
            serde_json::to_vec_pretty(&value).unwrap(),
        )
        .unwrap();
        reports.push(json!({"fixture":name,"seconds":started.elapsed().as_secs_f64(),"textItems":texts.len(),"provenanceRegions":regions,"tableCells":table_cells.len(),"precision":"fp32"}));
    }
    let (first_stop, first_task) = heartbeat_tasks.remove(0);
    first_stop.send(true).unwrap();
    first_task.await.unwrap();
    let status: Value = client
        .get(format!("{base}/local-service/v1/status"))
        .header("X-Api-Key", &secret)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(status["activeClientCount"], 1);
    let (stop, task) = heartbeat_tasks.remove(0);
    stop.send(true).unwrap();
    task.await.unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while child.0.try_wait().unwrap().is_none() {
        assert!(Instant::now() < deadline, "daemon did not idle-exit");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(!registry.exists());
    assert!(!token.exists());
    fs::write(
        report,
        serde_json::to_vec_pretty(&json!({"sharedClients":2,"idleExit":true,"fixtures":reports}))
            .unwrap(),
    )
    .unwrap();
}
