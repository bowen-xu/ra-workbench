use super::*;
const HEADER: &str = "x-codex-turn-state";

#[tokio::test]
async fn compact_header_flows_to_sampling_only_within_same_account_and_transport() {
    let reply = |state: &str| {
        let body =
            json!({"output":[{"type":"compaction","encrypted_content":"synthetic"}]}).to_string();
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nx-codex-turn-state: {state}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
    };
    let (url, peer) =
        sequential_server_with_headers(vec![reply("first"), reply("later"), ok(&body())]).await;
    let native = Arc::new(transport(url.clone(), Arc::new(Receipts::default())));
    let mut compact = compaction_tests::compact_request();
    compact.binding.kind =
        research_agent_core::context::compaction_strategy::CompactionKind::RemoteWindow;
    let strategy =
        ResponsesLegacyCompactionStrategy::new(native.clone(), compact.binding.clone()).unwrap();
    use research_agent_core::context::compaction_strategy::CompactionStrategy;
    strategy.compact(compact.clone()).await.unwrap();
    strategy.compact(compact).await.unwrap();
    let mut stream = native.start(request()).await.unwrap();
    while stream.next().await.unwrap().is_some() {}
    let (bodies, headers) = peer.await.unwrap();
    assert!(!headers[0].to_ascii_lowercase().contains(HEADER));
    assert!(
        headers[1]
            .to_ascii_lowercase()
            .contains("x-codex-turn-state: first")
    );
    assert!(
        headers[2]
            .to_ascii_lowercase()
            .contains("x-codex-turn-state: first")
    );
    assert!(
        bodies
            .iter()
            .all(|value| !value.to_string().contains("x-codex-turn-state"))
    );
    let mut request = reqwest::Request::new(reqwest::Method::POST, url.parse().unwrap());
    native.apply_turn_state(&mut request, Uuid::from_u128(1));
    assert_eq!(request.headers()[HEADER], "first");
    assert!(request.headers()[HEADER].is_sensitive());
    let mut another = reqwest::Request::new(reqwest::Method::POST, url.parse().unwrap());
    native.apply_turn_state(&mut another, Uuid::from_u128(2));
    assert!(!another.headers().contains_key(HEADER));
    let fresh = transport(url, Arc::new(Receipts::default()));
    fresh.apply_turn_state(&mut another, Uuid::from_u128(1));
    assert!(!another.headers().contains_key(HEADER));
}
