use super::*;
use crate::{
    agent::{
        writing_application_commands::restoration,
        writing_submission::workspace::UndoCheckpointVersion, writing_workspace_commands::Buffer,
    },
    runtime_plugins::writing_projects::files::{self, FileRequest, checkpoints::CheckpointCall},
};
use research_workbench_native_sdk::writing_checkpoint::{
    CheckpointWrite, DocumentCheckpoint, DocumentOperation, EditorReceipt,
};
use sha2::{Digest, Sha256};

pub(super) async fn preserve(
    f: &Fixture,
    binding: &Binding,
    checkpoint: DocumentCheckpoint,
    revision: Option<u64>,
) {
    files::operate_files(
        f.store.clone(),
        binding.service.clone(),
        binding.catalog.clone(),
        binding.package_key.clone(),
        f.manuscript.to_string(),
        FileRequest::Checkpoint {
            call: CheckpointCall::Save {
                write: Box::new(CheckpointWrite {
                    expected_revision: revision,
                    checkpoint,
                }),
            },
        },
    )
    .await
    .unwrap();
}

#[tokio::test]
#[ignore = "requires actual independent writing package; live editor compensation and current checkpoint CAS"]
async fn actual_package_compensation_keeps_current_checkpoint_and_live_editor_until_formal_save() {
    compensate(false).await;
}

#[tokio::test]
#[ignore = "requires actual independent writing package; rebuilt apply editor and original restoration receipt"]
async fn actual_package_compensation_restores_rebuilt_editor_through_current_checkpoint() {
    compensate(true).await;
}

