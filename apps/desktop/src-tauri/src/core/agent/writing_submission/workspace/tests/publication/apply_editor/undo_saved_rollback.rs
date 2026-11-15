use super::*;
use crate::agent::repository::proposal_applications::ApplicationPhase;
use crate::agent::writing_submission::tests::Fixture;
use crate::agent::writing_submission::workspace::EditorRestoreAdmission;
#[cfg(target_os = "macos")]
#[path = "undo_saved_rollback/terminal_discovery.rs"]
mod terminal_discovery;

fn save(f: &Fixture, expected: &str, text: &str) -> String {
    // Seed the exact bytes/index left by a completed formal save. Native
    // compensation is the behavior under test; the UI/save queue is not exercised.
    let path = f
        .store
        .manuscript_directory()
        .join(f.manuscript.to_string())
        .join("source/main.tex");
    assert_eq!(
        format!("{:x}", Sha256::digest(std::fs::read(&path).unwrap())),
        expected
    );
    std::fs::write(path, text).unwrap();
    let hash = format!("{:x}", Sha256::digest(text.as_bytes()));
    assert_eq!(f.store.connection().unwrap().execute(
        "update manuscript_files set sha256=?1,byte_size=?2 where manuscript_id=?3 and relative_path='main.tex'",
        rusqlite::params![hash, text.len() as i64, f.manuscript.to_string()],
    ).unwrap(), 1);
    hash
}
fn current(f: &Fixture, editor: &EditorVersion, hash: &str, text: &str) -> WritingWorkspaceRequest {
    WritingWorkspaceRequest {
        manuscript_id: f.manuscript,
        buffers: vec![EditorBuffer {
            path: "main.tex".into(),
            expected_disk_sha256: hash.into(),
            editor: editor.clone(),
            text: text.into(),
        }],
    }
}
fn observation(editor: &EditorVersion, text: &str) -> EditorObservation {
    EditorObservation {
        path: "main.tex".into(),
        version: editor.clone(),
        text: text.into(),
    }
}
async fn partial(
    f: &Fixture,
) -> (
    WritingSubmissionService,
    ApplicationScope,
    Uuid,
    EditorVersion,
    String,
) {
    let service = WritingSubmissionService::from_app_store(&f.store);
    let editor = EditorVersion {
        session_id: Uuid::new_v4(),
        revision: Uuid::new_v4(),
    };
    let mut input = request(f, Uuid::new_v4());
    input.context.files[0].buffer = Some(crate::agent::writing_context::WritingContextBuffer {
        revision: editor.revision,
        text: "Frozen manuscript".into(),
    });
    let prepared = service.prepare(&f.models, input).await.unwrap();
    let receipt = WritingSubmissionReceipt {
        conversation_id: f.conversation,
        request_id: prepared.record.request_id,
        manuscript_id: f.manuscript,
    };
    let copy = service
        .prepare_workspace(
            receipt,
            prepared.record.context_snapshot_id,
            current(f, &editor, &f.hash, "Frozen manuscript"),
        )
        .await
        .unwrap();
    let copy_source = copy.lease.working_directory().unwrap();
    for (path, text) in [
        ("main.tex", "proposal"),
        ("a.tex", "first disk file"),
        ("z.tex", "pending disk file"),
    ] {
        std::fs::write(copy_source.join(path), text).unwrap();
    }
    drop(copy);
    let review = service
        .review_workspace(receipt, prepared.record.context_snapshot_id)
        .await
        .unwrap();
    let proposal = service
        .publish_workspace_changes(review.binding, review.changes)
        .await
        .unwrap();
    let original = ApplicationScope {
        conversation: f.conversation,
        manuscript: f.manuscript,
        proposal,
        operation: Uuid::new_v4(),
    };
    service
        .begin_proposal_application(
            f.conversation,
            proposal,
            original.operation,
            current(f, &editor, &f.hash, "Frozen manuscript"),
        )
        .await
        .unwrap();
    service
        .prepare_editor_proposal_file(original, observation(&editor, "Frozen manuscript"))
        .await
        .unwrap();
    let applied = EditorVersion {
        revision: Uuid::new_v4(),
        ..editor.clone()
    };
    service
        .confirm_editor_proposal_file(original, observation(&applied, "proposal"))
        .await
        .unwrap();
    for path in ["a.tex", "z.tex"] {
        service
            .apply_disk_proposal_file(original, path.into())
            .await
            .unwrap();
    }
    let hash = save(f, &f.hash, "proposal");
    let undo = Uuid::new_v4();
    service
        .prepare_proposal_undo(original, undo, current(f, &applied, &hash, "proposal"))
        .await
        .unwrap();
    service
        .start_proposal_undo(original, undo, current(f, &applied, &hash, "proposal"))
        .await
        .unwrap();
    service
        .prepare_editor_undo(original, undo, observation(&applied, "proposal"))
        .await
        .unwrap();
    let undone = EditorVersion {
        revision: Uuid::new_v4(),
        ..editor
    };
    service
        .confirm_editor_undo(original, undo, observation(&undone, "Frozen manuscript"))
        .await
        .unwrap();
    service
        .apply_disk_undo_file(original, undo, "a.tex".into())
        .await
        .unwrap();
    service.defer_proposal_undo(original, undo).await.unwrap();
    let hash = save(f, &hash, "Frozen manuscript");
    (service, original, undo, undone, hash)
}
#[tokio::test]
async fn partial_undo_saved_editor_can_compensate_before_and_after_restoration_save() {
    for save_restored in [false, true] {
        let f = fixture_with_objects();
        let (service, original, undo, editor, hash) = partial(&f).await;
        service
            .prepare_proposal_undo_rollback(
                original,
                undo,
                current(&f, &editor, &hash, "Frozen manuscript"),
            )
            .await
            .unwrap();
        let restore_id = Uuid::new_v4();
        let EditorRestoreAdmission::Dispatch(edit) = service
            .prepare_undo_editor_restoration(
                original,
                undo,
                restore_id,
                observation(&editor, "Frozen manuscript"),
            )
            .await
            .unwrap()
        else {
            panic!("expected compensation");
        };
        assert_eq!(edit.text, "proposal");
        let restored = EditorVersion {
            revision: Uuid::new_v4(),
            ..editor
        };
        service
            .confirm_undo_editor_restoration(
                original,
                undo,
                restore_id,
                observation(&restored, "proposal"),
            )
            .await
            .unwrap();
        let hash = if save_restored {
            save(&f, &hash, "proposal")
        } else {
            hash
        };
        let done = service
            .rollback_disk_proposal_undo(original, undo, current(&f, &restored, &hash, "proposal"))
            .await
            .unwrap();
        assert_eq!(done.application.status, ApplicationStatus::RolledBack);
        assert_eq!(
            done.application
                .files
                .iter()
                .find(|file| file.path == "z.tex")
                .unwrap()
                .phase,
            ApplicationPhase::Pending
        );
        let source = f
            .store
            .manuscript_directory()
            .join(f.manuscript.to_string())
            .join("source");
        assert_eq!(
            std::fs::read(source.join("a.tex")).unwrap(),
            b"first disk file"
        );
        assert_eq!(
            std::fs::read(source.join("z.tex")).unwrap(),
            b"pending disk file"
        );
        assert_eq!(
            service
                .read_proposal_application(
                    original.conversation,
                    original.manuscript,
                    original.proposal,
                    original.operation
                )
                .await
                .unwrap()
                .status,
            ApplicationStatus::Applied
        );
    }
}
#[tokio::test]
async fn saved_partial_undo_rollback_rejects_other_bytes_and_stale_editor_versions() {
    let f = fixture_with_objects();
    let (service, original, undo, editor, hash) = partial(&f).await;
    for buffer in [
        current(
            &f,
            &EditorVersion {
                revision: Uuid::new_v4(),
                ..editor.clone()
            },
            &hash,
            "Frozen manuscript",
        ),
        current(&f, &editor, &hash, "later input"),
    ] {
        assert!(
            service
                .prepare_proposal_undo_rollback(original, undo, buffer)
                .await
                .is_err()
        );
    }
    // Known inverse bytes cannot bypass the independently maintained file index.
    f.store.connection().unwrap().execute(
        "update manuscript_files set byte_size=byte_size+1 where manuscript_id=?1 and relative_path='main.tex'",
        [f.manuscript.to_string()],
    ).unwrap();
    assert!(
        service
            .prepare_proposal_undo_rollback(
                original,
                undo,
                current(&f, &editor, &hash, "Frozen manuscript"),
            )
            .await
            .is_err()
    );
    f.store.connection().unwrap().execute(
        "update manuscript_files set byte_size=byte_size-1 where manuscript_id=?1 and relative_path='main.tex'",
        [f.manuscript.to_string()],
    ).unwrap();
    let external_hash = save(&f, &hash, "external input");
    assert!(
        service
            .prepare_proposal_undo_rollback(
                original,
                undo,
                current(&f, &editor, &external_hash, "Frozen manuscript")
            )
            .await
            .is_err()
    );
    assert!(
        service
            .read_undo_editor_restoration(original, undo, "main.tex".into())
            .await
            .unwrap()
            .1
            .is_none()
    );
    let source = f
        .store
        .manuscript_directory()
        .join(f.manuscript.to_string())
        .join("source");
    assert_eq!(
        std::fs::read(source.join("main.tex")).unwrap(),
        b"external input"
    );
    assert!(!source.join("a.tex").exists());
}
