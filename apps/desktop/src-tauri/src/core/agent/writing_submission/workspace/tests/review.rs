use super::*;

#[tokio::test]
async fn completed_workspace_remains_reviewable_without_restoring_execution_authority() {
    let f = fixture(true);
    let service = WritingSubmissionService::from_app_store(&f.store);
    let prepared = service
        .prepare(&f.models, request(&f, Uuid::new_v4()))
        .await
        .unwrap();
    let receipt = WritingSubmissionReceipt {
        conversation_id: f.conversation,
        request_id: prepared.record.request_id,
        manuscript_id: f.manuscript,
    };
    let context = prepared.record.context_snapshot_id;
    let copy = service
        .prepare_workspace(
            receipt,
            context,
            WritingWorkspaceRequest {
                manuscript_id: f.manuscript,
                buffers: vec![],
            },
        )
        .await
        .unwrap();
    let working = copy.lease.working_directory().unwrap();
    std::fs::write(working.join("main.tex"), b"Revised manuscript").unwrap();
    std::fs::write(working.join("new.tex"), b"New section").unwrap();
    // The review must not race a held lease, even before a process is started.
    assert!(service.review_workspace(receipt, context).await.is_err());
    let binding = copy.binding.clone();
    drop(copy);
    let terminal = research_agent_core::protocol::EventPayload::TurnFinished {
        turn_id: binding.turn_id,
        outcome: research_agent_core::protocol::TurnOutcome::Completed,
    };
    f.store.connection().unwrap().execute(
        "insert into agent_items(conversation_id,ordinal,schema_version,payload_json) values(?1,0,1,?2)",
        params![f.conversation.to_string(), serde_json::json!({"payload":terminal}).to_string()],
    ).unwrap();
    let review = service.review_workspace(receipt, context).await.unwrap();
    assert_eq!(review.binding, binding);
    assert_eq!(review.changes.len(), 2);
    let main = review
        .changes
        .iter()
        .find(|change| change.path == "main.tex")
        .unwrap();
    assert_eq!(
        main.kind,
        research_agent_core::workspace::FileChangeKind::Modified
    );
    assert_eq!(
        main.source.as_ref().unwrap().revision,
        format!("sha256:{}", f.hash)
    );
    let content = service
        .review_workspace_change(
            binding.clone(),
            main.path.clone(),
            main.before.clone(),
            main.after.clone(),
        )
        .await
        .unwrap();
    assert_eq!(
        content.before,
        Some(crate::agent::workspace::ReviewContent::Text(
            "Frozen manuscript".into()
        ))
    );
    assert_eq!(
        content.after,
        Some(crate::agent::workspace::ReviewContent::Text(
            "Revised manuscript".into()
        ))
    );
    assert!(
        execution_workspaces::authorize(&f.store.connection().unwrap(), &binding, &review.baseline)
            .is_err()
    );
    assert!(service.restore_workspace(receipt, context).await.is_err());
    assert!(
        service
            .review_workspace(receipt, Uuid::new_v4())
            .await
            .is_err()
    );
    assert_eq!(
        std::fs::read(
            f.store
                .manuscript_directory()
                .join(f.manuscript.to_string())
                .join("source/main.tex")
        )
        .unwrap(),
        b"Frozen manuscript"
    );
    // A proposal inventory is a fresh snapshot, never a cached permission to apply.
    std::fs::remove_file(working.join("main.tex")).unwrap();
    assert!(
        service
            .review_workspace_change(
                binding.clone(),
                main.path.clone(),
                main.before.clone(),
                main.after.clone()
            )
            .await
            .is_err()
    );
    let revised = service.review_workspace(receipt, context).await.unwrap();
    assert_eq!(
        revised
            .changes
            .iter()
            .find(|change| change.path == "main.tex")
            .unwrap()
            .kind,
        research_agent_core::workspace::FileChangeKind::Deleted
    );
    f.store
        .connection()
        .unwrap()
        .execute("delete from agent_conversation_bindings", [])
        .unwrap();
    assert!(service.review_workspace(receipt, context).await.is_err());
}
