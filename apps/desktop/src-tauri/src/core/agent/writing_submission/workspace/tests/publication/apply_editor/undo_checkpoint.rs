//! Resume trusts durable current checkpoint CAS, never inferred or rewritten receipts.
use super::*;
use crate::agent::repository::{
    proposal_applications::{ApplicationPhase, ApplicationTarget},
    proposal_undos,
};
use crate::agent::writing_submission::{
    tests::Fixture,
    workspace::{UndoCheckpointVersion, UndoEditorAdmission},
};
use serde_json::{Value, json};

pub(super) struct Scenario {
    pub(super) service: WritingSubmissionService,
    pub(super) scope: ApplicationScope,
    pub(super) undo: Uuid,
    pub(super) pending: EditorVersion,
    pub(super) request: WritingWorkspaceRequest,
    pub(super) proofs: Vec<UndoCheckpointVersion>,
    pub(super) checkpoints: Vec<Value>,
}
fn observe(path: &str, version: &EditorVersion, text: &str) -> EditorObservation {
    EditorObservation {
        path: path.into(),
        version: version.clone(),
        text: text.into(),
    }
}
pub(super) fn copy_request(request: &WritingWorkspaceRequest) -> WritingWorkspaceRequest {
    WritingWorkspaceRequest {
        manuscript_id: request.manuscript_id,
        buffers: request
            .buffers
            .iter()
            .map(|buffer| EditorBuffer {
                path: buffer.path.clone(),
                expected_disk_sha256: buffer.expected_disk_sha256.clone(),
                editor: buffer.editor.clone(),
                text: buffer.text.clone(),
            })
            .collect(),
    }
}
fn unchanged(actual: &proposal_undos::StoredUndo, frozen: &proposal_undos::StoredUndo) {
    assert_eq!(actual.original_operation, frozen.original_operation);
    assert_eq!(actual.application, frozen.application);
    assert_eq!(actual.changes, frozen.changes);
}
pub(super) fn persist(f: &Fixture, rows: &[Value]) {
    let db = f.store.connection().unwrap();
    for row in rows {
        db.execute("insert into writing_document_checkpoints(manuscript_id,relative_path,revision,checkpoint_json,updated_at)
            values(?1,?2,1,?3,'now') on conflict(manuscript_id,relative_path) do update set revision=1,checkpoint_json=excluded.checkpoint_json",
            rusqlite::params![f.manuscript.to_string(), row["relativePath"].as_str().unwrap(), row.to_string()]).unwrap();
    }
}
pub(super) async fn partial(f: &Fixture) -> Scenario {
    let (service, scope, undo, applied, second) =
        undo_rebind::prepared_with_second(f, false, true).await;
    let pending = second.unwrap();
    let live = |main: EditorVersion, text: &str, next: EditorVersion| WritingWorkspaceRequest {
        manuscript_id: f.manuscript,
        buffers: vec![
            EditorBuffer {
                path: "main.tex".into(),
                expected_disk_sha256: f.hash.clone(),
                editor: main,
                text: text.into(),
            },
            EditorBuffer {
                path: "next.tex".into(),
                expected_disk_sha256: f.hash.clone(),
                editor: next,
                text: undo_rebind::AFTER.into(),
            },
        ],
    };
    service
        .start_proposal_undo(
            scope,
            undo,
            live(applied.clone(), undo_rebind::AFTER, pending.clone()),
        )
        .await
        .unwrap();
    let UndoEditorAdmission::Dispatch(edit) = service
        .prepare_editor_undo(
            scope,
            undo,
            observe("main.tex", &applied, undo_rebind::AFTER),
        )
        .await
        .unwrap()
    else {
        panic!("dispatch");
    };
    let receipt = EditorVersion {
        revision: Uuid::new_v4(),
        ..applied
    };
    service
        .confirm_editor_undo(scope, undo, observe("main.tex", &receipt, &edit.text))
        .await
        .unwrap();
    // Use the real startup scan, as a new process would retire an applying journal.
    crate::agent::service::AgentSessionService::from_app_store(&f.store).unwrap();
    let request_id: String = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "select request_id from agent_change_sets where id=?1",
            [scope.proposal.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    let source = |operation: Uuid, text: &str| json!({"operationId":operation,"conversationId":scope.conversation,"requestId":request_id,"resultSha256":format!("{:x}",Sha256::digest(text.as_bytes()))});
    // Native effects/receipts above are real; seed only the reliable host checkpoints.
    // Version 3 may include baseline-only input after dispatch version 2.
    let checkpoints = vec![
        json!({"schemaVersion":1,"manuscriptId":f.manuscript,"relativePath":"main.tex","version":3,"content":edit.text,"reviewBaseline":edit.text,
            "savedSha256":f.hash,"savedVersion":0,"saveIntent":null,"operations":[source(scope.operation,undo_rebind::AFTER),source(undo,&edit.text)],
            "editorReceipt":{"operationId":undo,"sessionId":receipt.session_id,"revision":receipt.revision,"text":edit.text,"stateVersion":2}}),
        json!({"schemaVersion":1,"manuscriptId":f.manuscript,"relativePath":"next.tex","version":3,"content":undo_rebind::AFTER,"reviewBaseline":"Frozen manuscript",
            "savedSha256":f.hash,"savedVersion":0,"saveIntent":null,"operations":[source(scope.operation,undo_rebind::AFTER)],
            "editorReceipt":{"operationId":scope.operation,"sessionId":pending.session_id,"revision":pending.revision,"text":undo_rebind::AFTER,"stateVersion":1}}),
    ];
    persist(f, &checkpoints);
    let current = || EditorVersion {
        session_id: Uuid::new_v4(),
        revision: Uuid::new_v4(),
    };
    Scenario {
        service,
        scope,
        undo,
        pending,
        request: live(current(), "Frozen manuscript", current()),
        proofs: ["main.tex", "next.tex"]
            .into_iter()
            .map(|path| UndoCheckpointVersion {
                path: path.into(),
                revision: 1,
                state_version: 3,
            })
            .collect(),
        checkpoints,
    }
}

#[tokio::test]
async fn partially_confirmed_undo_resumes_rebuilt_pending_owner_without_replaying_ack() {
    let f = fixture_with_objects();
    let s = partial(&f).await;
    let frozen = s
        .service
        .read_proposal_undo(s.scope, s.undo)
        .await
        .unwrap()
        .unwrap();
    let resumed = s
        .service
        .resume_proposal_undo_with_checkpoints(
            s.scope,
            s.undo,
            copy_request(&s.request),
            s.proofs.clone(),
        )
        .await
        .unwrap();
    assert_eq!(resumed.changes, frozen.changes);
    assert_eq!(resumed.application.files[0], frozen.application.files[0]);
    assert_eq!(resumed.application.status, ApplicationStatus::Applying);
    assert_eq!(
        resumed.application.files[1].target,
        ApplicationTarget::Editor {
            expected: s.request.buffers[1].editor.clone(),
            result_revision: None
        }
    );
    // Even in Applying, the already acknowledged target is a read-only receipt.
    assert!(matches!(
        s.service
            .prepare_editor_undo(
                s.scope,
                s.undo,
                observe(
                    "main.tex",
                    &s.request.buffers[0].editor,
                    "Frozen manuscript"
                )
            )
            .await
            .unwrap(),
        UndoEditorAdmission::Existing(_)
    ));
    assert!(
        s.service
            .prepare_editor_undo(
                s.scope,
                s.undo,
                observe("next.tex", &s.pending, undo_rebind::AFTER)
            )
            .await
            .is_err()
    );
    let current = &s.request.buffers[1].editor;
    let UndoEditorAdmission::Dispatch(edit) = s
        .service
        .prepare_editor_undo(
            s.scope,
            s.undo,
            observe("next.tex", current, undo_rebind::AFTER),
        )
        .await
        .unwrap()
    else {
        panic!("pending dispatch");
    };
    let result = EditorVersion {
        revision: Uuid::new_v4(),
        ..current.clone()
    };
    s.service
        .confirm_editor_undo(s.scope, s.undo, observe("next.tex", &result, &edit.text))
        .await
        .unwrap();
    let done = s
        .service
        .apply_disk_undo_file(s.scope, s.undo, "z.tex".into())
        .await
        .unwrap();
    assert_eq!(done.application.status, ApplicationStatus::Applied);
    assert_eq!(done.application.files[0], frozen.application.files[0]);
    assert_eq!(
        s.service
            .read_proposal_application(
                s.scope.conversation,
                s.scope.manuscript,
                s.scope.proposal,
                s.scope.operation
            )
            .await
            .unwrap()
            .status,
        ApplicationStatus::Applied
    );
}

#[tokio::test]
async fn checkpoint_conflicts_leave_every_target_and_frozen_receipt_unchanged() {
    let f = fixture_with_objects();
    let s = partial(&f).await;
    let frozen = s
        .service
        .read_proposal_undo(s.scope, s.undo)
        .await
        .unwrap()
        .unwrap();
    let mut bad = s.proofs.clone();
    bad[1].revision += 1;
    let mut version = s.proofs.clone();
    version[0].state_version += 1;
    for proofs in [
        vec![],
        vec![s.proofs[0].clone()],
        vec![s.proofs[1].clone()],
        bad,
        version,
        vec![s.proofs[0].clone(), s.proofs[0].clone()],
    ] {
        assert!(
            s.service
                .resume_proposal_undo_with_checkpoints(
                    s.scope,
                    s.undo,
                    copy_request(&s.request),
                    proofs
                )
                .await
                .is_err()
        );
        unchanged(
            &s.service
                .read_proposal_undo(s.scope, s.undo)
                .await
                .unwrap()
                .unwrap(),
            &frozen,
        );
    }
    for fault in [
        "receipt-session",
        "receipt-revision",
        "source",
        "inverse-source",
        "pending-source",
        "text",
        "intent",
    ] {
        let mut rows = s.checkpoints.clone();
        match fault {
            "receipt-session" => rows[0]["editorReceipt"]["sessionId"] = json!(Uuid::new_v4()),
            "receipt-revision" => rows[0]["editorReceipt"]["revision"] = json!(Uuid::new_v4()),
            "source" => rows[0]["operations"][0]["requestId"] = json!(Uuid::new_v4()),
            "inverse-source" => rows[0]["operations"][1]["conversationId"] = json!(Uuid::new_v4()),
            "pending-source" => rows[1]["operations"][0]["resultSha256"] = json!("a".repeat(64)),
            "text" => rows[0]["content"] = json!("later input"),
            "intent" => {
                rows[1]["saveIntent"] = json!({"operationId":Uuid::new_v4(),"version":3,"expectedSha256":f.hash,"targetSha256":f.hash})
            }
            _ => unreachable!(),
        }
        persist(&f, &rows);
        assert!(
            s.service
                .resume_proposal_undo_with_checkpoints(
                    s.scope,
                    s.undo,
                    copy_request(&s.request),
                    s.proofs.clone()
                )
                .await
                .is_err(),
            "{fault}"
        );
        unchanged(
            &s.service
                .read_proposal_undo(s.scope, s.undo)
                .await
                .unwrap()
                .unwrap(),
            &frozen,
        );
    }
    persist(&f, &s.checkpoints);
    let mut live = copy_request(&s.request);
    live.buffers[1].editor = EditorVersion {
        revision: Uuid::new_v4(),
        ..s.pending.clone()
    };
    assert!(
        s.service
            .resume_proposal_undo_with_checkpoints(s.scope, s.undo, live, s.proofs.clone())
            .await
            .is_err()
    );
    // A failed phase transition must roll back Pending owner renewal too.
    f.store.connection().unwrap().execute_batch("create trigger fail_undo_resume before update of status on agent_proposal_applications when new.status='applying' begin select raise(abort,'fixture phase failure'); end;").unwrap();
    assert!(
        s.service
            .resume_proposal_undo_with_checkpoints(s.scope, s.undo, s.request, s.proofs)
            .await
            .is_err()
    );
    unchanged(
        &s.service
            .read_proposal_undo(s.scope, s.undo)
            .await
            .unwrap()
            .unwrap(),
        &frozen,
    );
    assert_eq!(
        frozen.application.files[0].phase,
        ApplicationPhase::Acknowledged
    );
    let db = f.store.connection().unwrap();
    assert!(db.execute("update agent_proposal_application_files set expected_editor_session=?2 where application_id=?1 and relative_path='main.tex'", rusqlite::params![s.undo.to_string(),Uuid::new_v4().to_string()]).is_err());
    assert!(
        db.execute(
            "update agent_proposal_undo_sources set source_json='{}' where operation_id=?1",
            [s.undo.to_string()]
        )
        .is_err()
    );
    unchanged(
        &proposal_undos::read_in(&db, s.scope, s.undo)
            .unwrap()
            .unwrap(),
        &frozen,
    );
}
