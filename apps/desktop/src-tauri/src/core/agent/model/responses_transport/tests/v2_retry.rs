//! V2 retries exercise the actual owned HTTP/SSE collector with synthetic data.
use super::*;
use research_agent_core::{context::compaction_strategy::*, resilience::RetryPolicy};

fn status(code: u16) -> String {
    format!("HTTP/1.1 {code} Synthetic\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
}
fn reply(items: &[Value], completed: bool) -> String {
    let body = compaction_tests::events(items, completed);
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}
fn policy(attempts: u8) -> RetryPolicy {
    RetryPolicy {
        max_attempts: attempts,
        base_delay_ms: 0,
        max_delay_ms: 0,
    }
}

#[test]
fn v2_rejects_invalid_injected_retry_policy() {
    for retry in [
        policy(0),
        RetryPolicy {
            max_attempts: 3,
            base_delay_ms: 2,
            max_delay_ms: 1,
        },
        RetryPolicy {
            max_attempts: 3,
            base_delay_ms: 0,
            max_delay_ms: 30_001,
        },
    ] {
        let request = compaction_tests::compact_request();
        let strategy = ResponsesV2CompactionStrategy::new(
            Arc::new(transport(
                "http://127.0.0.1:1/responses".into(),
                Arc::new(Receipts::default()),
            )),
            request.binding,
        )
        .unwrap();
        assert!(matches!(
            strategy.with_retry_policy(retry),
            Err(ModelError::InvalidRequest)
        ));
    }
}

#[tokio::test]
async fn v2_retries_frozen_request_and_discards_incomplete_compaction_output() {
    let abandoned = json!({"type":"compaction","encrypted_content":"abandoned-state"});
    let expected = compaction_tests::opaque();
    let (url, peer) = sequential_server_with_headers(vec![
        status(503),
        reply(&[abandoned], false),
        reply(std::slice::from_ref(&expected), true),
    ])
    .await;
    let receipts = Arc::new(Receipts::default());
    let request = compaction_tests::compact_request();
    let id = request.operation_id;
    let strategy = ResponsesV2CompactionStrategy::new(
        Arc::new(transport(url, receipts)),
        request.binding.clone(),
    )
    .unwrap()
    .with_retry_policy(policy(3))
    .unwrap();
    let result = strategy.compact(request).await.unwrap();
    assert!(matches!(result.output, CompactionOutput::RemoteOpaque(value) if value == expected));
    let (bodies, headers) = peer.await.unwrap();
    assert_eq!(bodies.len(), 3);
    assert!(bodies.iter().all(|body| body == &bodies[0]));
    assert!(headers.iter().all(|header| {
        header
            .to_ascii_lowercase()
            .contains(&format!("x-client-request-id: {id}"))
    }));
}

#[tokio::test]
async fn v2_caps_retry_budget_at_two_and_returns_the_original_error_category() {
    for (attempts, count) in [(1, 1), (2, 2), (99, 3)] {
        let (url, peer) = sequential_server(vec![status(503); count]).await;
        let request = compaction_tests::compact_request();
        let strategy = ResponsesV2CompactionStrategy::new(
            Arc::new(transport(url, Arc::new(Receipts::default()))),
            request.binding.clone(),
        )
        .unwrap()
        .with_retry_policy(policy(attempts))
        .unwrap();
        assert!(matches!(
            strategy.compact(request).await,
            Err(CompactionFailure::Model(ModelError::UnexpectedStatus(503)))
        ));
        assert_eq!(peer.await.unwrap().len(), count);
    }
}

#[tokio::test]
async fn v2_does_not_retry_authentication_or_invalid_compaction_output() {
    for response in [
        status(401),
        reply(
            &[json!({"type":"message","role":"assistant","content":[]})],
            true,
        ),
    ] {
        let (url, peer) = sequential_server(vec![response]).await;
        let request = compaction_tests::compact_request();
        let strategy = ResponsesV2CompactionStrategy::new(
            Arc::new(transport(url, Arc::new(Receipts::default()))),
            request.binding.clone(),
        )
        .unwrap()
        .with_retry_policy(policy(3))
        .unwrap();
        assert!(matches!(
            strategy.compact(request).await,
            Err(CompactionFailure::Model(
                ModelError::Authentication
                    | ModelError::HttpAuthentication(_)
                    | ModelError::InvalidStream
            )) | Err(CompactionFailure::InvalidOutput)
        ));
        assert_eq!(peer.await.unwrap().len(), 1);
    }
}

#[tokio::test]
async fn dropping_v2_compaction_during_backoff_drops_the_owned_wait() {
    let (url, peer) = sequential_server(vec![status(503)]).await;
    let request = compaction_tests::compact_request();
    let strategy = ResponsesV2CompactionStrategy::new(
        Arc::new(transport(url, Arc::new(Receipts::default()))),
        request.binding.clone(),
    )
    .unwrap()
    .with_retry_policy(RetryPolicy {
        max_attempts: 3,
        base_delay_ms: 30_000,
        max_delay_ms: 30_000,
    })
    .unwrap();
    let mut future = strategy.compact(request);
    tokio::select! {
        result = &mut future => panic!("finished before backoff: {}", result.is_ok()),
        sent = peer => assert_eq!(sent.unwrap().len(), 1),
    }
    assert!(
        timeout(Duration::from_millis(20), &mut future)
            .await
            .is_err()
    );
    drop(future);
}
