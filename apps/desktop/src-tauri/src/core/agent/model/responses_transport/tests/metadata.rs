use super::*;
use crate::agent::model::responses_metadata::ResponsesRequestMetadata;
use research_agent_core::context::compaction_strategy::{CompactionKind, CompactionStrategy};

#[derive(Default)]
struct MetadataSink {
    prepares: AtomicUsize,
}
impl ResponsesReceiptSink for MetadataSink {
    fn prepare_input(
        &self,
        _: Uuid,
        _: ModelIdentity,
        input: Vec<Value>,
    ) -> ModelFuture<'_, Vec<Value>> {
        Box::pin(async move { Ok(input) })
    }
    fn prepare_with_metadata(
        &self,
        _: Uuid,
        _: ModelIdentity,
        input: Vec<Value>,
        _: ResponsesInputKind,
    ) -> ModelFuture<'_, PreparedResponsesInput> {
        self.prepares.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            Ok(PreparedResponsesInput {
                input,
                metadata: Some(ResponsesRequestMetadata::new(
                    Uuid::from_u128(100),
                    Uuid::from_u128(200),
                    7,
                )?),
            })
        })
    }
    fn preserve(&self, _: ResponsesReceipt) -> ModelFuture<'_, ()> {
        Box::pin(async { Ok(()) })
    }
}

#[tokio::test]
async fn codex_window_metadata_reaches_all_http_paths_and_platform_omits_it() {
    for codex in [true, false] {
        let legacy = json!({"output":[compaction_tests::opaque()]}).to_string();
        let json_reply = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{legacy}",
            legacy.len()
        );
        let (url, peer) = sequential_server_with_headers(vec![
            ok(&body()),
            ok(&body()),
            ok(&compaction_tests::events(
                &[compaction_tests::opaque()],
                true,
            )),
            json_reply,
        ])
        .await;
        let sink = Arc::new(MetadataSink::default());
        let mut native = transport(url, sink.clone());
        let (lease, _epoch) = crate::agent::authentication::tests::synthetic_lease();
        if codex {
            native.kind = ResponsesEndpointKind::ChatGpt;
            native.auth = Authentication::SyntheticLease(Arc::new(lease));
        }
        let native = Arc::new(native);
        for text in [false, true] {
            let mut stream = if text {
                native.start_text_compaction(request()).await.unwrap()
            } else {
                native.start(request()).await.unwrap()
            };
            while stream.next().await.unwrap().is_some() {}
        }
        let mut compact = compaction_tests::compact_request();
        ResponsesV2CompactionStrategy::new(native.clone(), compact.binding.clone())
            .unwrap()
            .compact(compact.clone())
            .await
            .unwrap();
        compact.binding.kind = CompactionKind::RemoteWindow;
        ResponsesLegacyCompactionStrategy::new(native, compact.binding.clone())
            .unwrap()
            .compact(compact)
            .await
            .unwrap();
        let (bodies, headers) = peer.await.unwrap();
        assert_eq!(sink.prepares.load(Ordering::SeqCst), 4);
        for (index, (body, headers)) in bodies.iter().zip(headers).enumerate() {
            if codex {
                assert_eq!(body["prompt_cache_key"], Uuid::from_u128(100).to_string());
                if index == 3 {
                    assert!(body.get("client_metadata").is_none());
                } else {
                    assert_eq!(
                        body["client_metadata"]["x-codex-window-id"],
                        format!("{}:7", Uuid::from_u128(100))
                    );
                    assert_eq!(
                        body["client_metadata"]["turn_id"],
                        Uuid::from_u128(200).to_string()
                    );
                }
                assert!(
                    headers
                        .to_ascii_lowercase()
                        .contains(&format!("x-codex-window-id: {}:7", Uuid::from_u128(100)))
                );
            } else {
                assert!(body.get("prompt_cache_key").is_none());
                assert!(body.get("client_metadata").is_none());
                assert!(!headers.to_ascii_lowercase().contains("x-codex-window-id"));
            }
            assert!(!body["input"].to_string().contains("client_metadata"));
        }
    }
}

#[tokio::test]
async fn credential_refresh_reuses_frozen_request_metadata_without_repreparation() {
    let (first, _first_epoch) =
        crate::agent::authentication::tests::synthetic_lease_for_account("same-account");
    let (second, _second_epoch) =
        crate::agent::authentication::tests::synthetic_lease_for_account("same-account");
    let credentials = Arc::new(CredentialSequence {
        leases: Mutex::new(VecDeque::from([first, second])),
        acquisitions: AtomicUsize::new(0),
    });
    let (url, peer) = sequential_server_with_headers(vec![
        "HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(),
        ok(&body()),
    ])
    .await;
    let sink = Arc::new(MetadataSink::default());
    let native = synthetic_chatgpt_transport(url, credentials, sink.clone());
    let mut stream = native.start(request()).await.unwrap();
    while stream.next().await.unwrap().is_some() {}
    let (bodies, headers) = peer.await.unwrap();
    assert_eq!(bodies.len(), 2);
    assert_eq!(bodies[0], bodies[1]);
    assert_eq!(sink.prepares.load(Ordering::SeqCst), 1);
    for header in headers {
        assert!(
            header
                .to_ascii_lowercase()
                .contains(&format!("x-codex-window-id: {}:7", Uuid::from_u128(100)))
        );
    }
}
