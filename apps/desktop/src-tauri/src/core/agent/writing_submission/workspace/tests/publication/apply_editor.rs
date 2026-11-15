use super::*;
use crate::agent::repository::proposal_applications::{ApplicationScope, ApplicationStatus};
use crate::agent::writing_application_commands as boundary;
use crate::agent::writing_submission::workspace::{EditorAdmission, EditorObservation};
use crate::agent::writing_workspace::EditorBuffer;
use research_agent_core::workspace::EditorVersion;
#[path = "apply_editor/deleted_chat.rs"]
mod deleted_chat;
#[path = "apply_editor/undo_checkpoint.rs"]
mod undo_checkpoint;
#[path = "apply_editor/undo_checkpoint_rollback.rs"]
mod undo_checkpoint_rollback;
#[path = "apply_editor/undo_flow.rs"]
mod undo_flow;
#[path = "apply_editor/undo_rebind.rs"]
mod undo_rebind;
#[path = "apply_editor/undo_restoration.rs"]
mod undo_restoration;
#[path = "apply_editor/undo_resume.rs"]
mod undo_resume;
#[path = "apply_editor/undo_saved_rollback.rs"]
mod undo_saved_rollback;

#[tokio::test]
async fn editor_dispatch_validates_bytes_and_receipt_without_saving_disk() {
    exercise_editor_dispatch(ChatState::Attached).await;
}

#[tokio::test]
async fn editor_dispatch_and_undo_keep_receipt_protection_after_chat_detaches() {
    exercise_editor_dispatch(ChatState::Detached).await;
}

enum ChatState {
    Attached,
    Detached,
}

