use super::*;
use research_agent_core::context::compaction_strategy::{CompactionKind, CompactionStrategy};

fn compact_reply() -> String {
    let body =
        json!({"output":[{"type":"compaction","encrypted_content":"synthetic"}]}).to_string();
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}
fn status(code: u16) -> String {
    format!("HTTP/1.1 {code} Synthetic\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
}

#[tokio::test]
async fn transient_http_and_incomplete_body_retry_identical_frozen_input_once() {
    let truncated = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 100\r\nConnection: close\r\n\r\n{".into();
    let (url, peer) =
        sequential_server_with_headers(vec![status(429), status(503), truncated, compact_reply()])
            .await;
    let receipts = Arc::new(Receipts::default());
    let native = Arc::new(transport(url, receipts.clone()));
    let mut request = compaction_tests::compact_request();
    request.binding.kind = CompactionKind::RemoteWindow;
    let operation = request.operation_id;
    let strategy = ResponsesLegacyCompactionStrategy::new(native, request.binding.clone())
        .unwrap()
        .with_retry_policy(LegacyCompactionRetryPolicy::new(3, Duration::ZERO).unwrap());
    strategy.compact(request).await.unwrap();
    let (bodies, headers) = peer.await.unwrap();
    assert_eq!(bodies.len(), 4);
    assert!(bodies.iter().all(|body| body == &bodies[0]));
    assert!(headers.iter().all(|header| {
        header
            .to_ascii_lowercase()
            .contains(&format!("x-client-request-id: {operation}"))
    }));
    assert_eq!(receipts.prepares.load(Ordering::SeqCst), 1);
    assert!(receipts.values.lock().unwrap().is_empty());
}

#[tokio::test]
async fn retry_exhaustion_and_non_transient_errors_do_not_publish_candidates() {
    for (replies, retries) in [
        (vec![status(503), status(503)], 1),
        (vec![status(429)], 0),
        (vec![status(400)], 3),
        (vec![status(401)], 3),
        (vec![status(302)], 3),
        (vec!["HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 1\r\nConnection: close\r\n\r\nx".into()], 3),
    ] {
        let expected = replies.len();
        let (url, peer) = sequential_server(replies).await;
        let receipts = Arc::new(Receipts::default());
        let mut request = compaction_tests::compact_request();
        request.binding.kind = CompactionKind::RemoteWindow;
        let strategy = ResponsesLegacyCompactionStrategy::new(Arc::new(transport(url, receipts.clone())), request.binding.clone()).unwrap()
            .with_retry_policy(LegacyCompactionRetryPolicy::new(retries, Duration::ZERO).unwrap());
        assert!(strategy.compact(request).await.is_err());
        assert_eq!(peer.await.unwrap().len(), expected);
        assert_eq!(receipts.prepares.load(Ordering::SeqCst), 1);
        assert!(receipts.values.lock().unwrap().is_empty());
    }
}

#[test]
fn policy_has_bounded_attempts_and_delay_and_no_v2_budget() {
    assert!(LegacyCompactionRetryPolicy::new(6, Duration::ZERO).is_err());
    assert!(LegacyCompactionRetryPolicy::new(1, Duration::from_secs(31)).is_err());
    let policy = LegacyCompactionRetryPolicy::new(5, Duration::from_secs(10)).unwrap();
    assert_eq!(
        policy.delay(ModelError::Unavailable, 4),
        Some(Duration::from_secs(30))
    );
    assert_eq!(policy.delay(ModelError::Unavailable, 5), None);
    assert_eq!(policy.delay(ModelError::InvalidStream, 0), None);
    for error in [
        ModelError::ServerOverloaded,
        ModelError::InternalServerError,
        ModelError::UnexpectedStatus(503),
    ] {
        assert_eq!(policy.delay(error, 0), Some(Duration::from_secs(10)));
        assert_eq!(policy.delay(error, 5), None);
    }
    for error in [
        ModelError::ContextWindowExceeded,
        ModelError::UsageLimitReached,
    ] {
        assert_eq!(policy.delay(error, 0), None);
    }
    assert_eq!(
        LegacyCompactionRetryPolicy::NONE.delay(ModelError::Unavailable, 0),
        None
    );
}

#[tokio::test]
async fn dropping_candidate_during_backoff_drops_the_owned_retry_wait() {
    let (url, peer) = sequential_server(vec![status(503)]).await;
    let receipts = Arc::new(Receipts::default());
    let mut request = compaction_tests::compact_request();
    request.binding.kind = CompactionKind::RemoteWindow;
    let strategy = ResponsesLegacyCompactionStrategy::new(
        Arc::new(transport(url, receipts.clone())),
        request.binding.clone(),
    )
    .unwrap()
    .with_retry_policy(LegacyCompactionRetryPolicy::new(3, Duration::from_secs(30)).unwrap());
    let mut future = strategy.compact(request);
    tokio::select! {
        result = &mut future => panic!("candidate finished before retry wait: {}", result.is_ok()),
        sent = peer => assert_eq!(sent.unwrap().len(), 1),
    }
    assert!(
        timeout(Duration::from_millis(20), &mut future)
            .await
            .is_err()
    );
    drop(future);
    assert_eq!(receipts.prepares.load(Ordering::SeqCst), 1);
    assert!(receipts.values.lock().unwrap().is_empty());
}
