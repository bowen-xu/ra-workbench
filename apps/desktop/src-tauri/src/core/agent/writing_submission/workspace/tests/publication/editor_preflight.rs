use super::*;
use crate::agent::writing_workspace::EditorBuffer;
use research_agent_core::workspace::EditorVersion;
use std::os::unix::fs::PermissionsExt;

#[tokio::test]
async fn unsupported_editor_targets_reject_the_whole_plan_before_admission() {
    for case in ["delete", "binary", "oversized", "mode"] {
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
        let buffers = || WritingWorkspaceRequest {
            manuscript_id: f.manuscript,
            buffers: vec![EditorBuffer {
                path: "main.tex".into(),
                expected_disk_sha256: format!("{:x}", Sha256::digest(b"Frozen manuscript")),
                editor: editor.clone(),
                text: "unsaved baseline".into(),
            }],
        };
        let copy = service
            .prepare_workspace(receipt, prepared.record.context_snapshot_id, buffers())
            .await
            .unwrap();
        let root = copy.lease.working_directory().unwrap();
        // This valid disk addition sorts before the unsupported editor target.
        std::fs::write(root.join("aaa.tex"), "valid new file").unwrap();
        let target = root.join("main.tex");
        match case {
            "delete" => std::fs::remove_file(&target).unwrap(),
            "binary" => std::fs::write(&target, [0xff, 0xfe]).unwrap(),
            "oversized" => std::fs::write(&target, vec![b'a'; 4 * 1024 * 1024 + 1]).unwrap(),
            "mode" => {
                std::fs::write(&target, "changed text").unwrap();
                std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o700)).unwrap();
            }
            _ => unreachable!(),
        }
        drop(copy);
        let review = service
            .review_workspace(receipt, prepared.record.context_snapshot_id)
            .await
            .unwrap();
        assert_eq!(review.changes.len(), 2, "{case}");
        let proposal = service
            .publish_workspace_changes(review.binding, review.changes)
            .await
            .unwrap();
        assert!(
            service
                .begin_proposal_application(f.conversation, proposal, Uuid::new_v4(), buffers())
                .await
                .is_err(),
            "{case}"
        );
        assert!(
            service
                .find_proposal_application(f.conversation, f.manuscript, proposal)
                .await
                .unwrap()
                .is_none(),
            "{case}"
        );
        let source = f
            .store
            .manuscript_directory()
            .join(f.manuscript.to_string())
            .join("source");
        assert!(!source.join("aaa.tex").exists(), "{case}");
        assert_eq!(
            std::fs::read(source.join("main.tex")).unwrap(),
            b"Frozen manuscript",
            "{case}"
        );
        assert_eq!(
            service
                .review_proposal(f.conversation, f.manuscript, proposal)
                .await
                .unwrap()
                .status,
            crate::agent::repository::change_sets::ChangeSetStatus::Pending,
            "{case}"
        );
    }
}