async fn exercise_editor_dispatch(chat: ChatState) {
    let f = fixture_with_objects();
    let service = WritingSubmissionService::from_app_store(&f.store);
    let version = EditorVersion {
        session_id: Uuid::new_v4(),
        revision: Uuid::new_v4(),
    };
    let mut input = request(&f, Uuid::new_v4());
    input.context.files[0].buffer = Some(crate::agent::writing_context::WritingContextBuffer {
        revision: version.revision,
        text: "unsaved baseline".into(),
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
            expected_disk_sha256: format!("{:x}", Sha256::digest(b"Frozen manuscript")),
            editor: version.clone(),
            text: "unsaved baseline".into(),
        }],
    };
    let copy = service
        .prepare_workspace(receipt, prepared.record.context_snapshot_id, buffers())
        .await
        .unwrap();
    std::fs::write(
        copy.lease.working_directory().unwrap().join("main.tex"),
        "proposal 中文\r\n",
    )
    .unwrap();
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
    let wire_scope = serde_json::json!({"conversationId":f.conversation,"manuscriptId":f.manuscript,"proposalId":proposal,"operationId":scope.operation});
    let begin = serde_json::json!({"action":"begin","scope":wire_scope,"buffers":[{"path":"main.tex","expectedDiskSha256":format!("{:x}",Sha256::digest(b"Frozen manuscript")),"sessionId":version.session_id,"revision":version.revision,"text":"unsaved baseline"}]});
    let admitted = serde_json::to_value(
        boundary::execute(&f.store, serde_json::from_value(begin.clone()).unwrap())
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(admitted["kind"], "state");
    assert_eq!(admitted["scope"], wire_scope);
    assert_eq!(
        admitted["files"][0]["target"]["sessionId"],
        version.session_id.to_string()
    );
    assert_eq!(
        admitted["files"][0]["target"]["resultRevision"],
        serde_json::Value::Null
    );
    assert_eq!(
        serde_json::to_value(
            boundary::execute(&f.store, serde_json::from_value(begin).unwrap())
                .await
                .unwrap()
        )
        .unwrap(),
        admitted
    );
    let read = serde_json::json!({"action":"read","scope":wire_scope});
    assert!(
        boundary::execute(
            &f.store,
            serde_json::from_value(serde_json::json!({"action":"rollbackDisk","scope":wire_scope}))
                .unwrap()
        )
        .await
        .is_err()
    );
    assert!(
        boundary::execute(
            &f.store,
            serde_json::from_value(serde_json::json!({
                "action":"applyDisk","scope":wire_scope,"path":"main.tex"
            }))
            .unwrap()
        )
        .await
        .is_err()
    );
    let mut wrong = read.clone();
    wrong["scope"]["conversationId"] = serde_json::json!(Uuid::new_v4());
    assert!(
        boundary::execute(&f.store, serde_json::from_value(wrong).unwrap())
            .await
            .is_err()
    );
    let mut injected = read.clone();
    injected["shell"] = serde_json::json!("no");
    assert!(serde_json::from_value::<boundary::Request>(injected).is_err());
    assert!(
        boundary::execute(
            &f.store,
            serde_json::from_value(
                serde_json::json!({"action":"deferEditor","scope":wire_scope,"path":"main.tex"})
            )
            .unwrap()
        )
        .await
        .is_err()
    );
    let observe = |version: EditorVersion, text: &str| EditorObservation {
        path: "main.tex".into(),
        version,
        text: text.into(),
    };
    if matches!(chat, ChatState::Detached) {
        let db = f.store.connection().unwrap();
        db.execute(
            "update agent_conversations set archived=1 where id=?1",
            [f.conversation.to_string()],
        )
        .unwrap();
        db.execute(
            "delete from agent_conversation_bindings where conversation_id=?1",
            [f.conversation.to_string()],
        )
        .unwrap();
    }
    let result = EditorVersion {
        session_id: version.session_id,
        revision: Uuid::new_v4(),
    };
    // A byte-perfect forged early receipt cannot skip the durable dispatch.
    assert!(
        service
            .confirm_editor_proposal_file(scope, observe(result.clone(), "proposal 中文\r\n"))
            .await
            .is_err()
    );
    assert!(
        service
            .prepare_editor_proposal_file(scope, observe(version.clone(), "later input"))
            .await
            .is_err()
    );
    assert!(
        service
            .prepare_editor_proposal_file(scope, observe(result.clone(), "unsaved baseline"))
            .await
            .is_err()
    );
    let boundary::Response::Edit { schema_version, scope: returned_scope, expected, text } = boundary::execute(&f.store,
        serde_json::from_value(serde_json::json!({"action":"prepareEditor","scope":wire_scope,"current":{"path":"main.tex","sessionId":version.session_id,"revision":version.revision,"text":"unsaved baseline"}})).unwrap()).await.unwrap()
    else {
        panic!("expected one dispatch");
    };
    assert_eq!(schema_version, 1);
    assert_eq!(serde_json::to_value(returned_scope).unwrap(), wire_scope);
    let command = crate::agent::writing_submission::workspace::EditorDispatch {
        operation: scope.operation,
        expected: observe(
            EditorVersion {
                session_id: expected.session_id,
                revision: expected.revision,
            },
            &expected.text,
        ),
        text,
    };
    let deferred = serde_json::to_value(
        boundary::execute(
            &f.store,
            serde_json::from_value(
                serde_json::json!({"action":"deferEditor","scope":wire_scope,"path":"main.tex"}),
            )
            .unwrap(),
        )
        .await
        .unwrap(),
    )
    .unwrap();
    assert_eq!(deferred["status"], "recoveryRequired");
    assert_eq!(command.operation, scope.operation);
    assert_eq!(command.expected.version, version);
    assert_eq!(command.expected.text, "unsaved baseline");
    assert_eq!(command.text, "proposal 中文\r\n");
    assert!(matches!(
        service
            .prepare_editor_proposal_file(scope, observe(version.clone(), "unsaved baseline"))
            .await
            .unwrap(),
        EditorAdmission::Existing(_)
    ));
    assert!(
        service
            .confirm_editor_proposal_file(scope, observe(result.clone(), "proposal 中文\n"))
            .await
            .is_err()
    );
    assert!(
        service
            .confirm_editor_proposal_file(scope, observe(version.clone(), &command.text))
            .await
            .is_err()
    );
    let other = EditorVersion {
        session_id: Uuid::new_v4(),
        revision: result.revision,
    };
    assert!(
        service
            .confirm_editor_proposal_file(scope, observe(other, &command.text))
            .await
            .is_err()
    );
    f.store.connection().unwrap().execute_batch("create trigger fail_editor_ack before update of phase on agent_proposal_application_files when new.phase='acknowledged' begin select raise(abort,'fixture'); end;").unwrap();
    assert!(
        service
            .confirm_editor_proposal_file(scope, observe(result.clone(), &command.text))
            .await
            .is_err()
    );
    f.store
        .connection()
        .unwrap()
        .execute_batch("drop trigger fail_editor_ack;")
        .unwrap();
    assert_eq!(
        service
            .read_proposal_application(
                scope.conversation,
                scope.manuscript,
                scope.proposal,
                scope.operation
            )
            .await
            .unwrap()
            .status,
        ApplicationStatus::RecoveryRequired
    );
    let saved = service
        .confirm_editor_proposal_file(scope, observe(result.clone(), &command.text))
        .await
        .unwrap();
    assert_eq!(saved.status, ApplicationStatus::Applied);
    // Undo uses the current buffer as a new baseline, never the stale disk copy.
    let undo_request = || {
        let mut current = buffers();
        current.buffers[0].editor = result.clone();
        current.buffers[0].text = command.text.clone();
        current
    };
    let inverse = service
        .preview_proposal_undo(scope, undo_request())
        .await
        .unwrap();
    let undo = service
        .prepare_proposal_undo(scope, Uuid::new_v4(), undo_request())
        .await
        .unwrap();
    assert_eq!(undo.changes, inverse);
    assert_eq!(undo.application.status, ApplicationStatus::Prepared);
    assert!(matches!(&undo.application.files[0].target,
        crate::agent::repository::proposal_applications::ApplicationTarget::Editor { expected, result_revision: None } if expected == &result));
    assert_eq!(
        inverse[0].source.as_ref().unwrap().editor.as_ref(),
        Some(&result)
    );
    assert_eq!(
        inverse[0].after.as_ref().unwrap().sha256,
        format!("{:x}", Sha256::digest(b"unsaved baseline"))
    );
    let mut edited = undo_request();
    edited.buffers[0].text.push_str("later input");
    assert!(service.preview_proposal_undo(scope, edited).await.is_err());
    assert!(
        service
            .preview_proposal_undo(
                scope,
                WritingWorkspaceRequest {
                    manuscript_id: f.manuscript,
                    buffers: vec![]
                }
            )
            .await
            .is_err()
    );
    // Completed applications require a distinct undo plan, not interrupted recovery.
    assert!(
        service
            .prepare_editor_restoration(
                scope,
                Uuid::new_v4(),
                observe(result.clone(), &command.text)
            )
            .await
            .is_err()
    );
    let confirmed = serde_json::to_value(boundary::execute(&f.store,
        serde_json::from_value(serde_json::json!({"action":"confirmEditor","scope":wire_scope,"actual":{"path":"main.tex","sessionId":result.session_id,"revision":result.revision,"text":command.text}})).unwrap()).await.unwrap()).unwrap();
    assert_eq!(confirmed["status"], "applied");
    assert_eq!(confirmed["files"][0]["phase"], "acknowledged");
    assert_eq!(
        confirmed["files"][0]["target"]["resultRevision"],
        result.revision.to_string()
    );
    assert_eq!(
        serde_json::to_value(
            boundary::execute(&f.store, serde_json::from_value(read).unwrap())
                .await
                .unwrap()
        )
        .unwrap(),
        confirmed
    );
    assert_eq!(
        service
            .confirm_editor_proposal_file(scope, observe(result.clone(), &command.text))
            .await
            .unwrap(),
        saved
    );
    let source = f
        .store
        .manuscript_directory()
        .join(f.manuscript.to_string())
        .join("source/main.tex");
    assert_eq!(std::fs::read(source).unwrap(), b"Frozen manuscript");
    let hash: String = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "select sha256 from manuscript_files where relative_path='main.tex'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(hash, format!("{:x}", Sha256::digest(b"Frozen manuscript")));
    undo_flow::exercise(
        &f,
        &service,
        scope,
        undo.application.operation_id,
        result,
        &command.text,
    )
    .await;
}
