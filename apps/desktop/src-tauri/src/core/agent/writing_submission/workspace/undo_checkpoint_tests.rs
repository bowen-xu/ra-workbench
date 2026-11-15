use super::*;
use crate::agent::writing_checkpoint_contracts::CheckpointRecord;
use crate::agent::writing_context::WritingContextError;
use crate::agent::writing_workspace::EditorBuffer;
use research_agent_core::workspace::EditorVersion;
use rusqlite::Connection;

fn request(manuscript: Uuid) -> WritingWorkspaceRequest {
    WritingWorkspaceRequest {
        manuscript_id: manuscript,
        buffers: vec![EditorBuffer {
            path: "main.tex".into(),
            expected_disk_sha256: "0".repeat(64),
            editor: EditorVersion {
                session_id: Uuid::new_v4(),
                revision: Uuid::new_v4(),
            },
            text: "current input".into(),
        }],
    }
}

fn recording_read(
    db: &Connection,
    manuscript: Uuid,
    path: &str,
) -> Result<Option<CheckpointRecord>, WritingContextError> {
    db.execute(
        "insert into reads(manuscript,path) values(?1,?2)",
        rusqlite::params![manuscript.to_string(), path],
    )
    .unwrap();
    Ok(None)
}

fn unexpected_read(
    _: &Connection,
    _: Uuid,
    _: &str,
) -> Result<Option<CheckpointRecord>, WritingContextError> {
    panic!("this branch must not read a checkpoint")
}

#[test]
fn checkpoint_versions_are_lazy_and_keep_buffer_lookup_before_query() {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch("create table reads(manuscript text,path text)")
        .unwrap();
    let request = request(Uuid::new_v4());
    assert!(read_in(&db, None, &request, &[]).unwrap().is_empty());
    let proof = UndoCheckpointVersion {
        path: "main.tex".into(),
        revision: 0,
        state_version: u64::MAX,
    };
    assert!(matches!(
        read_in(&db, None, &request, std::slice::from_ref(&proof)),
        Err(Error::Lease(LeaseError::Conflict))
    ));
    let foreign = UndoCheckpointVersion {
        path: "missing.tex".into(),
        ..proof.clone()
    };
    assert!(matches!(
        read_in(&db, Some(unexpected_read), &request, &[foreign]),
        Err(Error::Lease(LeaseError::Conflict))
    ));
    // The original query precedes numeric/version proof validation.
    assert!(matches!(
        read_in(&db, Some(recording_read), &request, &[proof]),
        Err(Error::Lease(LeaseError::Conflict))
    ));
    let reads: Vec<(String, String)> = db
        .prepare("select manuscript,path from reads")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        reads,
        vec![(request.manuscript_id.to_string(), "main.tex".into())]
    );
}

#[test]
fn editor_checkpoint_queries_before_proof_and_keeps_conflict_mapping() {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch("create table reads(manuscript text,path text)")
        .unwrap();
    let manuscript = Uuid::new_v4();
    let scope = ApplicationScope {
        conversation: Uuid::new_v4(),
        manuscript,
        proposal: Uuid::new_v4(),
        operation: Uuid::new_v4(),
    };
    let current = EditorObservation {
        path: "main.tex".into(),
        version: request(manuscript).buffers.remove(0).editor,
        text: "current input".into(),
    };
    let invalid_proof = UndoCheckpointVersion {
        path: "wrong.tex".into(),
        revision: 0,
        state_version: u64::MAX,
    };
    assert!(matches!(
        read_editor_in(&db, Some(recording_read), scope, &current, &invalid_proof),
        Err(Error::Lease(LeaseError::Conflict))
    ));
    let read: (String, String) = db
        .query_row("select manuscript,path from reads", [], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .unwrap();
    assert_eq!(read, (manuscript.to_string(), current.path.clone()));
    assert!(matches!(
        read_editor_in(&db, None, scope, &current, &invalid_proof),
        Err(Error::Lease(LeaseError::Conflict))
    ));
    let failed_read: ReadCheckpoint = |_, _, _| Err(WritingContextError::Unavailable);
    assert!(matches!(
        read_editor_in(&db, Some(failed_read), scope, &current, &invalid_proof),
        Err(Error::Lease(LeaseError::Conflict))
    ));
}
