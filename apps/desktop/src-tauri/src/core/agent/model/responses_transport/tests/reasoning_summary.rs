use super::*;
use crate::agent::model::responses_reasoning::ResponsesReasoningSummary;
use research_agent_core::context::compaction_strategy::{CompactionKind, CompactionStrategy};

#[tokio::test]
async fn reasoning_summary_reaches_sampling_text_v2_and_legacy_without_losing_effort() {
    for codex in [true, false] {
        for summary in [
            None,
            Some(ResponsesReasoningSummary::Auto),
            Some(ResponsesReasoningSummary::Concise),
            Some(ResponsesReasoningSummary::Detailed),
        ] {
            let legacy = json!({"output":[compaction_tests::opaque()]}).to_string();
            let reply = format!(
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
                reply,
            ])
            .await;
            let mut native = transport(url, Arc::new(Receipts::default()));
            let (lease, _epoch) = crate::agent::authentication::tests::synthetic_lease();
            if codex {
                native.kind = ResponsesEndpointKind::ChatGpt;
                native.auth = Authentication::SyntheticLease(Arc::new(lease));
            }
            native.config.capabilities.reasoning_efforts = vec!["medium".into()];
            let native = Arc::new(
                native
                    .with_reasoning_summary(request().model, summary)
                    .unwrap(),
            );
            let mut sampling = request();
            sampling.reasoning_effort = Some("medium".into());
            for text in [false, true] {
                let mut stream = if text {
                    native
                        .start_text_compaction(sampling.clone())
                        .await
                        .unwrap()
                } else {
                    native.start(sampling.clone()).await.unwrap()
                };
                while stream.next().await.unwrap().is_some() {}
            }
            let mut compact = compaction_tests::compact_request();
            compact.source = sampling;
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
            let (bodies, _) = peer.await.unwrap();
            assert_eq!(bodies.len(), 4);
            for body in bodies {
                assert_eq!(body["reasoning"]["effort"], "medium");
                assert_eq!(
                    body["reasoning"].get("summary").cloned(),
                    summary.map(|summary| json!(summary))
                );
            }
        }
    }
}

#[test]
fn summary_without_effort_is_encoded_and_other_model_or_revision_is_rejected() {
    let native = transport("http://127.0.0.1:1".into(), Arc::new(Receipts::default()))
        .with_reasoning_summary(request().model, Some(ResponsesReasoningSummary::Auto))
        .unwrap();
    let request = request();
    let body = native.request_body(&request, false).unwrap();
    assert_eq!(body["reasoning"], json!({"summary":"auto"}));
    let mut revision = request.clone();
    revision.model.revision = Some("other-revision".into());
    assert!(matches!(
        native.request_body(&revision, true),
        Err(ModelError::UnsupportedCapability)
    ));
    let mut other = request;
    other.model.model_id = "other-model".into();
    assert!(matches!(
        native.request_body(&other, false),
        Err(ModelError::UnsupportedCapability)
    ));
}
