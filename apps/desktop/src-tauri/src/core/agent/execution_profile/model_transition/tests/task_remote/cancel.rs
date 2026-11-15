//! Cancellation owns the pending HTTP response and never installs partial state.
use super::*;
use crate::agent::tasks::TaskCompletion;
use research_agent_core::{conversation::ConversationId, protocol::EventPayload, runner::RunError};
use std::time::Duration;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::oneshot,
};

#[tokio::test]
async fn cancel_pending_remote_fallback_releases_http_and_preserves_window() {
    for native in [
        NativeCompactionPolicy::ChatGptResponsesV2,
        NativeCompactionPolicy::ChatGptResponsesFullWindow,
    ] {
        exercise(native).await;
    }
}

async fn exercise(native: NativeCompactionPolicy) {
    let mut fixture = Fixture::new().await;
    let old_model = model("old");
    let current_model = model("current");
    let item = json!({"type":"compaction","encrypted_content":"original-window"});
    let failed_old = if native == NativeCompactionPolicy::ChatGptResponsesV2 {
        wire("text/event-stream", [json!({"type":"response.created","response":{"id":"old-failed","model":"gpt-5.6-terra"}}), json!({"type":"error","code":"context_length_exceeded"})].into_iter().map(|e| format!("data: {e}\n\n")).collect())
    } else {
        "HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into()
    };
    let (old_url, old_peer) = server(vec![
        sample("old-response"),
        compact(native, &item),
        failed_old,
    ])
    .await;
    let (current_url, entered, current_peer) = pending_server(native).await;
    let first = fixture
        .record(old_model.clone(), "historical evidence")
        .await;
    let old_transport = fixture.transport(&first, &old_url, native);
    fixture
        .run(
            &first,
            fixture.execution(&first, old_transport.clone(), native),
            false,
        )
        .await;
    let manual = fixture.record(old_model.clone(), "/compact").await;
    fixture
        .run(
            &manual,
            fixture.execution(
                &manual,
                fixture.transport(&manual, &old_url, native),
                native,
            ),
            true,
        )
        .await;
    let original = fixture
        .service
        .active_context_checkpoint(fixture.id)
        .await
        .unwrap()
        .unwrap();
    let next = fixture
        .record(current_model.clone(), "fresh submission")
        .await;
    let mut execution = fixture.execution(
        &next,
        fixture.transport(&next, &current_url, native),
        native,
    );
    let selected = Arc::new(
        PreviousModelStrategy::compose(
            native
                .responses_strategy(old_transport.clone(), old_model.clone())
                .unwrap(),
            old_transport,
            native.binding(current_model.clone()),
            None,
            64,
            true,
        )
        .unwrap(),
    );
    execution.transport = Arc::new(Selected {
        inner: execution.transport.clone(),
        selected,
    });
    let owned = fixture.start(&next, execution, false).await;
    tokio::time::timeout(Duration::from_secs(2), entered)
        .await
        .unwrap()
        .unwrap();
    assert!(
        fixture
            .service
            .tasks()
            .cancel_turn(ConversationId::from_uuid(fixture.id), owned.turn_id)
            .unwrap()
    );
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), owned.task.wait())
            .await
            .unwrap(),
        TaskCompletion::Canceled
    );
    let request = current_peer.await.unwrap();
    assert!(request["input"].as_array().unwrap().contains(&item));
    assert!(!request.to_string().contains("fresh submission"));
    assert_eq!(old_peer.await.unwrap().len(), 3);
    let projection = fixture
        .service
        .open(ConversationId::from_uuid(fixture.id))
        .await
        .unwrap();
    let turn = projection.turn(next.turn_id).unwrap();
    assert_eq!(turn.outcome, Some(TurnOutcome::Canceled));
    assert!(
        turn.attempts
            .values()
            .all(|attempt| attempt.actual_model.is_none())
    );
    assert!(turn.items.values().all(|item| !matches!(
        item.kind,
        research_agent_core::protocol::ItemKind::AssistantMessage
    )));
    let errors: Vec<_> = fixture
        .events(next.turn_id)
        .into_iter()
        .filter_map(|event| match event.payload() {
            EventPayload::ModelRequestFailed { error, .. } => Some(error.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(errors.len(), 3);
    assert_eq!(errors[1], RunError::Model(ModelError::Canceled).to_string());
    assert_eq!(errors[2], errors[1]);
    for reopen in [false, true] {
        if reopen {
            fixture.reopen().await;
        }
        let retained = fixture
            .service
            .active_context_checkpoint(fixture.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            serde_json::to_value(retained).unwrap(),
            serde_json::to_value(&original).unwrap()
        );
    }
}

/// Send a partial response, then require the actual peer to close on cancellation.
async fn pending_server(
    native: NativeCompactionPolicy,
) -> (
    String,
    oneshot::Receiver<()>,
    tokio::task::JoinHandle<Value>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let (entered, receiver) = oneshot::channel();
    let task = tokio::spawn(async move {
        tokio::time::timeout(Duration::from_secs(5), async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = vec![];
            let request = loop {
                let mut buffer = [0; 4096];
                let count = socket.read(&mut buffer).await.unwrap();
                assert!(count > 0);
                bytes.extend_from_slice(&buffer[..count]);
                if let Some(end) = bytes.windows(4).position(|b| b == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&bytes[..end]).to_ascii_lowercase();
                    let size: usize = headers.lines().find_map(|line| line.strip_prefix("content-length: ")).unwrap().parse().unwrap();
                    if bytes.len() >= end + 4 + size {
                        break serde_json::from_slice(&bytes[end + 4..end + 4 + size]).unwrap();
                    }
                }
            };
            let item = json!({"type":"compaction","encrypted_content":"unfinished-window"});
            let (mime, partial) = if native == NativeCompactionPolicy::ChatGptResponsesV2 {
                ("text/event-stream", [json!({"type":"response.created","response":{"id":"pending","model":"gpt-5.6-terra"}}), json!({"type":"response.output_item.done","output_index":0,"item":item})].into_iter().map(|event| format!("data: {event}\n\n")).collect::<String>())
            } else {
                ("application/json", format!("{{\"model\":\"gpt-5.6-terra\",\"output\":[{item}"))
            };
            let response = format!("HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{partial}", partial.len() + 100);
            socket.write_all(response.as_bytes()).await.unwrap();
            entered.send(()).unwrap();
            let mut buffer = [0; 1];
            match socket.read(&mut buffer).await {
                Ok(0) => {},
                Err(error) if matches!(error.kind(), std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::ConnectionAborted) => {},
                other => panic!("canceled HTTP peer did not close: {other:?}"),
            }
            request
        }).await.expect("pending response was not released")
    });
    (url, receiver, task)
}
