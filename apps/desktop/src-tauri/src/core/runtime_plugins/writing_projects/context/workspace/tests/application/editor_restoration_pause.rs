use super::*;
use crate::agent::{
    writing_checkpoint_contracts::CheckpointRecord,
    writing_submission::workspace::{
        self, EditorObservation, UndoCheckpointVersion, restoration_source::RestorationSource,
    },
    writing_workspace::index::FileIndex,
};
use crate::ai::structured_resource_validation::Flow;
use research_agent_core::workspace::EditorVersion;
use serde_json::json;
use sha2::{Digest, Sha256};

struct PausingSource {
    record: CheckpointRecord,
    phase: &'static str,
    checkpoints: usize,
    indexes: usize,
}
impl RestorationSource for PausingSource {
    type Pause = &'static str;
    fn checkpoint(
        &mut self,
        _: &rusqlite::Connection,
        _: Uuid,
        _: &str,
    ) -> Result<Option<CheckpointRecord>, Flow<workspace::Error, Self::Pause>> {
        self.checkpoints += 1;
        if self.phase == "checkpoint" {
            Err(Flow::Need(self.phase))
        } else {
            Ok(Some(self.record.clone()))
        }
    }
    fn index(&mut self, _: &str) -> Result<FileIndex, Flow<workspace::Error, Self::Pause>> {
        self.indexes += 1;
        Err(Flow::Need(self.phase))
    }
}

#[tokio::test]
#[ignore = "requires actual independent writing package; rebuilt editor preparation source pause"]
async fn actual_package_rebuilt_editor_restoration_settles_each_source_and_fences_failed_rollback()
{
    for phase in ["checkpoint", "index"] {
        for fenced in [false, true] {
            let (f, binding, scope, session, revision) = super::editor::partial_editor().await;
            let checked = application_scope(&scope);
            let proposal = crate::agent::repository::change_sets::read_file_operation_in(
                &f.store.connection().unwrap(),
                checked.conversation,
                checked.manuscript,
                checked.proposal,
            )
            .unwrap();
            let hash = format!("{:x}", Sha256::digest(b"Revised manuscript"));
            let record = CheckpointRecord {
                revision:1,
                checkpoint:serde_json::from_value(json!({
                    "schemaVersion":1,"manuscriptId":f.manuscript,"relativePath":"main.tex",
                    "version":1,"content":"Revised manuscript","reviewBaseline":"Frozen manuscript",
                    "savedSha256":hash,"savedVersion":1,"saveIntent":null,
                    "operations":[{"operationId":scope.operation_id,"conversationId":scope.conversation_id,
                        "requestId":proposal.binding.request_id,"resultSha256":hash}],
                    "editorReceipt":{"operationId":scope.operation_id,"sessionId":session,
                        "revision":revision,"text":"Revised manuscript","stateVersion":1}
                })).unwrap(),
            };
            let mut source = PausingSource {
                record,
                phase,
                checkpoints: 0,
                indexes: 0,
            };
            let current = EditorObservation {
                path: "main.tex".into(),
                version: EditorVersion {
                    session_id: Uuid::new_v4(),
                    revision: Uuid::new_v4(),
                },
                text: "Revised manuscript".into(),
            };
            let text_allocation = current.text.as_ptr();
            let original = state(&f.store, &scope).await;
            super::compensation_pause::deny_rollback(&f, fenced);
            let result = workspace::restore_editor::prepare_with_source(
                &f.store,
                checked,
                Uuid::new_v4(),
                &current,
                &Some(UndoCheckpointVersion {
                    path: "main.tex".into(),
                    revision: 1,
                    state_version: 1,
                }),
                &mut source,
            );
            super::compensation_pause::check_result(&f, result, fenced, phase);
            assert_eq!(current.text, "Revised manuscript");
            assert_eq!(current.text.as_ptr(), text_allocation);
            assert_eq!(source.checkpoints, 1);
            assert_eq!(source.indexes, usize::from(phase == "index"));
            assert_eq!(
                std::fs::read(source_root(&f).join("main.tex")).unwrap(),
                b"Revised manuscript"
            );
            if !fenced {
                assert_eq!(state(&f.store, &scope).await, original);
            }
            super::compensation_pause::close(f, binding, fenced).await;
        }
    }
}
