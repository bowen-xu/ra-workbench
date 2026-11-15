use super::*;
#[cfg(target_os = "macos")]
#[path = "tests/shell.rs"]
mod shell;
use research_agent_core::{
    conversation::ConversationId,
    protocol::ModelIdentity,
    submission::{ContextText, SubmissionInput},
    workspace::{BaselineFile, FileVersion, SourceVersion, WorkspaceFile, WorkspaceIdentity},
};
fn setup(db: &mut Connection) -> (Uuid, Uuid, WorkspaceManifest) {
    crate::storage_migration_tests::migrate_through(db, 89);
    db.execute_batch(
        "insert into research_projects(id,title,summary,created_at) values('p','p','','now');",
    )
    .unwrap();
    let manuscript = Uuid::new_v4();
    let conversation = Uuid::new_v4();
    let request = Uuid::new_v4();
    db.execute("insert into manuscripts(id,project_id,title,created_at,updated_at) values(?1,'p','paper','now','now')",[manuscript.to_string()]).unwrap();
    crate::agent::repository::create_conversation(
        db,
        ConversationId::from_uuid(conversation),
        "test",
    )
    .unwrap();
    db.execute("insert into agent_conversation_bindings(conversation_id,resource_kind,resource_id) values(?1,'manuscript',?2)",params![conversation.to_string(),manuscript.to_string()]).unwrap();
    let hash = format!("{:x}", Sha256::digest(b"source"));
    let tx = db.transaction().unwrap();
    let record=crate::agent::repository::submissions::insert_in(&tx,conversation,request,SubmissionInput {
remote_compaction_v2: None,
            user_input: vec![],
            skill_instructions: vec![],
        text:"edit".into(),instructions:vec![],instruction_configs:vec![],skills:vec![],reasoning_effort:None,service_tier:None,
        model:ModelIdentity {provider_id:"fixture".into(),model_id:"fixture".into(),revision:None},
        context:vec![ContextText {source_id:"s".into(),kind:ContextSourceKind::Manuscript,resource_id:"main.tex".into(),revision:format!("sha256:{hash}"),text:"source".into(),anchor:Some(serde_json::json!({"manuscriptId":manuscript,"diskSha256":hash,"bufferRevision":null}).to_string())}],
    }).unwrap();
    tx.commit().unwrap();
    (
        request,
        manuscript,
        WorkspaceManifest {
            schema_version: 1,
            identity: WorkspaceIdentity {
                conversation_id: conversation,
                turn_id: record.turn_id,
                lease_id: Uuid::new_v4(),
                environment_id: "writing".into(),
            },
            files: vec![BaselineFile {
                file: WorkspaceFile {
                    path: "main.tex".into(),
                    version: FileVersion {
                        sha256: hash.clone(),
                        byte_size: 6,
                        executable: false,
                    },
                },
                source: SourceVersion {
                    resource_id: format!("{manuscript}/main.tex"),
                    revision: format!("sha256:{hash}"),
                    editor: None,
                },
            }],
        },
    )
}
#[test]
fn explicit_approval_survives_reopen_and_cannot_be_transferred_or_reactivated() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("db.sqlite");
    let mut db = Connection::open(&path).unwrap();
    let (request, manuscript, manifest) = setup(&mut db);
    let tx = db.transaction().unwrap();
    let binding = prepare_in(&tx, request, manuscript, &manifest).unwrap();
    assert_eq!(
        prepare_in(&tx, request, manuscript, &manifest).unwrap(),
        binding
    );
    tx.commit().unwrap();
    assert_eq!(authorize(&db, &binding, &manifest), Err(Error::Denied));
    let tx = db.transaction().unwrap();
    approve_in(&tx, &binding).unwrap();
    tx.commit().unwrap();
    drop(db);
    let mut db = Connection::open(path).unwrap();
    db.pragma_update(None, "foreign_keys", true).unwrap();
    authorize(&db, &binding, &manifest).unwrap();
    let mut changed = manifest.clone();
    changed.identity.lease_id = Uuid::new_v4();
    assert_eq!(authorize(&db, &binding, &changed), Err(Error::Conflict));
    let tx = db.transaction().unwrap();
    assert_eq!(
        prepare_in(&tx, request, manuscript, &changed),
        Err(Error::Conflict)
    );
    tx.rollback().unwrap();
    let mut wrong = binding.clone();
    wrong.context_snapshot_id = Uuid::new_v4();
    assert!(authorize(&db, &wrong, &manifest).is_err());
    let tx = db.transaction().unwrap();
    revoke_in(&tx, binding.conversation_id, request).unwrap();
    tx.commit().unwrap();
    assert_eq!(authorize(&db, &binding, &manifest), Err(Error::Denied));
    let tx = db.transaction().unwrap();
    assert_eq!(approve_in(&tx, &binding), Err(Error::Denied));
}
#[test]
fn rollback_is_not_approval_and_terminal_commit_revokes_the_exact_turn() {
    let mut db = Connection::open_in_memory().unwrap();
    let (request, manuscript, manifest) = setup(&mut db);
    let tx = db.transaction().unwrap();
    let binding = prepare_in(&tx, request, manuscript, &manifest).unwrap();
    tx.commit().unwrap();
    let tx = db.transaction().unwrap();
    approve_in(&tx, &binding).unwrap();
    tx.rollback().unwrap();
    assert_eq!(authorize(&db, &binding, &manifest), Err(Error::Denied));
    let tx = db.transaction().unwrap();
    approve_in(&tx, &binding).unwrap();
    tx.commit().unwrap();
    let terminal = research_agent_core::protocol::EventPayload::TurnFinished {
        turn_id: binding.turn_id,
        outcome: research_agent_core::protocol::TurnOutcome::Completed,
    };
    db.execute("insert into agent_items(conversation_id,ordinal,schema_version,payload_json) values(?1,0,1,?2)",params![binding.conversation_id.to_string(),serde_json::json!({"payload":terminal}).to_string()]).unwrap();
    assert_eq!(authorize(&db, &binding, &manifest), Err(Error::Denied));
    let state: String = db
        .query_row("select state from agent_execution_workspaces", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(state, "revoked");
    assert_eq!(
        read(&db, binding.conversation_id, request),
        Err(Error::Denied)
    );
    assert_eq!(
        read_for_review(&db, binding.conversation_id, request).unwrap(),
        Some((binding.clone(), manifest))
    );
    let tx = db.transaction().unwrap();
    assert_eq!(approve_in(&tx, &binding), Err(Error::Denied));
}
#[test]
fn mismatched_source_and_removed_binding_cannot_authorize_execution() {
    let mut db = Connection::open_in_memory().unwrap();
    let (request, manuscript, manifest) = setup(&mut db);
    let mut changed = manifest.clone();
    changed.files[0].file.version.sha256 = "0".repeat(64);
    let tx = db.transaction().unwrap();
    assert_eq!(
        prepare_in(&tx, request, manuscript, &changed),
        Err(Error::Conflict)
    );
    tx.rollback().unwrap();
    let tx = db.transaction().unwrap();
    let binding = prepare_in(&tx, request, manuscript, &manifest).unwrap();
    approve_in(&tx, &binding).unwrap();
    tx.commit().unwrap();
    db.execute(
        "update agent_execution_workspaces set manifest_json='{}'",
        [],
    )
    .unwrap();
    assert_eq!(authorize(&db, &binding, &manifest), Err(Error::Conflict));
    assert_eq!(
        read_for_review(&db, binding.conversation_id, request),
        Err(Error::Conflict)
    );
    db.execute(
        "update agent_execution_workspaces set manifest_json=?1",
        [encoded(&manifest).unwrap().1],
    )
    .unwrap();
    db.execute("delete from agent_conversation_bindings", [])
        .unwrap();
    assert_eq!(authorize(&db, &binding, &manifest), Err(Error::Denied));
    assert_eq!(
        read_for_review(&db, binding.conversation_id, request),
        Err(Error::Denied)
    );
}

#[test]
fn history_pages_durable_copies_with_ties_and_scope_isolation() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("history.sqlite");
    let mut db = Connection::open(&path).unwrap();
    let (request, manuscript, manifest) = setup(&mut db);
    let conversation = manifest.identity.conversation_id;
    let input = super::super::submissions::read(&db, conversation, request)
        .unwrap()
        .unwrap()
        .input;
    let mut requests = vec![request];
    for _ in 0..3 {
        let tx = db.transaction().unwrap();
        let request = Uuid::new_v4();
        let record =
            super::super::submissions::insert_in(&tx, conversation, request, input.clone())
                .unwrap();
        let mut copy = manifest.clone();
        copy.identity.turn_id = record.turn_id;
        copy.identity.lease_id = Uuid::new_v4();
        prepare_in(&tx, request, manuscript, &copy).unwrap();
        revoke_in(&tx, conversation, request).unwrap();
        tx.commit().unwrap();
        requests.push(request);
    }
    let tx = db.transaction().unwrap();
    prepare_in(&tx, request, manuscript, &manifest).unwrap();
    tx.commit().unwrap();
    db.execute(
        "update agent_submissions set created_at='2026-09-09T00:00:00.000Z'",
        [],
    )
    .unwrap();
    drop(db);
    let mut db = Connection::open(path).unwrap();
    requests.sort_by_key(|id| std::cmp::Reverse(id.to_string()));
    let first = history::list(&db, conversation, manuscript, None, 2).unwrap();
    assert_eq!(
        first
            .entries
            .iter()
            .map(|e| e.request_id)
            .collect::<Vec<_>>(),
        requests[..2]
    );
    assert_eq!(first.next_cursor, Some(requests[1]));
    // Pending records must not duplicate entries, and completed cleanup must not
    // remove the cursor or the route to frozen proposals from history.
    let (binding, baseline) = read_metadata_unscoped(&db, conversation, requests[1])
        .unwrap()
        .unwrap();
    let cleanup = super::discard::Request {
        operation_id: Uuid::new_v4(),
        binding,
        environment_id: baseline.identity.environment_id,
        inventory_hash: "a".repeat(64),
    };
    let mut tx = db.transaction().unwrap();
    super::discard::begin_in(&mut tx, &cleanup).unwrap();
    tx.commit().unwrap();
    assert_eq!(
        history::list(&db, conversation, manuscript, None, 2).unwrap(),
        first
    );
    let mut tx = db.transaction().unwrap();
    super::discard::complete_in(&mut tx, &cleanup).unwrap();
    tx.commit().unwrap();
    assert_eq!(
        history::list(&db, conversation, manuscript, None, 2).unwrap(),
        first
    );
    let second = history::list(&db, conversation, manuscript, first.next_cursor, 2).unwrap();
    assert_eq!(
        second
            .entries
            .iter()
            .map(|e| e.request_id)
            .collect::<Vec<_>>(),
        requests[2..]
    );
    assert_eq!(second.next_cursor, None);
    assert!(history::list(&db, conversation, manuscript, Some(Uuid::new_v4()), 2).is_err());
    assert!(history::list(&db, conversation, manuscript, None, 101).is_err());
    assert!(
        history::list(&db, conversation, Uuid::new_v4(), None, 2)
            .unwrap()
            .entries
            .is_empty()
    );
    assert!(
        history::list(&db, Uuid::new_v4(), manuscript, None, 2)
            .unwrap()
            .entries
            .is_empty()
    );
    db.execute("update agent_conversations set archived=1", [])
        .unwrap();
    // Archive disables execution, but retains read-only execution history.
    assert_eq!(
        history::list(&db, conversation, manuscript, None, 2).unwrap(),
        first
    );
    db.execute("update agent_conversations set archived=0", [])
        .unwrap();
    db.execute("delete from agent_conversation_bindings", [])
        .unwrap();
    assert!(
        history::list(&db, conversation, manuscript, None, 2)
            .unwrap()
            .entries
            .is_empty()
    );
}

#[path = "tests/discard.rs"]
mod discard;
#[path = "tests/proposals.rs"]
mod proposals;

#[path = "tests/dynamic_scope.rs"]
mod dynamic_scope;
