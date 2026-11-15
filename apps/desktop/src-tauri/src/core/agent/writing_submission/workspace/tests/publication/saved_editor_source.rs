//! Saving the captured working buffer does not make its Agent result stale.
use super::*;
use crate::agent::repository::proposal_applications::ApplicationScope;
use crate::agent::writing_workspace::EditorBuffer;
use research_agent_core::workspace::EditorVersion;

#[tokio::test]
async fn saved_frozen_buffer_is_admitted_but_different_disk_bytes_are_rejected() {
    for disk_text in ["latest unsaved buffer", "external different content"] {
        let f = fixture_with_objects();
        let service = WritingSubmissionService::from_app_store(&f.store);
        let editor = EditorVersion {
            session_id: Uuid::new_v4(),
            revision: Uuid::new_v4(),
        };
        let mut input = request(&f, Uuid::new_v4());
        input.context.files[0].buffer = Some(crate::agent::writing_context::WritingContextBuffer {
            revision: editor.revision,
            text: "latest unsaved buffer".into(),
        });
        let prepared = service.prepare(&f.models, input).await.unwrap();
        let receipt = WritingSubmissionReceipt {
            conversation_id: f.conversation,
            request_id: prepared.record.request_id,
            manuscript_id: f.manuscript,
        };
        let current = WritingWorkspaceRequest {
            manuscript_id: f.manuscript,
            buffers: vec![EditorBuffer {
                path: "main.tex".into(),
                expected_disk_sha256: format!("{:x}", Sha256::digest(b"Frozen manuscript")),
                editor: editor.clone(),
                text: "latest unsaved buffer".into(),
            }],
        };
        let copy = service
            .prepare_workspace(receipt, prepared.record.context_snapshot_id, current)
            .await
            .unwrap();
        std::fs::write(
            copy.lease.working_directory().unwrap().join("main.tex"),
            "Agent after latest input",
        )
        .unwrap();
        drop(copy);
        let review = service
            .review_workspace(receipt, prepared.record.context_snapshot_id)
            .await
            .unwrap();
        let historical = review.changes.clone();
        let proposal = service
            .publish_workspace_changes(review.binding, review.changes)
            .await
            .unwrap();
        let source = f
            .store
            .manuscript_directory()
            .join(f.manuscript.to_string())
            .join("source/main.tex");
        std::fs::write(&source, disk_text).unwrap();
        let disk_hash = format!("{:x}", Sha256::digest(disk_text.as_bytes()));
        // Represent a completed formal save: file bytes and inventory agree.
        f.store.connection().unwrap().execute("update manuscript_files set sha256=?1,byte_size=?2 where manuscript_id=?3 and relative_path='main.tex'",
            params![disk_hash, disk_text.len(), f.manuscript.to_string()]).unwrap();
        let current = WritingWorkspaceRequest {
            manuscript_id: f.manuscript,
            buffers: vec![EditorBuffer {
                path: "main.tex".into(),
                expected_disk_sha256: disk_hash,
                editor: editor.clone(),
                text: "latest unsaved buffer".into(),
            }],
        };
        let scope = ApplicationScope {
            conversation: f.conversation,
            manuscript: f.manuscript,
            proposal,
            operation: Uuid::new_v4(),
        };
        let result = service
            .begin_proposal_application(f.conversation, proposal, scope.operation, current)
            .await;
        if disk_text == "latest unsaved buffer" {
            result.unwrap();
            let dispatch = service
                .prepare_editor_proposal_file(
                    scope,
                    EditorObservation {
                        path: "main.tex".into(),
                        version: editor,
                        text: "latest unsaved buffer".into(),
                    },
                )
                .await
                .unwrap();
            let EditorAdmission::Dispatch(edit) = dispatch else {
                panic!("expected current editor dispatch")
            };
            assert_eq!(edit.text, "Agent after latest input");
        } else {
            assert!(result.is_err());
            assert!(
                service
                    .find_proposal_application(f.conversation, f.manuscript, proposal)
                    .await
                    .unwrap()
                    .is_none()
            );
        }
        assert_eq!(std::fs::read_to_string(&source).unwrap(), disk_text);
        assert_eq!(
            service
                .review_proposal(f.conversation, f.manuscript, proposal)
                .await
                .unwrap()
                .changes,
            historical
        );
    }
}
