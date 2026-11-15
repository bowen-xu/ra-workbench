use super::*;

pub(super) fn legacy_execution(
    fixture: &Fixture,
    record: &SubmissionRecord,
    url: &str,
) -> AgentExecution {
    let transport = fixture.transport(record, url);
    let strategy =
        crate::agent::execution_profile::NativeCompactionPolicy::ChatGptResponsesFullWindow
            .responses_strategy(transport.clone(), record.input.model.clone())
            .unwrap();
    let mut execution = fixture.execution(record, transport);
    execution.compaction_strategy = Some(strategy);
    execution
}
pub(super) fn full_window(text: &str) -> Vec<Value> {
    vec![
        json!({"type":"message","role":"user","content":[{"type":"input_text","text":text}]}),
        json!({"type":"compaction","encrypted_content":text,"provider_extension":3}),
    ]
}
pub(super) fn json_response(items: &[Value]) -> String {
    let body = json!({"output":items,"usage":{"input_tokens":9,"output_tokens":2}}).to_string();
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

#[tokio::test]
async fn native_full_window_installs_restores_and_recompacts_without_response_identity() {
    let fixture = Fixture::new().await;
    let first = full_window("first window");
    let second = full_window("second window");
    let mut returned_first = first.clone();
    returned_first.extend([
        json!({"type":"message","role":"developer","content":[{"type":"input_text","text":"stale authority"}]}),
        json!({"type":"message","role":"user","content":[{"type":"input_text","text":"<skill>stale skill</skill>"}]}),
        json!({"type":"function_call_output","call_id":"stale-call","output":"old output"}),
        json!({"type":"reasoning","summary":[],"encrypted_content":"old reasoning"}),
        json!({"type":"future_provider_item","value":"future data"}),
    ]);
    let (url, peer) = sequential_server(vec![
        ok(&body()),
        json_response(&returned_first),
        ok(&body()),
        json_response(&second),
        ok(&body()),
    ])
    .await;
    let mut windows = vec![];
    for (text, compact) in [
        ("old history", false),
        ("/compact", true),
        ("fresh", false),
        ("/compact", true),
        ("after second", false),
    ] {
        let record = fixture.record(text).await;
        let execution = legacy_execution(&fixture, &record, &url);
        assert_eq!(
            fixture
                .start(&record, execution, compact)
                .await
                .task
                .wait()
                .await,
            TaskCompletion::Completed
        );
        if compact {
            let checkpoint = fixture
                .service
                .active_context_checkpoint(fixture.id)
                .await
                .unwrap()
                .unwrap();
            assert!(checkpoint.retained.is_empty());
            assert!(matches!(
                checkpoint.output,
                CompactionOutput::RemoteWindow(_)
            ));
            let history = fixture
                .service
                .open(ConversationId::from_uuid(fixture.id))
                .await
                .unwrap();
            let turn = history.turn(record.turn_id).unwrap();
            assert_eq!(turn.items.len(), 1);
            assert!(turn.items[&checkpoint.window_id].completed);
            assert_eq!(turn.attempts.len(), 1);
            assert!(
                turn.attempts
                    .values()
                    .all(|attempt| attempt.response_id.is_none() && attempt.outcome.is_some())
            );
            windows.push(checkpoint);
        }
    }
    let sent = peer.await.unwrap();
    assert_eq!(
        &sent[2]["input"].as_array().unwrap()[1..1 + first.len()],
        first.as_slice()
    );
    assert!(!sent[2].to_string().contains("old history"));
    for removed in [
        "stale authority",
        "stale skill",
        "stale-call",
        "old reasoning",
        "future data",
    ] {
        assert!(!sent[2].to_string().contains(removed));
    }
    assert!(sent[3]["input"].as_array().unwrap().contains(&first[1]));
    assert_eq!(
        &sent[4]["input"].as_array().unwrap()[1..1 + second.len()],
        second.as_slice()
    );
    assert!(!sent[4]["input"].as_array().unwrap().contains(&first[1]));
    assert_eq!(windows[1].previous_window_id, Some(windows[0].window_id));
    assert!(
        sent.iter()
            .all(|value| !value.to_string().contains("/compact"))
    );
}

#[tokio::test]
async fn restored_media_are_normalized_only_on_wire_through_repeated_sampling_and_compaction() {
    for vision in [false, true] {
        let fixture = Fixture::new().await;
        let image = json!({"type":"input_image","image_url":"data:image/png;base64,AQID"});
        let original = vec![
            json!({"type":"message","role":"user","content":[{"type":"input_audio","audio_url":"fixture-audio"},image.clone()]}),
            json!({"type":"compaction","encrypted_content":"media window","provider_extension":3}),
        ];
        let replacement = full_window("replacement window");
        let (url, peer) = sequential_server(vec![
            json_response(&original),
            ok(&body()),
            ok(&body()),
            json_response(&replacement),
            ok(&body()),
        ])
        .await;
        for (index, (text, compact)) in [
            ("/compact", true),
            ("first follow-up", false),
            ("second follow-up", false),
            ("/compact", true),
            ("after replacement", false),
        ]
        .into_iter()
        .enumerate()
        {
            let record = fixture.record(text).await;
            let mut native = Arc::try_unwrap(fixture.transport(&record, &url))
                .unwrap_or_else(|_| panic!("fixture transport must have one owner"));
            native.config.capabilities.vision = vision;
            let native = Arc::new(native);
            let strategy =
                crate::agent::execution_profile::NativeCompactionPolicy::ChatGptResponsesFullWindow
                    .responses_strategy(native.clone(), record.input.model.clone())
                    .unwrap();
            let mut execution = fixture.execution(&record, native);
            execution.compaction_strategy = Some(strategy);
            let completion = fixture
                .start(&record, execution, compact)
                .await
                .task
                .wait()
                .await;
            if completion != TaskCompletion::Completed {
                let history = fixture
                    .service
                    .open(ConversationId::from_uuid(fixture.id))
                    .await
                    .unwrap();
                eprintln!("failed turn: {:?}", history.turn(record.turn_id));
                let events = fixture
                    .service
                    .read_since(ConversationId::from_uuid(fixture.id), 0, 100)
                    .await
                    .unwrap();
                eprintln!("events: {:?}", events.items);
            }
            assert_eq!(
                completion,
                TaskCompletion::Completed,
                "vision={vision}, request={index}"
            );
            let checkpoint = fixture
                .service
                .active_context_checkpoint(fixture.id)
                .await
                .unwrap()
                .unwrap();
            let CompactionOutput::RemoteWindow(installed) = checkpoint.output else {
                panic!("legacy checkpoint must contain a replacement window");
            };
            assert_eq!(
                installed,
                if index < 3 { &original } else { &replacement }.clone(),
                "sampling normalization must not rewrite durable provider evidence"
            );
        }
        let sent = peer.await.unwrap();
        for prompt in &sent[1..4] {
            let input = prompt["input"].as_array().unwrap();
            let media = input
                .iter()
                .find(|item| {
                    item["content"][0]["text"]
                        == "audio content omitted because you do not support audio input"
                })
                .unwrap();
            if vision {
                assert_eq!(media["content"][1], image);
            } else {
                assert_eq!(
                    media["content"][1]["text"],
                    "image content omitted because you do not support image input"
                );
            }
            assert!(!prompt.to_string().contains("fixture-audio"));
            assert!(input.contains(&original[1]));
        }
        let final_input = sent[4]["input"].as_array().unwrap();
        assert!(final_input.contains(&replacement[1]));
        assert!(!final_input.contains(&original[1]));
    }
}

#[tokio::test]
async fn full_window_canceled_after_candidate_does_not_install() {
    let fixture = Fixture::new().await;
    let (url, peer) = server(json_response(&full_window("candidate")), false).await;
    let record = fixture.record("/compact").await;
    let mut execution = legacy_execution(&fixture, &record, &url);
    let held = Arc::new(HeldValidation {
        inner: fixture.transport(&record, &url),
        entered: tokio::sync::Notify::new(),
        dropped: Arc::new(AtomicBool::new(false)),
    });
    execution.transport = held.clone();
    let owned = fixture.start(&record, execution, true).await;
    timeout(Duration::from_secs(3), held.entered.notified())
        .await
        .unwrap();
    fixture
        .service
        .tasks()
        .cancel_turn(ConversationId::from_uuid(fixture.id), owned.turn_id)
        .unwrap();
    assert_eq!(
        timeout(Duration::from_secs(3), owned.task.wait())
            .await
            .unwrap(),
        TaskCompletion::Canceled
    );
    assert!(held.dropped.load(Ordering::SeqCst));
    assert!(
        fixture
            .service
            .active_context_checkpoint(fixture.id)
            .await
            .unwrap()
            .is_none()
    );
    peer.await.unwrap();
}
