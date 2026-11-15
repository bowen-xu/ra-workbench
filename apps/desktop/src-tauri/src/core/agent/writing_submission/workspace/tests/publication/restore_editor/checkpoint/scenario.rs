//! A real partial application with an unsaved CRLF baseline and durable old receipt.
use super::super::*;
use crate::agent::writing_submission::tests::Fixture;
use serde_json::{Value, json};

pub(super) const BEFORE: &str = "未保存 baseline\r\n";
pub(super) const AFTER: &str = "proposal 中文\r\n";
pub(super) struct Scenario {
    pub(super) service: WritingSubmissionService,
    pub(super) scope: ApplicationScope,
    pub(super) owner: EditorVersion,
    pub(super) checkpoint: Value,
}
pub(super) fn observe(owner: &EditorVersion, text: &str) -> EditorObservation {
    EditorObservation {
        path: "main.tex".into(),
        version: owner.clone(),
        text: text.into(),
    }
}
pub(super) fn persist(f: &Fixture, row: &Value) {
    f.store.connection().unwrap().execute("insert into writing_document_checkpoints(manuscript_id,relative_path,revision,checkpoint_json,updated_at) values(?1,'main.tex',1,?2,'now') on conflict(manuscript_id,relative_path) do update set revision=1,checkpoint_json=excluded.checkpoint_json",
        rusqlite::params![f.manuscript.to_string(),row.to_string()]).unwrap();
}
pub(super) async fn partial(f: &Fixture) -> Scenario {
    create(f, true).await
}
pub(super) async fn applying(f: &Fixture) -> Scenario {
    create(f, false).await
}
async fn create(f: &Fixture, reopen: bool) -> Scenario {
    let service = WritingSubmissionService::from_app_store(&f.store);
    let original = EditorVersion {
        session_id: Uuid::new_v4(),
        revision: Uuid::new_v4(),
    };
    let mut input = request(f, Uuid::new_v4());
    input.context.files[0].buffer = Some(crate::agent::writing_context::WritingContextBuffer {
        revision: original.revision,
        text: BEFORE.into(),
    });
    let prepared = service.prepare(&f.models, input).await.unwrap();
    let receipt = WritingSubmissionReceipt {
        conversation_id: f.conversation,
        request_id: prepared.record.request_id,
        manuscript_id: f.manuscript,
    };
    let buffers = || WritingWorkspaceRequest {
        manuscript_id: f.manuscript,
        buffers: vec![EditorBuffer {
            path: "main.tex".into(),
            expected_disk_sha256: f.hash.clone(),
            editor: original.clone(),
            text: BEFORE.into(),
        }],
    };
    let copy = service
        .prepare_workspace(receipt, prepared.record.context_snapshot_id, buffers())
        .await
        .unwrap();
    let root = copy.lease.working_directory().unwrap();
    std::fs::write(root.join("main.tex"), AFTER).unwrap();
    std::fs::write(root.join("a.tex"), "added disk").unwrap();
    std::fs::write(root.join("z.tex"), "pending disk").unwrap();
    drop(copy);
    let review = service
        .review_workspace(receipt, prepared.record.context_snapshot_id)
        .await
        .unwrap();
    let proposal = service
        .publish_workspace_changes(review.binding, review.changes)
        .await
        .unwrap();
    let scope = ApplicationScope {
        conversation: f.conversation,
        manuscript: f.manuscript,
        proposal,
        operation: Uuid::new_v4(),
    };
    service
        .begin_proposal_application(f.conversation, proposal, scope.operation, buffers())
        .await
        .unwrap();
    service
        .apply_disk_proposal_file(scope, "a.tex".into())
        .await
        .unwrap();
    assert!(matches!(
        service
            .prepare_editor_proposal_file(scope, observe(&original, BEFORE))
            .await
            .unwrap(),
        EditorAdmission::Dispatch(_)
    ));
    let receipt_owner = EditorVersion {
        revision: Uuid::new_v4(),
        ..original
    };
    service
        .confirm_editor_proposal_file(scope, observe(&receipt_owner, AFTER))
        .await
        .unwrap();
    if reopen {
        crate::agent::service::AgentSessionService::from_app_store(&f.store).unwrap();
    }
    let checkpoint = json!({"schemaVersion":1,"manuscriptId":f.manuscript,"relativePath":"main.tex","version":2,
        "content":AFTER,"reviewBaseline":BEFORE,"savedSha256":f.hash,"savedVersion":0,"saveIntent":null,
        "operations":[{"operationId":scope.operation,"conversationId":scope.conversation,"requestId":prepared.record.request_id,
            "resultSha256":format!("{:x}",Sha256::digest(AFTER.as_bytes()))}],
        "editorReceipt":{"operationId":scope.operation,"sessionId":receipt_owner.session_id,"revision":receipt_owner.revision,"text":AFTER,"stateVersion":1}});
    persist(f, &checkpoint);
    Scenario {
        service,
        scope,
        owner: EditorVersion {
            session_id: Uuid::new_v4(),
            revision: Uuid::new_v4(),
        },
        checkpoint,
    }
}
pub(super) fn prepare_request(
    s: &Scenario,
    restore: Uuid,
    proof: Value,
    text: &str,
) -> boundary::Request {
    serde_json::from_value(json!({"action":"prepare","scope":{"conversationId":s.scope.conversation,"manuscriptId":s.scope.manuscript,"proposalId":s.scope.proposal,"operationId":s.scope.operation},
        "restoreId":restore,"current":{"path":"main.tex","sessionId":s.owner.session_id,"revision":s.owner.revision,"text":text},"checkpointVersion":proof})).unwrap()
}
