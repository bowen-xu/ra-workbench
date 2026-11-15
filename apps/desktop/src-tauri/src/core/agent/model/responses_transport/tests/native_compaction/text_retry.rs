use super::*;
use rusqlite::OptionalExtension;

#[tokio::test]
async fn native_text_compaction_shrinks_capacity_failure_and_keeps_original_journal() {
    let fixture = Fixture::new().await;
    let failed = format!(
        "data: {}\n\ndata: {}\n\n",
        json!({"type":"response.created","response":{"id":"failed1","model":"fixture-model"}}),
        json!({"type":"error","code":"context_length_exceeded"})
    );
    let (url, peer) =
        sequential_server(vec![ok(&body()), ok(&failed), ok(&body()), ok(&body())]).await;
    let first = fixture.record("old user evidence").await;
    let execution = fixture.execution(&first, fixture.transport(&first, &url));
    assert_eq!(
        fixture
            .start(&first, execution, false)
            .await
            .task
            .wait()
            .await,
        TaskCompletion::Completed
    );
    let compact = fixture.record("/compact").await;
    let transport = fixture.transport(&compact, &url);
    let mut execution = fixture.execution(&compact, transport.clone());
    execution.compaction_strategy = Some(Arc::new(
        research_agent_core::context::text_compaction::TextCompactionStrategy::new(
            transport,
            crate::agent::execution_profile::NativeCompactionPolicy::Text
                .binding(compact.input.model.clone()),
        )
        .unwrap()
        .with_compact_prompt("Frozen CUSTOM COMPACT prompt: preserve evidence anchors".into())
        .unwrap(),
    ));
    let completion = fixture
        .start(&compact, execution, true)
        .await
        .task
        .wait()
        .await;
    let diagnostic: Option<String> = fixture.app.shared_agent_connection().lock().unwrap().query_row(
        "select json_extract(payload_json,'$.payload.error') from agent_items where json_extract(payload_json,'$.payload.type')='modelRequestFailed' order by ordinal desc limit 1", [], |row| row.get(0)
    ).optional().unwrap();
    assert_eq!(completion, TaskCompletion::Completed, "{:?}", diagnostic);
    let checkpoint = fixture
        .service
        .active_context_checkpoint(fixture.id)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        checkpoint.output,
        CompactionOutput::TextSummary(_)
    ));
    // Successful summary output belongs only to the installed context window;
    // it must not advance the ordinary Responses continuation receipt.
    {
        let connection = fixture.app.shared_agent_connection();
        let db = connection.lock().unwrap();
        let accepted = |turn: Uuid| -> u64 {
            db.query_row("select count(*) from agent_items where json_extract(payload_json,'$.payload.turnId')=?1 and json_extract(payload_json,'$.payload.type')='itemAccepted' and json_extract(payload_json,'$.payload.source.providerDataJson') is not null",[turn.to_string()],|row|row.get(0)).unwrap()
        };
        assert_eq!(accepted(first.turn_id), 1);
        assert_eq!(
            accepted(compact.turn_id),
            0,
            "summary candidates must not become ordinary accepted model output"
        );
        assert_eq!(
            db.query_row(
                "select count(*) from agent_responses_checkpoints",
                [],
                |row| row.get::<_, u64>(0)
            )
            .unwrap(),
            0
        );
    }
    let next = fixture.record("continue").await;
    let transport = fixture.transport(&next, &url);
    let mut execution = fixture.execution(&next, transport.clone());
    execution.compaction_strategy = Some(
        crate::agent::execution_profile::NativeCompactionPolicy::Text
            .responses_strategy(transport, next.input.model.clone())
            .unwrap(),
    );
    assert_eq!(
        fixture
            .start(&next, execution, false)
            .await
            .task
            .wait()
            .await,
        TaskCompletion::Completed
    );
    let projection = fixture
        .service
        .open(ConversationId::from_uuid(fixture.id))
        .await
        .unwrap();
    assert!(
        projection
            .turn(first.turn_id)
            .unwrap()
            .items
            .values()
            .any(|item| item.text == "old user evidence")
    );
    let requests = peer.await.unwrap();
    assert!(requests[1].to_string().contains("old user evidence"));
    assert!(!requests[2].to_string().contains("old user evidence"));
    for request in &requests[1..3] {
        assert!(request.to_string().contains("current research authority"));
        assert!(request.to_string().contains("Frozen CUSTOM COMPACT prompt"));
    }
    assert!(
        !requests[3]
            .to_string()
            .contains("Frozen CUSTOM COMPACT prompt")
    );
}
