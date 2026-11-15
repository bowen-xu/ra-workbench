use super::*;
use crate::{
    agent::{
        writing_checkpoint_contracts::CheckpointRecord,
        writing_submission::workspace::{
            self, UndoCheckpointVersion,
            application_rollback_check::{self, RollbackStage},
            restoration_source::RestorationSource,
            undo_rollback_plan::{self, UndoRollbackCheck},
        },
        writing_workspace::{EditorBuffer, index::FileIndex},
    },
    ai::structured_resource_validation::Flow,
};
use research_agent_core::workspace::EditorVersion;
use rusqlite::hooks::{AuthAction, AuthContext, Authorization, TransactionOperation};
use serde_json::json;
use sha2::{Digest, Sha256};

#[derive(Default)]
struct Pause {
    checkpoints: usize,
    indexes: usize,
}
impl RestorationSource for Pause {
    type Pause = &'static str;
    fn checkpoint(
        &mut self,
        _: &rusqlite::Connection,
        _: Uuid,
        _: &str,
    ) -> Result<Option<CheckpointRecord>, Flow<workspace::Error, Self::Pause>> {
        self.checkpoints += 1;
        Err(Flow::Need("checkpoint"))
    }
    fn index(&mut self, _: &str) -> Result<FileIndex, Flow<workspace::Error, Self::Pause>> {
        self.indexes += 1;
        Err(Flow::Need("index"))
    }
}
pub(super) fn deny_rollback(f: &Fixture, deny: bool) {
    if deny {
        f.store
            .connection()
            .unwrap()
            .authorizer(Some(|context: AuthContext<'_>| match context.action {
                AuthAction::Transaction {
                    operation: TransactionOperation::Rollback,
                } => Authorization::Deny,
                _ => Authorization::Allow,
            }));
    }
}
pub(super) async fn close(f: Fixture, binding: Binding, fenced: bool) {
    binding.service.shutdown().await.unwrap();
    let shutdown = f.store.shared_agent_connection().shutdown();
    if fenced {
        assert!(
            matches!(shutdown, Err(crate::data_interface::DataError::Admission(cause)) if cause.contains("data_recovery_required"))
        );
    } else {
        shutdown.unwrap();
    }
}
pub(super) fn check_result<T>(
    f: &Fixture,
    result: Result<T, Flow<workspace::Error, &'static str>>,
    fenced: bool,
    expected: &str,
) {
    if fenced {
        assert!(
            matches!(result, Err(Flow::Fail(cause)) if matches!(cause.error, workspace::Error::RecoveryRequired))
        );
        assert!(f.store.connection().is_err());
    } else {
        assert!(matches!(result, Err(Flow::Need(pause)) if pause == expected));
        assert!(f.store.connection().unwrap().is_autocommit());
    }
}

#[tokio::test]
#[ignore = "requires actual independent writing package; application compensation checkpoint/index source pause"]
async fn actual_package_application_compensation_settles_both_source_phases_and_fences_failed_rollback()
 {
    for checkpoint in [false, true] {
        for fenced in [false, true] {
            let (f, binding, scope, session, revision) = super::editor::partial_editor().await;
            let request = WritingWorkspaceRequest {
                manuscript_id: f.manuscript,
                buffers: vec![EditorBuffer {
                    path: "main.tex".into(),
                    expected_disk_sha256: format!("{:x}", Sha256::digest(b"Revised manuscript")),
                    editor: EditorVersion {
                        session_id: session,
                        revision,
                    },
                    text: "Revised manuscript".into(),
                }],
            };
            let proofs = if checkpoint {
                vec![UndoCheckpointVersion {
                    path: "main.tex".into(),
                    revision: 1,
                    state_version: 1,
                }]
            } else {
                vec![]
            };
            let before = state(&f.store, &scope).await;
            deny_rollback(&f, fenced);
            let mut source = Pause::default();
            let result = application_rollback_check::check_with_source(
                &f.store,
                application_scope(&scope),
                &request,
                &proofs,
                RollbackStage::BeforeEditorCompensation,
                &mut source,
            );
            check_result(
                &f,
                result,
                fenced,
                if checkpoint { "checkpoint" } else { "index" },
            );
            assert_eq!(source.checkpoints, usize::from(checkpoint));
            assert_eq!(source.indexes, usize::from(!checkpoint));
            assert_eq!(request.buffers[0].text, "Revised manuscript");
            assert_eq!(
                std::fs::read(source_root(&f).join("main.tex")).unwrap(),
                b"Revised manuscript"
            );
            if !fenced {
                assert_eq!(state(&f.store, &scope).await, before);
            }
            close(f, binding, fenced).await;
        }
    }
}

#[tokio::test]
#[ignore = "requires actual independent writing package; inverse compensation source pause"]
async fn actual_package_undo_compensation_settles_before_effects_and_fences_failed_rollback() {
    for fenced in [false, true] {
        let (f, binding, scope) = super::undo::applied().await;
        let operation = Uuid::new_v4();
        for action in ["prepare", "start"] {
            super::undo::run(
                &f.store,
                &scope,
                operation,
                json!({"action":action,"buffers":[]}),
            )
            .await
            .unwrap();
        }
        super::undo::run(
            &f.store,
            &scope,
            operation,
            json!({"action":"applyDisk","path":"main.tex"}),
        )
        .await
        .unwrap();
        let before = super::undo::run(&f.store, &scope, operation, json!({"action":"read"}))
            .await
            .unwrap();
        deny_rollback(&f, fenced);
        let mut source = Pause::default();
        let result = undo_rollback_plan::check_with_source(
            &f.store,
            &WritingWorkspaceRequest {
                manuscript_id: f.manuscript,
                buffers: vec![],
            },
            &UndoRollbackCheck {
                original: application_scope(&scope),
                operation,
                allow_editor_effects: false,
                checkpoints: vec![],
            },
            &mut source,
        );
        check_result(&f, result, fenced, "index");
        assert_eq!(source.checkpoints, 0);
        assert_eq!(source.indexes, 1);
        assert_eq!(
            std::fs::read(source_root(&f).join("main.tex")).unwrap(),
            b"Frozen manuscript"
        );
        if !fenced {
            assert_eq!(
                super::undo::run(&f.store, &scope, operation, json!({"action":"read"}))
                    .await
                    .unwrap(),
                before
            );
        }
        close(f, binding, fenced).await;
    }
}
