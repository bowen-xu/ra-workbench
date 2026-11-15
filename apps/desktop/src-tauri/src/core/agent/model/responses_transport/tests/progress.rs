use super::*;

#[tokio::test]
async fn completed_item_precedes_a_later_failure_in_the_same_http_chunk() {
    let payload: String = [
        json!({"type":"response.created","response":{"id":"r1","model":"fixture-model"}}),
        json!({"type":"response.output_item.done","item":{"type":"function_call","id":"fc1","call_id":"call1","name":"read","arguments":"{}"}}),
        json!({"type":"response.failed","response":{"id":"r1","error":{"code":"server_error","message":"fixture"}}}),
    ].into_iter().map(|event| format!("data: {event}\n\n")).collect();
    let (url, peer) = server(ok(&payload), false).await;
    let sink = Arc::new(Receipts::default());
    let transport = transport(url, sink.clone());
    let mut stream = transport.start(request()).await.unwrap();
    let mut accepted_call = false;
    loop {
        match stream.next().await {
            Ok(Some(ModelStreamEvent::OutputItemDone(item))) => {
                assert!(
                    matches!(&item.normalized,Some(research_agent_core::model::ModelOutputItem::ToolCall(call)) if call.call_id=="call1")
                );
                accepted_call = true;
            }
            Ok(Some(_)) => {}
            Err(_) => break,
            Ok(None) => panic!("failed response must remain a failure"),
        }
    }
    assert!(
        accepted_call,
        "the next frame cannot retract a completed item"
    );
    assert!(sink.values.lock().unwrap().is_empty());
    peer.await.unwrap();
}

#[tokio::test]
async fn additive_metadata_survives_chatgpt_and_platform_http_streams() {
    let item = json!({"type":"message","id":"m1","role":"assistant","content":[{"type":"output_text","text":"Hello"}]});
    let payload: String = [
        json!({"type":"response.created","response":{"id":"r1","model":"fixture-model"}}),
        json!({"type":"response.model_verification","status":"progress"}),
        json!({"type":"response.output_text.delta","item_id":"m1","delta":"Hello"}),
        json!({"type":"response.output_text.annotation.added","item_id":"m1","annotation":{"type":"fixture"}}),
        json!({"type":"response.output_item.done","output_index":0,"item":item}),
        json!({"type":"response.future_progress","status":"completed"}),
        json!({"type":"response.completed","response":{"id":"r1","model":"fixture-model","status":"completed","output":[item]}}),
    ].into_iter().map(|event| format!("data: {event}\n\n")).collect();
    for kind in [
        ResponsesEndpointKind::ChatGpt,
        ResponsesEndpointKind::Platform,
    ] {
        let (url, peer) = server(ok(&payload), true).await;
        let sink = Arc::new(Receipts::default());
        let mut transport = transport(url, sink.clone());
        transport.kind = kind;
        let mut stream = transport.start(request()).await.unwrap();
        let mut text = String::new();
        let mut completions = 0;
        let mut canonical = vec![];
        while let Some(event) = stream.next().await.unwrap() {
            match event {
                ModelStreamEvent::TextDelta(delta)
                | ModelStreamEvent::AssistantTextDelta { text: delta, .. } => text.push_str(&delta),
                ModelStreamEvent::Completed { .. } => completions += 1,
                ModelStreamEvent::OutputItemDone(item) => canonical.push(item),
                ModelStreamEvent::ToolCallStarted { .. } => panic!("metadata cannot execute tools"),
                _ => {}
            }
        }
        assert_eq!(text, "Hello");
        assert_eq!(completions, 1);
        assert!(sink.values.lock().unwrap().is_empty());
        assert_eq!(canonical.len(), 1);
        assert_eq!(
            crate::agent::model::responses_native::render(&canonical[0]).unwrap(),
            item
        );
        peer.await.unwrap();
    }
}
