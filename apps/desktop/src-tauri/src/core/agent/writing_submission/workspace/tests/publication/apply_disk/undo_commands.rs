use super::*;
use crate::agent::writing_application_commands::undo;
use serde_json::{Value, json};

#[tokio::test]
async fn undo_boundary_preserves_both_identities_and_rejects_invalid_commands() {
    let f = fixture_with_objects();
    let service = WritingSubmissionService::from_app_store(&f.store);
    let original = existing(&f, false).await;
    service
        .apply_disk_proposal_file(original, "main.tex".into())
        .await
        .unwrap();
    let undo_id = Uuid::new_v4();
    let scope = json!({"conversationId":original.conversation,"manuscriptId":original.manuscript,
        "proposalId":original.proposal,"operationId":original.operation});
    let request = |command: Value| json!({"scope":scope,"undoId":undo_id,"command":command});
    let run = |value: Value| async {
        undo::execute(&f.store, serde_json::from_value(value).unwrap()).await
    };
    let missing =
        serde_json::to_value(run(request(json!({"action":"read"}))).await.unwrap()).unwrap();
    assert_eq!(missing["result"], Value::Null);
    for invalid in [
        json!({"action":"read","extra":true}),
        json!({"action":"rollbackDisk"}),
        json!({"action":"prepare"}),
        json!({"action":"resume"}),
    ] {
        assert!(serde_json::from_value::<undo::Request>(request(invalid)).is_err());
    }
    for id in [Uuid::nil(), original.operation] {
        let mut value = request(json!({"action":"read"}));
        value["undoId"] = json!(id);
        assert!(run(value).await.is_err());
    }
    let mut foreign = request(json!({"action":"read"}));
    foreign["scope"]["conversationId"] = json!(Uuid::new_v4());
    assert!(run(foreign).await.is_err());
    let prepared = serde_json::to_value(
        run(request(json!({"action":"prepare","buffers":[]})))
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(prepared["schemaVersion"], 1);
    assert_eq!(prepared["originalScope"], scope);
    assert_eq!(prepared["undoId"], json!(undo_id));
    assert_eq!(prepared["result"]["scope"]["operationId"], json!(undo_id));
    assert_eq!(prepared["result"]["status"], "prepared");
    assert!(
        run(request(json!({"action":"applyDisk","path":"../main.tex"})))
            .await
            .is_err()
    );
    assert!(run(request(json!({"action":"prepareEditor","current":{"path":"main.tex","sessionId":Uuid::nil(),"revision":Uuid::new_v4(),"text":"replacement"}}))).await.is_err());
    let started = serde_json::to_value(
        run(request(json!({"action":"start","buffers":[]})))
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(started["result"]["status"], "applying");
    run(request(json!({"action":"defer"}))).await.unwrap();
    let resumed = serde_json::to_value(
        run(request(json!({"action":"resume","buffers":[]})))
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(resumed["result"]["status"], "applying");
    assert_eq!(resumed["originalScope"], scope);
    assert_eq!(resumed["result"]["scope"]["operationId"], json!(undo_id));
    let finished = serde_json::to_value(
        run(request(json!({"action":"applyDisk","path":"main.tex"})))
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(finished["result"]["status"], "applied");
    assert_eq!(finished["result"]["files"][0]["phase"], "acknowledged");
    assert_eq!(finished["originalScope"], scope);
    assert_eq!(
        std::fs::read(source(&f).join("main.tex")).unwrap(),
        b"Frozen manuscript"
    );
    assert!(
        run(request(json!({"action":"cancelPending"})))
            .await
            .is_err()
    );
    assert_eq!(
        serde_json::to_value(
            run(request(json!({"action":"reconcileDisk","path":"main.tex"})))
                .await
                .unwrap()
        )
        .unwrap(),
        finished
    );
}

#[tokio::test]
async fn lookup_recovers_latest_durable_undo_without_execution_or_scope_leaks() {
    let f = fixture_with_objects();
    let service = WritingSubmissionService::from_app_store(&f.store);
    let original = existing(&f, false).await;
    service
        .apply_disk_proposal_file(original, "main.tex".into())
        .await
        .unwrap();
    let scope = || crate::agent::writing_application_commands::Scope {
        conversation_id: original.conversation,
        manuscript_id: original.manuscript,
        proposal_id: original.proposal,
        operation_id: original.operation,
    };
    assert!(undo::find(&f.store, scope()).await.unwrap().is_none());
    let mut foreign = scope();
    foreign.proposal_id = Uuid::new_v4();
    assert!(undo::find(&f.store, foreign).await.is_err());
    let request = || WritingWorkspaceRequest {
        manuscript_id: f.manuscript,
        buffers: vec![],
    };
    // UUID lexical order is deliberately opposite to insertion order.
    let first = Uuid::parse_str("ffffffff-ffff-4fff-8fff-ffffffffffff").unwrap();
    let second = Uuid::parse_str("11111111-1111-4111-8111-111111111111").unwrap();
    service
        .prepare_proposal_undo(original, first, request())
        .await
        .unwrap();
    let found =
        serde_json::to_value(undo::find(&f.store, scope()).await.unwrap().unwrap()).unwrap();
    assert_eq!(found["undoId"], json!(first));
    assert_eq!(found["result"]["status"], "prepared");
    service
        .cancel_pending_proposal_undo(original, first)
        .await
        .unwrap();
    service
        .prepare_proposal_undo(original, second, request())
        .await
        .unwrap();
    let _new_service =
        crate::agent::service::AgentSessionService::from_app_store(&f.store).unwrap();
    let found =
        serde_json::to_value(undo::find(&f.store, scope()).await.unwrap().unwrap()).unwrap();
    assert_eq!(found["undoId"], json!(second));
    assert_eq!(found["result"]["scope"]["operationId"], json!(second));
    assert_eq!(found["result"]["status"], "prepared");
    assert_eq!(
        std::fs::read(source(&f).join("main.tex")).unwrap(),
        b"replacement"
    );
    assert_eq!(
        service
            .read_proposal_undo(original, first)
            .await
            .unwrap()
            .unwrap()
            .application
            .status,
        ApplicationStatus::RolledBack
    );
}
