use super::*;
use research_agent_core::text_patch::backend::{TargetStatus, TextBackend, TextChange};

#[tokio::test]
async fn builtin_patch_waits_for_exact_approval_and_rechecks_revision() {
    let f = fixture(true);
    let backend = Arc::new(crate::system_documents::backend::Backend {
        store: f.tools.store.clone(),
        record: f.tools.record.clone(),
        cancellation: f.tools.cancellation.clone(),
    });
    let path = "source://agents/prompts/structured-task.md";
    assert!(backend.read(path, false, false).await.is_ok());
    let before = backend.read(path, false, true).await.unwrap();
    let after = format!("{}\nApproved user rule.\n", before.text);
    let changes = vec![TextChange {
        path: path.into(),
        destination: None,
        before: Some(before.clone()),
        after: Some(after.clone()),
        edits: vec![],
    }];
    let writing = {
        let backend = backend.clone();
        tokio::spawn(async move { backend.commit("builtin-patch", changes).await })
    };
    let approvals = f.tools.store.system_document_approvals();
    let pending = loop {
        let pending = approvals.pending(f.tools.record.conversation_id).unwrap();
        if !pending.is_empty() {
            break pending;
        }
        tokio::task::yield_now().await;
    };
    assert_eq!(
        crate::system_documents::read(
            f.tools.store.agent_configuration_directory(),
            "structured-task"
        )
        .unwrap()
        .content,
        before.text
    );
    approvals
        .decide(
            f.tools.record.conversation_id,
            Uuid::parse_str(&pending[0].id).unwrap(),
            true,
        )
        .unwrap();
    assert_eq!(
        writing.await.unwrap().unwrap()[0].status,
        TargetStatus::Applied
    );
    let saved = crate::system_documents::read(
        f.tools.store.agent_configuration_directory(),
        "structured-task",
    )
    .unwrap();
    assert_eq!(saved.content, after);
    assert_eq!(saved.baseline, before.text);
    assert!(backend.query("builtin-patch").await.unwrap().is_some());
    let read = backend.read(path, false, true).await.unwrap();
    let changes = vec![TextChange {
        path: path.into(),
        destination: None,
        before: Some(read.clone()),
        after: Some(format!("{}\nSecond\n", read.text)),
        edits: vec![],
    }];
    let writing = {
        let backend = backend.clone();
        tokio::spawn(async move { backend.commit("builtin-patch-2", changes).await })
    };
    let pending = loop {
        let pending = approvals.pending(f.tools.record.conversation_id).unwrap();
        if !pending.is_empty() {
            break pending;
        }
        tokio::task::yield_now().await;
    };
    crate::system_documents::write(
        f.tools.store.agent_configuration_directory(),
        "structured-task",
        &saved.revision,
        &saved.content,
        &saved.content,
    )
    .unwrap();
    approvals
        .decide(
            f.tools.record.conversation_id,
            Uuid::parse_str(&pending[0].id).unwrap(),
            true,
        )
        .unwrap();
    assert!(writing.await.unwrap().is_err());
    assert_eq!(
        crate::system_documents::read(
            f.tools.store.agent_configuration_directory(),
            "structured-task"
        )
        .unwrap()
        .content,
        after
    );
    assert!(backend.query("builtin-patch-2").await.unwrap().is_none());
}

#[tokio::test]
async fn canceling_builtin_patch_retires_approval_and_blocks_late_write() {
    let f = fixture(true);
    let backend = Arc::new(crate::system_documents::backend::Backend {
        store: f.tools.store.clone(),
        record: f.tools.record.clone(),
        cancellation: f.tools.cancellation.clone(),
    });
    let path = "source://agents/prompts/structured-task.md";
    let before = backend.read(path, false, true).await.unwrap();
    let changes = vec![TextChange {
        path: path.into(),
        destination: None,
        before: Some(before.clone()),
        after: Some(format!("{}\nCanceled change", before.text)),
        edits: vec![],
    }];
    let writing = {
        let backend = backend.clone();
        tokio::spawn(async move { backend.commit("cancel-built-in", changes).await })
    };
    let approvals = f.tools.store.system_document_approvals();
    let pending = loop {
        let requests = approvals.pending(f.tools.record.conversation_id).unwrap();
        if let Some(request) = requests.first() {
            break Uuid::parse_str(&request.id).unwrap();
        }
        tokio::task::yield_now().await;
    };
    f.tools
        .cancel_pending(ToolCall {
            namespace: None,
            raw_arguments_json: None,
            call_id: "cancel-built-in".into(),
            name: PATCH.into(),
            arguments: json!({}),
        })
        .await
        .unwrap();
    assert!(
        tokio::time::timeout(std::time::Duration::from_secs(2), writing)
            .await
            .unwrap()
            .unwrap()
            .is_err()
    );
    assert!(
        approvals
            .pending(f.tools.record.conversation_id)
            .unwrap()
            .is_empty()
    );
    assert!(
        approvals
            .decide(f.tools.record.conversation_id, pending, true)
            .is_err()
    );
    assert_eq!(
        backend.read(path, false, false).await.unwrap().text,
        before.text
    );
    assert!(backend.query("cancel-built-in").await.unwrap().is_none());
}

#[tokio::test]
async fn builtin_patch_declined_receipt_preserves_reason_and_replays_without_writing() {
    let f = fixture(true);
    let document = crate::system_documents::read(
        f.tools.store.agent_configuration_directory(),
        "structured-task",
    )
    .unwrap();
    let target = "source://agents/prompts/structured-task.md";
    let input = patch(
        target,
        document.content.trim_end(),
        &format!("{}\nProposed addition.", document.content.trim_end()),
    );
    let approvals = f.tools.store.system_document_approvals();
    let decline = async {
        loop {
            let pending = approvals.pending(f.tools.record.conversation_id).unwrap();
            if let Some(request) = pending.first() {
                approvals
                    .decide(
                        f.tools.record.conversation_id,
                        Uuid::parse_str(&request.id).unwrap(),
                        false,
                    )
                    .unwrap();
                break;
            }
            tokio::task::yield_now().await;
        }
    };
    let applying = f.tools.apply("declined-receipt", &input);
    tokio::pin!(applying);
    let result = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        tokio::select! {
            result = &mut applying => result,
            () = decline => applying.await,
        }
    })
    .await
    .unwrap();
    let result = result.unwrap();
    assert_eq!(result["status"], "declined", "{result}");
    assert!(result.get("fileDiffs").is_none());
    let reason: Value =
        serde_json::from_str(result["targets"][0]["message"].as_str().unwrap()).unwrap();
    assert_eq!(reason["code"], "edit_declined");
    assert!(reason["operationId"].is_string());
    assert_eq!(
        f.tools.apply("declined-receipt", &input).await.unwrap(),
        result
    );
    assert_eq!(
        crate::system_documents::read(
            f.tools.store.agent_configuration_directory(),
            "structured-task"
        )
        .unwrap()
        .content,
        document.content
    );
}
