//! New live CAS ownership is admitted only for a wholly undispatched inverse.
use super::*;
use crate::agent::repository::proposal_applications::{ApplicationPhase, ApplicationTarget};
use crate::agent::writing_submission::tests::Fixture;
use crate::agent::writing_submission::workspace::UndoEditorAdmission;

pub(super) const AFTER: &str = "proposal 摘要😀\r\n";
fn editor() -> EditorVersion {
    EditorVersion {
        session_id: Uuid::new_v4(),
        revision: Uuid::new_v4(),
    }
}
fn buffers(f: &Fixture, version: &EditorVersion, text: &str) -> WritingWorkspaceRequest {
    WritingWorkspaceRequest {
        manuscript_id: f.manuscript,
        buffers: vec![EditorBuffer {
            path: "main.tex".into(),
            expected_disk_sha256: f.hash.clone(),
            editor: version.clone(),
            text: text.into(),
        }],
    }
}
fn observation(version: &EditorVersion, text: &str) -> EditorObservation {
    EditorObservation {
        path: "main.tex".into(),
        version: version.clone(),
        text: text.into(),
    }
}
async fn prepared(
    f: &Fixture,
    recovery: bool,
) -> (
    WritingSubmissionService,
    ApplicationScope,
    Uuid,
    EditorVersion,
) {
    let (service, scope, operation, applied, _) = prepared_with_second(f, recovery, false).await;
    (service, scope, operation, applied)
}

pub(super) async fn prepared_with_second(
    f: &Fixture,
    recovery: bool,
    second: bool,
) -> (
    WritingSubmissionService,
    ApplicationScope,
    Uuid,
    EditorVersion,
    Option<EditorVersion>,
) {
    let service = WritingSubmissionService::from_app_store(&f.store);
    let version = editor();
    let second_version = second.then(editor);
    if second {
        let path = f
            .store
            .manuscript_directory()
            .join(f.manuscript.to_string())
            .join("source/next.tex");
        std::fs::write(path, "Frozen manuscript").unwrap();
        f.store.connection().unwrap().execute("insert into manuscript_files(manuscript_id,relative_path,file_kind,byte_size,sha256,modified_at) values(?1,'next.tex','tex',17,?2,'now')", rusqlite::params![f.manuscript.to_string(), f.hash]).unwrap();
    }
    let live = |main: &EditorVersion, next: &Option<EditorVersion>, text: &str| {
        let mut request = buffers(f, main, text);
        if let Some(next) = next {
            request.buffers.push(EditorBuffer {
                path: "next.tex".into(),
                editor: next.clone(),
                expected_disk_sha256: f.hash.clone(),
                text: text.into(),
            });
        }
        request
    };
    let mut input = request(f, Uuid::new_v4());
    input.context.files[0].buffer = Some(crate::agent::writing_context::WritingContextBuffer {
        revision: version.revision,
        text: "Frozen manuscript".into(),
    });
    let admission = service.prepare(&f.models, input).await.unwrap();
    let receipt = WritingSubmissionReceipt {
        conversation_id: f.conversation,
        request_id: admission.record.request_id,
        manuscript_id: f.manuscript,
    };
    let copy = service
        .prepare_workspace(
            receipt,
            admission.record.context_snapshot_id,
            live(&version, &second_version, "Frozen manuscript"),
        )
        .await
        .unwrap();
    let path = copy.lease.working_directory().unwrap();
    std::fs::write(path.join("main.tex"), AFTER).unwrap();
    if second {
        std::fs::write(path.join("next.tex"), AFTER).unwrap();
    }
    std::fs::write(path.join("z.tex"), "new disk file").unwrap();
    drop(copy);
    let review = service
        .review_workspace(receipt, admission.record.context_snapshot_id)
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
        .begin_proposal_application(
            f.conversation,
            proposal,
            scope.operation,
            live(&version, &second_version, "Frozen manuscript"),
        )
        .await
        .unwrap();
    service
        .prepare_editor_proposal_file(scope, observation(&version, "Frozen manuscript"))
        .await
        .unwrap();
    let applied = EditorVersion {
        revision: Uuid::new_v4(),
        ..version
    };
    service
        .confirm_editor_proposal_file(scope, observation(&applied, AFTER))
        .await
        .unwrap();
    let second_applied = if let Some(next) = second_version {
        let observe_next = |version: EditorVersion, text: &str| EditorObservation {
            path: "next.tex".into(),
            version,
            text: text.into(),
        };
        service
            .prepare_editor_proposal_file(scope, observe_next(next.clone(), "Frozen manuscript"))
            .await
            .unwrap();
        let applied = EditorVersion {
            revision: Uuid::new_v4(),
            ..next
        };
        service
            .confirm_editor_proposal_file(scope, observe_next(applied.clone(), AFTER))
            .await
            .unwrap();
        Some(applied)
    } else {
        None
    };
    service
        .apply_disk_proposal_file(scope, "z.tex".into())
        .await
        .unwrap();
    let operation = Uuid::new_v4();
    service
        .prepare_proposal_undo(scope, operation, live(&applied, &second_applied, AFTER))
        .await
        .unwrap();
    if recovery {
        service
            .start_proposal_undo(scope, operation, live(&applied, &second_applied, AFTER))
            .await
            .unwrap();
        // The real startup scan retires an applying journal, without inventing receipts.
        crate::agent::service::AgentSessionService::from_app_store(&f.store).unwrap();
    }
    (service, scope, operation, applied, second_applied)
}
async fn admit(
    service: &WritingSubmissionService,
    scope: ApplicationScope,
    id: Uuid,
    request: WritingWorkspaceRequest,
    recovery: bool,
) -> Result<
    crate::agent::repository::proposal_undos::StoredUndo,
    crate::agent::writing_submission::workspace::Error,
