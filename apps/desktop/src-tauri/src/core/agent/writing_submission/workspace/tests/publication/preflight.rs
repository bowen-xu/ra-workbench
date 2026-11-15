use super::*;

#[tokio::test]
async fn preflight_checks_live_disk_and_pending_status_without_changing_sources() {
    let f = fixture_with_objects();
    let service = WritingSubmissionService::from_app_store(&f.store);
    let review = changed(&f).await;
    let id = service
        .publish_workspace_changes(review.binding.clone(), review.changes.clone())
        .await
        .unwrap();
    let request = || WritingWorkspaceRequest {
        manuscript_id: f.manuscript,
        buffers: vec![],
    };
    let checked = service
        .preflight_proposal(f.conversation, id, request())
        .await
        .unwrap();
    assert_eq!(checked.changes, review.changes);
    let source = f
        .store
        .manuscript_directory()
        .join(f.manuscript.to_string())
        .join("source/main.tex");
    assert_eq!(std::fs::read(&source).unwrap(), b"Frozen manuscript");
    assert!(
        service
            .preflight_proposal(Uuid::new_v4(), id, request())
            .await
            .is_err()
    );
    std::fs::write(&source, b"Concurrent edit").unwrap();
    assert!(
        service
            .preflight_proposal(f.conversation, id, request())
            .await
            .is_err()
    );
    std::fs::write(&source, b"Frozen manuscript").unwrap();
    service
        .reject_proposal(f.conversation, f.manuscript, id)
        .await
        .unwrap();
    assert!(
        service
            .preflight_proposal(f.conversation, id, request())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn preflight_does_not_treat_an_open_editor_as_the_disk_baseline() {
    let f = fixture_with_objects();
    let service = WritingSubmissionService::from_app_store(&f.store);
    let review = changed(&f).await;
    let id = service
        .publish_workspace_changes(review.binding, review.changes)
        .await
        .unwrap();
    let request = WritingWorkspaceRequest {
        manuscript_id: f.manuscript,
        buffers: vec![crate::agent::writing_workspace::EditorBuffer {
            path: "main.tex".into(),
            expected_disk_sha256: format!("{:x}", Sha256::digest(b"Frozen manuscript")),
            editor: research_agent_core::workspace::EditorVersion {
                session_id: Uuid::new_v4(),
                revision: Uuid::new_v4(),
            },
            text: "New unsaved edit".into(),
        }],
    };
    assert!(
        service
            .preflight_proposal(f.conversation, id, request)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn preflight_rejects_corrupt_proposal_objects_even_when_metadata_still_matches() {
    let f = fixture_with_objects();
    let service = WritingSubmissionService::from_app_store(&f.store);
    let review = changed(&f).await;
    let sha = review.changes[0].after.as_ref().unwrap().sha256.clone();
    let id = service
        .publish_workspace_changes(review.binding, review.changes)
        .await
        .unwrap();
    let path: String = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "select relative_path from content_objects where sha256=?1",
            [sha],
            |r| r.get(0),
        )
        .unwrap();
    std::fs::write(f.store.content_directory().join(path), b"corrupted").unwrap();
    assert!(
        service
            .preflight_proposal(
                f.conversation,
                id,
                WritingWorkspaceRequest {
                    manuscript_id: f.manuscript,
                    buffers: vec![]
                }
            )
            .await
            .is_err()
    );
    assert_eq!(
        service
            .review_proposal(f.conversation, f.manuscript, id)
            .await
            .unwrap()
            .status,
        crate::agent::repository::change_sets::ChangeSetStatus::Pending
    );
}

#[tokio::test]
async fn captured_buffer_must_still_match_identity_revision_and_bytes() {
    use crate::agent::writing_workspace::EditorBuffer;
    use research_agent_core::workspace::EditorVersion;
    let f = fixture_with_objects();
    let service = WritingSubmissionService::from_app_store(&f.store);
    let editor = EditorVersion {
        session_id: Uuid::new_v4(),
        revision: Uuid::new_v4(),
    };
    let mut input = request(&f, Uuid::new_v4());
    input.context.files[0].buffer = Some(crate::agent::writing_context::WritingContextBuffer {
        revision: editor.revision,
        text: "unsaved baseline".into(),
    });
    let prepared = service.prepare(&f.models, input).await.unwrap();
    let receipt = WritingSubmissionReceipt {
        conversation_id: f.conversation,
        request_id: prepared.record.request_id,
        manuscript_id: f.manuscript,
    };
    let make = |editor: EditorVersion, text: &str| WritingWorkspaceRequest {
        manuscript_id: f.manuscript,
        buffers: vec![EditorBuffer {
            path: "main.tex".into(),
            expected_disk_sha256: format!("{:x}", Sha256::digest(b"Frozen manuscript")),
            editor,
            text: text.into(),
        }],
    };
    let copy = service
        .prepare_workspace(
            receipt,
            prepared.record.context_snapshot_id,
            make(editor.clone(), "unsaved baseline"),
        )
        .await
        .unwrap();
    std::fs::write(
        copy.lease.working_directory().unwrap().join("main.tex"),
        b"proposal after",
    )
    .unwrap();
    drop(copy);
    let review = service
        .review_workspace(receipt, prepared.record.context_snapshot_id)
        .await
        .unwrap();
    let id = service
        .publish_workspace_changes(review.binding, review.changes)
        .await
        .unwrap();
    assert!(
        service
            .preflight_proposal(f.conversation, id, make(editor.clone(), "unsaved baseline"))
            .await
            .is_ok()
    );
    assert!(
        service
            .preflight_proposal(
                f.conversation,
                id,
                WritingWorkspaceRequest {
                    manuscript_id: f.manuscript,
                    buffers: vec![]
                }
            )
            .await
            .is_err()
    );
    assert!(
        service
            .preflight_proposal(f.conversation, id, make(editor.clone(), "changed bytes"))
            .await
            .is_err()
    );
    assert!(
        service
            .preflight_proposal(
                f.conversation,
                id,
                make(
                    EditorVersion {
                        revision: Uuid::new_v4(),
                        ..editor.clone()
                    },
                    "unsaved baseline"
                )
            )
            .await
            .is_err()
    );
    assert!(
        service
            .preflight_proposal(
                f.conversation,
                id,
                make(
                    EditorVersion {
                        session_id: Uuid::new_v4(),
                        ..editor.clone()
                    },
                    "unsaved baseline"
                )
            )
            .await
            .is_err()
    );
    let operation = Uuid::new_v4();
    service
        .begin_proposal_application(
            f.conversation,
            id,
            operation,
            make(editor.clone(), "unsaved baseline"),
        )
        .await
        .unwrap();
    let saved:(String,String,String)=f.store.connection().unwrap().query_row(
        "select target_kind,expected_editor_session,expected_editor_revision from agent_proposal_application_files where application_id=?1",
        [operation.to_string()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert_eq!(
        saved,
        (
            "editor".into(),
            editor.session_id.to_string(),
            editor.revision.to_string()
        )
    );
    use crate::agent::repository::proposal_applications::{ApplicationPhase, ApplicationTarget};
    let application = service
        .read_proposal_application(f.conversation, f.manuscript, id, operation)
        .await
        .unwrap();
    assert_eq!(
        application.files[0].target,
        ApplicationTarget::Editor {
            expected: editor.clone(),
            result_revision: None
        }
    );
    let result_revision = Uuid::new_v4();
    {
        use crate::agent::repository::{change_sets, proposal_applications as applications};
        let mut db = f.store.connection().unwrap();
        let tx = db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .unwrap();
        let scope = applications::ApplicationScope {
            conversation: f.conversation,
            manuscript: f.manuscript,
            proposal: id,
            operation,
        };
        let frozen = change_sets::read_in(&tx, f.conversation, f.manuscript, id).unwrap();
        let mut receipt = applications::FileReceipt {
            path: frozen.changes[0].path.clone(),
            after: frozen.changes[0].after.clone(),
            editor: Some(editor.clone()),
        };
        applications::dispatch_in(&tx, scope, &receipt.path).unwrap();
        assert!(applications::acknowledge_in(&tx, scope, &receipt).is_err());
        receipt.editor = Some(EditorVersion {
            session_id: Uuid::new_v4(),
            revision: result_revision,
        });
        assert!(applications::acknowledge_in(&tx, scope, &receipt).is_err());
        receipt.editor = Some(EditorVersion {
            session_id: editor.session_id,
            revision: result_revision,
        });
        applications::acknowledge_in(&tx, scope, &receipt).unwrap();
        applications::acknowledge_in(&tx, scope, &receipt).unwrap();
        receipt.editor = Some(EditorVersion {
            session_id: editor.session_id,
            revision: Uuid::new_v4(),
        });
        assert!(applications::acknowledge_in(&tx, scope, &receipt).is_err());
        tx.commit().unwrap();
    }
    let acknowledged = service
        .read_proposal_application(f.conversation, f.manuscript, id, operation)
        .await
        .unwrap();
    assert_eq!(acknowledged.files[0].phase, ApplicationPhase::Acknowledged);
    assert_eq!(
        acknowledged.files[0].target,
        ApplicationTarget::Editor {
            expected: editor.clone(),
            result_revision: Some(result_revision)
        }
    );
    {
        use crate::agent::repository::proposal_applications as applications;
        use applications::{
            EditorRestorationAdmission, begin_editor_restoration_in, read_editor_restoration_in,
        };
        let scope = applications::ApplicationScope {
            conversation: f.conversation,
            manuscript: f.manuscript,
            proposal: id,
            operation,
        };
        let expected = EditorVersion {
            session_id: editor.session_id,
            revision: result_revision,
        };
        let restore = Uuid::new_v4();
        let mut db = f.store.connection().unwrap();
        let mut tx = db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .unwrap();
        assert!(begin_editor_restoration_in(&tx, scope, "main.tex", restore, &expected).is_err());
        applications::require_recovery_in(&mut tx, scope).unwrap();
        for invalid in [Uuid::nil(), operation] {
            assert!(
                begin_editor_restoration_in(&tx, scope, "main.tex", invalid, &expected).is_err()
            );
        }
        assert!(begin_editor_restoration_in(&tx, scope, "main.tex", restore, &editor).is_err());
        assert!(matches!(
            begin_editor_restoration_in(&tx, scope, "main.tex", restore, &expected).unwrap(),
            EditorRestorationAdmission::Created(_)
        ));
        assert!(matches!(
            begin_editor_restoration_in(&tx, scope, "main.tex", restore, &expected).unwrap(),
            EditorRestorationAdmission::Existing(_)
        ));
        assert!(
            begin_editor_restoration_in(&tx, scope, "main.tex", Uuid::new_v4(), &expected).is_err()
        );
        assert!(
            read_editor_restoration_in(
                &tx,
                applications::ApplicationScope {
                    conversation: Uuid::new_v4(),
                    ..scope
                },
                "main.tex"
            )
            .is_err()
        );
        assert!(
            tx.execute(
                "update agent_editor_restorations set expected_revision=?1",
                [Uuid::new_v4().to_string()]
            )
            .is_err()
        );
        tx.commit().unwrap();
        let stored = read_editor_restoration_in(&db, scope, "main.tex")
            .unwrap()
            .unwrap();
        assert_eq!(stored.id, restore);
        assert_eq!(stored.expected, expected);
        assert_eq!(stored.result_revision, None);
        let directory = tempfile::tempdir().unwrap();
        let backup = directory.path().join("restoration.sqlite3");
        db.execute("vacuum into ?1", [backup.to_str().unwrap()])
            .unwrap();
        let reopened = rusqlite::Connection::open(backup).unwrap();
        assert_eq!(
            read_editor_restoration_in(&reopened, scope, "main.tex").unwrap(),
            Some(stored)
        );
    }
    // SQL length guards cannot establish a real editor identity.
    for invalid in [Uuid::nil().to_string(), "x".repeat(36)] {
        f.store.connection().unwrap().execute("update agent_proposal_application_files set result_editor_revision=?2 where application_id=?1", rusqlite::params![operation.to_string(), invalid]).unwrap();
        assert!(
            service
                .read_proposal_application(f.conversation, f.manuscript, id, operation)
                .await
                .is_err()
        );
    }
}