async fn compensate(rebuilt: bool) {
    let (f, binding, scope, session, revision) = super::editor::partial_editor().await;
    let hash = format!("{:x}", Sha256::digest(b"Revised manuscript"));
    let request_id = crate::agent::repository::change_sets::read_file_operation_in(
        &f.store.connection().unwrap(),
        scope.conversation_id,
        scope.manuscript_id,
        scope.proposal_id,
    )
    .unwrap()
    .binding
    .request_id
    .to_string();
    let mut checkpoint = DocumentCheckpoint {
        schema_version: 1,
        manuscript_id: f.manuscript.to_string(),
        relative_path: "main.tex".into(),
        version: 1,
        content: "Revised manuscript".into(),
        review_baseline: "Frozen manuscript".into(),
        saved_sha256: hash.clone(),
        saved_version: 1,
        operations: vec![DocumentOperation {
            operation_id: scope.operation_id.to_string(),
            conversation_id: scope.conversation_id.to_string(),
            request_id: request_id.clone(),
            result_sha256: hash.clone(),
        }],
        save_intent: None,
        editor_receipt: Some(EditorReceipt {
            operation_id: scope.operation_id.to_string(),
            session_id: session.to_string(),
            revision: revision.to_string(),
            text: "Revised manuscript".into(),
            state_version: Some(1),
        }),
    };
    preserve(&f, &binding, checkpoint.clone(), None).await;
    let session = if rebuilt { Uuid::new_v4() } else { session };
    let revision = if rebuilt { Uuid::new_v4() } else { revision };
    let buffer = |revision, text: &str| {
        vec![Buffer {
            path: "main.tex".into(),
            expected_disk_sha256: hash.clone(),
            session_id: session,
            revision,
            text: text.into(),
        }]
    };
    let proof = |revision, version| {
        vec![UndoCheckpointVersion {
            path: "main.tex".into(),
            revision,
            state_version: version,
        }]
    };
    assert!(
        commands::execute(
            &f.store,
            Request::PrepareRollback {
                scope: scope.clone(),
                buffers: buffer(revision, "Revised manuscript"),
                checkpoint_versions: proof(2, 1)
            }
        )
        .await
        .is_err()
    );
    commands::execute(
        &f.store,
        Request::PrepareRollback {
            scope: scope.clone(),
            buffers: buffer(revision, "Revised manuscript"),
            checkpoint_versions: proof(1, 1),
        },
    )
    .await
    .unwrap();
    assert!(
        commands::execute(
            &f.store,
            Request::RollbackRemainder {
                scope: scope.clone(),
                buffers: Some(buffer(revision, "Revised manuscript")),
                checkpoint_versions: proof(1, 1)
            }
        )
        .await
        .is_err()
    );
    let restore_id = Uuid::new_v4();
    let absent = f.store.clone().with_native_writing_context(Binding {
        package_key: "0".repeat(64),
        ..binding.clone()
    });
    let prepare = |proof| restoration::Request::Prepare {
        scope: scope.clone(),
        restore_id,
        current: commands::Observation {
            path: "main.tex".into(),
            session_id: session,
            revision,
            text: "Revised manuscript".into(),
        },
        checkpoint_version: proof,
    };
    if rebuilt {
        for proof in [None, proof(2, 1).pop()] {
            assert!(
                restoration::execute(&f.store, prepare(proof))
                    .await
                    .is_err()
            );
        }
        let cause = serde_json::to_value(
            restoration::execute(&absent, prepare(proof(1, 1).pop()))
                .await
                .err()
                .expect("missing rebuilt source"),
        )
        .unwrap();
        assert_eq!(cause["code"], "plugin_host_package_unavailable");
        assert_eq!(cause["operationId"], scope.operation_id.to_string());
        let before = std::fs::read(source_root(&f).join("main.tex")).unwrap();
        std::fs::write(source_root(&f).join("main.tex"), "user changed formal file").unwrap();
        assert!(
            restoration::execute(&f.store, prepare(proof(1, 1).pop()))
                .await
                .is_err()
        );
        assert_eq!(
            std::fs::read(source_root(&f).join("main.tex")).unwrap(),
            b"user changed formal file"
        );
        std::fs::write(source_root(&f).join("main.tex"), before).unwrap();
    }
    let dispatched = restoration::execute(
        &f.store,
        prepare(if rebuilt { proof(1, 1).pop() } else { None }),
    )
    .await
    .unwrap();
    let original_files = serde_json::to_value(
        commands::execute(
            &f.store,
            Request::Read {
                scope: scope.clone(),
            },
        )
        .await
        .unwrap(),
    )
    .unwrap()["files"]
        .clone();
    let replay = restoration::execute(
        &absent,
        restoration::Request::Prepare {
            scope: scope.clone(),
            restore_id,
            current: commands::Observation {
                path: "main.tex".into(),
                session_id: Uuid::new_v4(),
                revision: Uuid::new_v4(),
                text: "later input".into(),
            },
            checkpoint_version: None,
        },
    )
    .await
    .unwrap();
    assert!(matches!(replay, restoration::Response::State { .. }));
    assert_eq!(
        serde_json::to_value(
            commands::execute(
                &f.store,
                Request::Read {
                    scope: scope.clone()
                }
            )
            .await
            .unwrap()
        )
        .unwrap()["files"],
        original_files
    );
    let restoration::Response::Edit { text, .. } = dispatched else {
        panic!("original editor compensation");
    };
    assert_eq!(text, "Frozen manuscript");
    let restored_revision = Uuid::new_v4();
    restoration::execute(
        &f.store,
        restoration::Request::Confirm {
            scope: scope.clone(),
            restore_id,
            actual: commands::Observation {
                path: "main.tex".into(),
                session_id: session,
                revision: restored_revision,
                text: text.clone(),
            },
        },
    )
    .await
    .unwrap();
    assert_eq!(
        std::fs::read(source_root(&f).join("main.tex")).unwrap(),
        b"Revised manuscript"
    );
    checkpoint.version = 2;
    checkpoint.content = text.clone();
    checkpoint.operations.push(DocumentOperation {
        operation_id: restore_id.to_string(),
        conversation_id: scope.conversation_id.to_string(),
        request_id,
        result_sha256: format!("{:x}", Sha256::digest(text.as_bytes())),
    });
    checkpoint.editor_receipt = Some(EditorReceipt {
        operation_id: restore_id.to_string(),
        session_id: session.to_string(),
        revision: restored_revision.to_string(),
        text: text.clone(),
        state_version: Some(2),
    });
    preserve(&f, &binding, checkpoint.clone(), Some(1)).await;
    let done = serde_json::to_value(
        commands::execute(
            &f.store,
            Request::RollbackRemainder {
                scope: scope.clone(),
                buffers: Some(buffer(restored_revision, &text)),
                checkpoint_versions: proof(2, 2),
            },
        )
        .await
        .unwrap(),
    )
    .unwrap();
    assert_eq!(done["status"], "rolledBack");
    assert_eq!(
        std::fs::read(source_root(&f).join("remove.tex")).unwrap(),
        b"Retained original"
    );
    assert!(!source_root(&f).join("new.tex").exists());
    let pending = super::terminal::applications(&f.store, f.manuscript)
        .await
        .unwrap();
    assert_eq!(pending["items"].as_array().unwrap().len(), 1);
    let cause = serde_json::to_value(
        super::terminal::applications(&absent, f.manuscript)
            .await
            .unwrap_err(),
    )
    .unwrap();
    assert_eq!(cause["code"], "plugin_host_package_unavailable");
    assert_eq!(cause["operationId"], scope.operation_id.to_string());
    assert_eq!(
        std::fs::read(source_root(&f).join("main.tex")).unwrap(),
        b"Revised manuscript"
    );
    files::operate_files(
        f.store.clone(),
        binding.service.clone(),
        binding.catalog.clone(),
        binding.package_key.clone(),
        f.manuscript.to_string(),
        FileRequest::Save {
            request: research_workbench_native_sdk::writing_project::SaveFileRequest {
                manuscript_id: f.manuscript.to_string(),
                relative_path: "main.tex".into(),
                content: text,
                expected_sha256: Some(hash),
            },
        },
    )
    .await
    .unwrap();
    checkpoint.saved_sha256 = format!("{:x}", Sha256::digest(b"Frozen manuscript"));
    checkpoint.saved_version = 2;
    preserve(&f, &binding, checkpoint, Some(2)).await;
    assert!(
        super::terminal::applications(&absent, f.manuscript)
            .await
            .unwrap()["items"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        std::fs::read(source_root(&f).join("main.tex")).unwrap(),
        b"Frozen manuscript"
    );
    binding.service.shutdown().await.unwrap();
    f.store.shared_agent_connection().shutdown().unwrap();
}