> {
    if recovery {
        service.resume_proposal_undo(scope, id, request).await
    } else {
        service.start_proposal_undo(scope, id, request).await
    }
}

#[tokio::test]
async fn wholly_pending_undo_rebinds_new_session_without_rewriting_frozen_sources() {
    for recovery in [false, true] {
        let f = fixture_with_objects();
        let (service, scope, id, departed) = prepared(&f, recovery).await;
        let frozen = service
            .read_proposal_undo(scope, id)
            .await
            .unwrap()
            .unwrap()
            .changes;
        let current = editor();
        let resumed = admit(&service, scope, id, buffers(&f, &current, AFTER), recovery)
            .await
            .unwrap();
        assert_eq!(resumed.changes, frozen);
        assert_eq!(resumed.application.status, ApplicationStatus::Applying);
        assert_eq!(
            resumed.application.files[0].target,
            ApplicationTarget::Editor {
                expected: current.clone(),
                result_revision: None
            }
        );
        // Admission changes live targets only; neither frozen source provenance
        // nor revisions inside a still-live owner can be edited through SQL.
        {
            let db = f.store.connection().unwrap();
            assert!(
                db.execute(
                    "update agent_proposal_undo_sources set source_json='{}' where operation_id=?1",
                    [id.to_string()]
                )
                .is_err()
            );
            assert!(db.execute("update agent_proposal_application_files set expected_editor_revision=?2 where application_id=?1 and relative_path='main.tex'", rusqlite::params![id.to_string(), Uuid::new_v4().to_string()]).is_err());
        }
        assert!(
            service
                .prepare_editor_undo(scope, id, observation(&departed, AFTER))
                .await
                .is_err()
        );
        let UndoEditorAdmission::Dispatch(edit) = service
            .prepare_editor_undo(scope, id, observation(&current, AFTER))
            .await
            .unwrap()
        else {
            panic!("new dispatch");
        };
        assert_eq!(edit.expected.version, current);
        assert_eq!(edit.text, "Frozen manuscript");
        assert!(
            service
                .confirm_editor_undo(scope, id, observation(&departed, &edit.text))
                .await
                .is_err()
        );
        let result = EditorVersion {
            revision: Uuid::new_v4(),
            ..current
        };
        service
            .confirm_editor_undo(scope, id, observation(&result, &edit.text))
            .await
            .unwrap();
        // The deliberately stale ACK retired dispatch authority. Resume checks
        // the renewed owner's confirmed inverse before continuing the disk file.
        service
            .resume_proposal_undo(scope, id, buffers(&f, &result, &edit.text))
            .await
            .unwrap();
        let done = service
            .apply_disk_undo_file(scope, id, "z.tex".into())
            .await
            .unwrap();
        assert_eq!(done.application.status, ApplicationStatus::Applied);
        assert_eq!(done.changes, frozen);
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
            ApplicationStatus::Applied
        );
    }
}

#[tokio::test]
async fn pending_rebind_rejects_live_revision_bytes_disk_and_coverage_conflicts() {
    for recovery in [false, true] {
        let f = fixture_with_objects();
        let (service, scope, id, departed) = prepared(&f, recovery).await;
        let original = service
            .read_proposal_undo(scope, id)
            .await
            .unwrap()
            .unwrap();
        let mut disk = buffers(&f, &editor(), AFTER);
        disk.buffers[0].expected_disk_sha256 = "a".repeat(64);
        for request in [
            buffers(
                &f,
                &EditorVersion {
                    revision: Uuid::new_v4(),
                    ..departed.clone()
                },
                AFTER,
            ),
            buffers(&f, &editor(), "later input"),
            disk,
            WritingWorkspaceRequest {
                manuscript_id: f.manuscript,
                buffers: vec![],
            },
        ] {
            assert!(admit(&service, scope, id, request, recovery).await.is_err());
            let now = service
                .read_proposal_undo(scope, id)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(now.application.files, original.application.files);
            assert_eq!(now.changes, original.changes);
            assert_eq!(now.application.status, original.application.status);
        }
    }
}

#[tokio::test]
async fn dispatched_undo_and_failed_transition_cannot_rebind_ownership() {
    let f = fixture_with_objects();
    let (service, scope, id, departed) = prepared(&f, false).await;
    f.store.connection().unwrap().execute_batch("create trigger fixture_deny_start before update of status on agent_proposal_applications when old.direction='undo' and new.status='applying' begin select raise(abort,'fixture'); end;").unwrap();
    assert!(
        service
            .start_proposal_undo(scope, id, buffers(&f, &editor(), AFTER))
            .await
            .is_err()
    );
    let failed = service
        .read_proposal_undo(scope, id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(failed.application.status, ApplicationStatus::Prepared);
    assert_eq!(
        failed.application.files[0].target,
        ApplicationTarget::Editor {
            expected: departed.clone(),
            result_revision: None
        }
    );
    f.store
        .connection()
        .unwrap()
        .execute_batch("drop trigger fixture_deny_start")
        .unwrap();
    service
        .start_proposal_undo(scope, id, buffers(&f, &departed, AFTER))
        .await
        .unwrap();
    service
        .prepare_editor_undo(scope, id, observation(&departed, AFTER))
        .await
        .unwrap();
    service.defer_proposal_undo(scope, id).await.unwrap();
    assert!(
        service
            .resume_proposal_undo(scope, id, buffers(&f, &editor(), AFTER))
            .await
            .is_err()
    );
    let stopped = service
        .read_proposal_undo(scope, id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        stopped.application.files[0].phase,
        ApplicationPhase::Dispatched
    );
    assert_eq!(
        stopped.application.files[0].target,
        failed.application.files[0].target
    );
}
