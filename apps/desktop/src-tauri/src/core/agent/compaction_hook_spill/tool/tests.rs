use super::super::native::tests::{complete, fixture, prepare};
use super::*;
use crate::agent::{
    composed_tools::RejectingToolExecutor,
    execution_profile::{AgentExecutionMode, execution_profile},
    host_execution,
    service::AgentSessionService,
};

fn profile(record: &SubmissionRecord) -> AgentExecutionProfile {
    let model = ModelIdentity {
        provider_id: "deepseek".into(),
        model_id: "deepseek-v4-flash".into(),
        revision: None,
    };
    let mut profile = execution_profile(&model, AgentExecutionMode::DeepSeekNonThinking).unwrap();
    // Native synthetic transport profile, not a provider capability claim.
    profile.model = record.input.model.clone();
    profile
}
async fn tools(
    store: &AppStore,
    record: &SubmissionRecord,
    profile: &AgentExecutionProfile,
) -> OwnedToolRegistration {
    host_execution::freeze_grant_sql(&store.connection().unwrap(), record).unwrap();
    let sessions = Arc::new(AgentSessionService::from_app_store(store).unwrap());
    let host = host_execution::registration_with_hooks(
        store.clone(),
        record.clone(),
        sessions,
        OwnedToolRegistration::new(vec![], Arc::new(RejectingToolExecutor)),
    )
    .await
    .unwrap();
    registration(
        store.clone(),
        record.clone(),
        profile,
        host.workspace,
        host.tools,
    )
    .unwrap()
}
fn call(id: Uuid, offset: usize, limit: usize) -> ToolCall {
    ToolCall {
        namespace: None,
        raw_arguments_json: None,
        call_id: "native-read".into(),
        name: NAME.into(),
        arguments: serde_json::json!({"artifactId":id,"offset":offset,"limit":limit}),
    }
}
#[tokio::test]
async fn owned_read_tool_requires_completed_context_and_reads_full_unicode_without_a_lease() {
    let (_root, store, record, request, _path) = fixture();
    let text = "完整证据🙂\n".repeat(1000);
    let (id, output) = prepare(&store, &request, &text).await;
    let registered = tools(&store, &record, &profile(&record)).await;
    let (definitions, executor) = registered.into_parts();
    assert!(definitions.iter().any(|d| d.name == NAME));
    assert!(
        executor
            .authorize_and_execute(call(id, 0, 64))
            .await
            .is_err()
    );
    complete(&store, &request, &output);
    let mut offset = 0;
    let mut full = String::new();
    loop {
        let result = executor
            .authorize_and_execute(call(id, offset, 4096))
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(
            !result
                .content
                .contains(store.content_directory().to_str().unwrap())
        );
        let value: serde_json::Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(value["totalBytes"].as_u64().unwrap(), text.len() as u64);
        assert_eq!(
            value["sha256"].as_str().unwrap(),
            crate::agent::workspace::file_version(text.as_bytes(), false).sha256
        );
        let slice = value["text"].as_str().unwrap();
        assert!(slice.len() <= 4096);
        full.push_str(slice);
        executor
            .result_committed(call(id, offset, 4096))
            .await
            .unwrap();
        match value["nextOffset"].as_u64() {
            Some(next) => {
                assert!(next as usize > offset);
                offset = next as usize;
            }
            None => break,
        }
    }
    assert_eq!(full, text);
    assert!(
        !store
            .agent_workspace_directory()
            .join(record.turn_id.to_string())
            .join("work")
            .exists()
    );
    assert_eq!(
        store
            .connection()
            .unwrap()
            .query_row(
                "select count(*) from agent_host_execution_workspaces",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
}
#[tokio::test]
async fn read_tool_uses_live_frozen_host_grant_and_rejects_forged_scope() {
    let (_root, store, record, request, _) = fixture();
    let (id, output) = prepare(&store, &request, &"evidence".repeat(1000)).await;
    complete(&store, &request, &output);
    let (_, executor) = tools(&store, &record, &profile(&record)).await.into_parts();
    assert!(
        executor
            .authorize_and_execute(call(id, 0, 64))
            .await
            .is_ok()
    );
    for key in ["model", "path", "conversationId", "through", "windowId"] {
        let mut wrong = call(id, 0, 64);
        wrong.arguments[key] = serde_json::json!("forged");
        assert!(executor.validate(&wrong).is_err());
        assert!(executor.authorize_and_execute(wrong).await.is_err());
    }
    for limit in [0, 3, 65537] {
        assert!(executor.validate(&call(id, 0, limit)).is_err());
    }
    assert!(executor.validate(&call(Uuid::nil(), 0, 64)).is_err());
    store
        .connection()
        .unwrap()
        .execute("delete from agent_host_shell_grants", [])
        .unwrap();
    assert!(
        executor
            .authorize_and_execute(call(id, 0, 64))
            .await
            .is_err()
    );
    // A newly approved revision cannot silently expand a frozen task's scope.
    store.connection().unwrap().execute("insert into agent_host_shell_grants(conversation_id,host_kind,resource_id,revision) select conversation_id,resource_kind,resource_id,'new-revision' from agent_conversation_bindings where conversation_id=?1",[record.conversation_id.to_string()]).unwrap();
    assert!(
        executor
            .authorize_and_execute(call(id, 0, 64))
            .await
            .is_err()
    );
}
#[tokio::test]
async fn completed_turn_cannot_reuse_its_owned_output_reader() {
    use research_agent_core::{
        conversation::ConversationId,
        protocol::{AgentEvent, EventPayload, TurnOutcome},
    };
    let (_root, store, record, request, _) = fixture();
    let (id, output) = prepare(&store, &request, &"evidence".repeat(1000)).await;
    complete(&store, &request, &output);
    let (_, executor) = tools(&store, &record, &profile(&record)).await.into_parts();
    assert!(
        executor
            .authorize_and_execute(call(id, 0, 64))
            .await
            .is_ok()
    );
    let event = AgentEvent::new(
        ConversationId::from_uuid(record.conversation_id),
        2,
        2,
        record.request_id,
        EventPayload::TurnFinished {
            turn_id: record.turn_id,
            outcome: TurnOutcome::Completed,
        },
    )
    .unwrap();
    crate::agent::repository::append_batch(
        &mut store.connection().unwrap(),
        event.conversation_id(),
        Uuid::new_v4(),
        1,
        &[event],
    )
    .unwrap();
    assert!(
        executor
            .authorize_and_execute(call(id, 0, 64))
            .await
            .is_err()
    );
}
#[tokio::test]
async fn read_tool_bounds_serialized_control_text_by_the_frozen_output_profile() {
    let (_root, store, record, request, _) = fixture();
    let (id, output) = prepare(&store, &request, &"\u{0001}".repeat(2000)).await;
    complete(&store, &request, &output);
    let mut profile = profile(&record);
    profile.limits.tool_output_bytes = 2048;
    let (definitions, executor) = tools(&store, &record, &profile).await.into_parts();
    let definition = definitions.iter().find(|d| d.name == NAME).unwrap();
    assert_eq!(definition.parameters["properties"]["limit"]["maximum"], 256);
    assert!(executor.validate(&call(id, 0, 257)).is_err());
    let result = executor
        .authorize_and_execute(call(id, 0, 256))
        .await
        .unwrap();
    assert!(result.content.len() <= profile.limits.tool_output_bytes);
    let mut default = call(id, 0, 256);
    default.arguments.as_object_mut().unwrap().remove("limit");
    let result = executor.authorize_and_execute(default).await.unwrap();
    assert!(result.content.len() <= profile.limits.tool_output_bytes);
}
#[test]
fn absent_workspace_does_not_register_a_reader_or_grant_access() {
    let (_root, store, record, _, _) = fixture();
    let registered = registration(
        store,
        record.clone(),
        &profile(&record),
        None,
        OwnedToolRegistration::new(vec![], Arc::new(RejectingToolExecutor)),
    )
    .unwrap();
    assert!(registered.into_parts().0.is_empty());
}

#[tokio::test]
async fn original_host_authority_checks_each_resource_without_acquiring_a_lease() {
    for kind in ["paper", "note", "readingNote"] {
        let (_root, store, record) = crate::agent::host_execution::tests::fixture(kind);
        crate::storage::migrate(&mut store.connection().unwrap()).unwrap();
        host_execution::freeze_grant_sql(&store.connection().unwrap(), &record).unwrap();
        let sessions = Arc::new(AgentSessionService::from_app_store(&store).unwrap());
        let registration = host_execution::registration_with_hooks(
            store.clone(),
            record.clone(),
            sessions,
            OwnedToolRegistration::new(vec![], Arc::new(RejectingToolExecutor)),
        )
        .await
        .unwrap();
        let authority = registration.workspace.unwrap();
        authority.authorize_artifact_read(&record).unwrap();
        assert!(
            !store
                .agent_workspace_directory()
                .join(record.turn_id.to_string())
                .join("work")
                .exists()
        );
        store
            .connection()
            .unwrap()
            .execute("delete from agent_host_shell_grants", [])
            .unwrap();
        assert!(authority.authorize_artifact_read(&record).is_err());
    }
}
