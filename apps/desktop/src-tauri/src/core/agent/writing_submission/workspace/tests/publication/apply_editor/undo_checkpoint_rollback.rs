//! Rebuilt compensation owns fresh CAS without modifying the original Undo receipt.
use super::undo_checkpoint::{copy_request, partial, persist};
use super::*;
use crate::agent::repository::proposal_applications::{ApplicationStatus, ApplicationTarget};
use crate::agent::writing_submission::workspace::EditorRestoreAdmission;
use serde_json::json;

fn copy_observation(value: &EditorObservation) -> EditorObservation {
    EditorObservation {
        path: value.path.clone(),
        version: value.version.clone(),
        text: value.text.clone(),
    }
}

#[tokio::test]
async fn rebuilt_compensation_restores_once_and_rechecks_receipt_after_another_owner_rebuild() {
    let f = fixture_with_objects();
    let mut s = partial(&f).await;
    s.service
        .resume_proposal_undo_with_checkpoints(
            s.scope,
            s.undo,
            copy_request(&s.request),
            s.proofs.clone(),
        )
        .await
        .unwrap();
    // Leave a disk effect to compensate after another editor owner rebuild.
    s.service
        .apply_disk_undo_file(s.scope, s.undo, "z.tex".into())
        .await
        .unwrap();
    s.service
        .defer_proposal_undo(s.scope, s.undo)
        .await
        .unwrap();
    let frozen = s
        .service
        .read_proposal_undo(s.scope, s.undo)
        .await
        .unwrap()
        .unwrap();
    s.service
        .prepare_proposal_undo_rollback_with_checkpoints(
            s.scope,
            s.undo,
            copy_request(&s.request),
            s.proofs.clone(),
        )
        .await
        .unwrap();
    let current = EditorObservation {
        path: "main.tex".into(),
        version: s.request.buffers[0].editor.clone(),
        text: s.request.buffers[0].text.clone(),
    };
    let restore = Uuid::new_v4();
    // Missing/stale proof cannot create an intent in a rebuilt owner.
    for proof in [
        None,
        Some(
            crate::agent::writing_submission::workspace::UndoCheckpointVersion {
                revision: 2,
                ..s.proofs[0].clone()
            },
        ),
    ] {
        assert!(
            s.service
                .prepare_undo_editor_restoration_with_checkpoint(
                    s.scope,
                    s.undo,
                    restore,
                    copy_observation(&current),
                    proof
                )
                .await
                .is_err()
        );
        assert!(
            s.service
                .read_undo_editor_restoration(s.scope, s.undo, "main.tex".into())
                .await
                .unwrap()
                .1
                .is_none()
        );
    }
    let EditorRestoreAdmission::Dispatch(edit) = s
        .service
        .prepare_undo_editor_restoration_with_checkpoint(
            s.scope,
            s.undo,
            restore,
            copy_observation(&current),
            Some(s.proofs[0].clone()),
        )
        .await
        .unwrap()
    else {
        panic!("fresh compensation");
    };
    assert_eq!(edit.expected.version, current.version);
    assert_eq!(edit.text, undo_rebind::AFTER);
    let actual = EditorObservation {
        path: "main.tex".into(),
        version: EditorVersion {
            revision: Uuid::new_v4(),
            ..current.version.clone()
        },
        text: edit.text.clone(),
    };
    s.service
        .confirm_undo_editor_restoration(s.scope, s.undo, restore, copy_observation(&actual))
        .await
        .unwrap();
    let (_, intent) = s
        .service
        .read_undo_editor_restoration(s.scope, s.undo, "main.tex".into())
        .await
        .unwrap();
    let intent = intent.unwrap();
    assert_eq!(intent.expected, current.version);
    assert_eq!(intent.result_revision, Some(actual.version.revision));
    let saved = s
        .service
        .read_proposal_undo(s.scope, s.undo)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        saved.application.files[0].target,
        frozen.application.files[0].target
    );
    assert_eq!(saved.changes, frozen.changes);
    // Persist the real observed compensation receipt, then rebuild both owners.
    let request_id = s.checkpoints[0]["operations"][0]["requestId"].clone();
    let checkpoint = &mut s.checkpoints[0];
    checkpoint["content"] = json!(actual.text);
    checkpoint["reviewBaseline"] = json!(actual.text);
    checkpoint["version"] = json!(4);
    checkpoint["editorReceipt"] = json!({"operationId":restore,"sessionId":actual.version.session_id,"revision":actual.version.revision,"text":actual.text,"stateVersion":4});
    checkpoint["operations"].as_array_mut().unwrap().push(json!({"operationId":restore,"conversationId":s.scope.conversation,"requestId":request_id,"resultSha256":format!("{:x}",Sha256::digest(actual.text.as_bytes()))}));
    persist(&f, &s.checkpoints);
    s.proofs[0].state_version = 4;
    s.request.buffers[0].text = actual.text;
    for buffer in &mut s.request.buffers {
        buffer.editor = EditorVersion {
            session_id: Uuid::new_v4(),
            revision: Uuid::new_v4(),
        };
    }
    // Existing identity is history, never permission to dispatch again.
    assert!(matches!(
        s.service
            .prepare_undo_editor_restoration_with_checkpoint(
                s.scope,
                s.undo,
                restore,
                EditorObservation {
                    path: "main.tex".into(),
                    version: s.request.buffers[0].editor.clone(),
                    text: s.request.buffers[0].text.clone()
                },
                Some(s.proofs[0].clone())
            )
            .await
            .unwrap(),
        EditorRestoreAdmission::Existing { .. }
    ));
    let done = s
        .service
        .rollback_disk_proposal_undo_with_checkpoints(
            s.scope,
            s.undo,
            copy_request(&s.request),
            s.proofs.clone(),
        )
        .await
        .unwrap();
    assert_eq!(done.application.status, ApplicationStatus::RolledBack);
    assert_eq!(
        done.application.files[0].target,
        frozen.application.files[0].target
    );
    assert_eq!(done.changes, frozen.changes);
    assert!(matches!(
        done.application.files[0].target,
        ApplicationTarget::Editor { .. }
    ));
    assert_eq!(
        s.service
            .read_undo_editor_restoration(s.scope, s.undo, "main.tex".into())
            .await
            .unwrap()
            .1
            .unwrap(),
        intent
    );
}

#[tokio::test]
async fn rebuilt_compensation_refuses_later_input_and_forged_receipts_without_creating_intents() {
    let f = fixture_with_objects();
    let s = partial(&f).await;
    let frozen = s
        .service
        .read_proposal_undo(s.scope, s.undo)
        .await
        .unwrap()
        .unwrap();
    for fault in ["source", "receipt", "intent", "version", "text"] {
        let mut rows = s.checkpoints.clone();
        let mut proofs = s.proofs.clone();
        let mut request = copy_request(&s.request);
        match fault {
            "source" => rows[1]["operations"][0]["requestId"] = json!(Uuid::new_v4()),
            "receipt" => rows[0]["editorReceipt"]["sessionId"] = json!(Uuid::new_v4()),
            "intent" => {
                rows[0]["saveIntent"] = json!({"operationId":Uuid::new_v4(),"version":3,"expectedSha256":f.hash,"targetSha256":f.hash})
            }
            "version" => proofs[0].revision += 1,
            "text" => {
                rows[0]["content"] = json!("later input");
                request.buffers[0].text = "later input".into();
            }
            _ => unreachable!(),
        }
        persist(&f, &rows);
        assert!(
            s.service
                .prepare_proposal_undo_rollback_with_checkpoints(s.scope, s.undo, request, proofs)
                .await
                .is_err(),
            "{fault}"
        );
        let current = s
            .service
            .read_proposal_undo(s.scope, s.undo)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(current.application, frozen.application);
        assert_eq!(current.changes, frozen.changes);
        assert!(
            s.service
                .read_undo_editor_restoration(s.scope, s.undo, "main.tex".into())
                .await
                .unwrap()
                .1
                .is_none()
        );
    }
    // SQL also rejects a rebuilt intent without the actual original Undo receipt.
    let db = f.store.connection().unwrap();
    assert!(db.execute("insert into agent_editor_restorations(application_id,relative_path,restore_id,editor_session,expected_revision) values(?1,'main.tex',?2,?3,?4)",rusqlite::params![s.undo.to_string(),Uuid::new_v4().to_string(),Uuid::new_v4().to_string(),Uuid::new_v4().to_string()]).is_err());
}
