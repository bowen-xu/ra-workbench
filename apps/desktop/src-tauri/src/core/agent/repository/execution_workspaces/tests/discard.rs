use super::super::discard::{Request, State, begin_in, complete_in, read};
use super::*;
#[path = "discard/recovery.rs"]
mod recovery;

fn fixture(db: &mut Connection) -> (Request, WorkspaceManifest) {
    let (submission, manuscript, manifest) = setup(db);
    let tx = db.transaction().unwrap();
    let binding = prepare_in(&tx, submission, manuscript, &manifest).unwrap();
    approve_in(&tx, &binding).unwrap();
    tx.commit().unwrap();
    (
        Request {
            operation_id: Uuid::new_v4(),
            binding,
            environment_id: manifest.identity.environment_id.clone(),
            inventory_hash: "a".repeat(64),
        },
        manifest,
    )
}

#[test]
fn intent_and_completion_survive_reopen_and_fence_preparation_after_conversation_deletion() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("discard.sqlite");
    let mut db = Connection::open(&path).unwrap();
    let (request, manifest) = fixture(&mut db);
    let b = &request.binding;
    // Archiving or removing a UI binding must not lose ownership of the private copy.
    db.execute("update agent_conversations set archived=1", [])
        .unwrap();
    db.execute("delete from agent_conversation_bindings", [])
        .unwrap();
    let mut tx = db.transaction().unwrap();
    assert_eq!(begin_in(&mut tx, &request).unwrap(), State::Pending);
    tx.commit().unwrap();
    assert!(
        db.execute("update agent_execution_workspaces set state='approved'", [])
            .is_err()
    );
    assert!(
        db.execute(
            "update agent_execution_workspaces set manifest_json='{}'",
            []
        )
        .is_err()
    );
    assert!(db.execute("delete from agent_conversations", []).is_err());
    assert!(super::super::read_for_review(&db, b.conversation_id, b.request_id).is_err());
    drop(db);
    let mut db = Connection::open(&path).unwrap();
    db.pragma_update(None, "foreign_keys", true).unwrap();
    assert_eq!(read(&db, &request).unwrap(), Some(State::Pending));
    let mut changed = request.clone();
    changed.inventory_hash = "b".repeat(64);
    assert_eq!(read(&db, &changed), Err(Error::Conflict));
    changed = request.clone();
    changed.operation_id = Uuid::new_v4();
    assert_eq!(read(&db, &changed), Err(Error::Conflict));
    let mut tx = db.transaction().unwrap();
    complete_in(&mut tx, &request).unwrap();
    tx.commit().unwrap();
    db.execute("delete from agent_conversations", []).unwrap();
    assert_eq!(read(&db, &request).unwrap(), Some(State::Complete));
    let mut tx = db.transaction().unwrap();
    complete_in(&mut tx, &request).unwrap();
    assert_eq!(
        prepare_in(&tx, b.request_id, b.manuscript_id, &manifest),
        Err(Error::Denied)
    );
    tx.commit().unwrap();
    assert!(
        db.execute("delete from agent_workspace_discards", [])
            .is_err()
    );
    assert!(
        db.execute(
            "update agent_workspace_discards set state='pending',completed_at=null",
            []
        )
        .is_err()
    );
    assert!(
        db.execute(
            "update agent_workspace_discards set inventory_hash=?1",
            ["c".repeat(64)]
        )
        .is_err()
    );
    let foreign_keys: i64 = db
        .query_row("select count(*) from pragma_foreign_key_check", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(foreign_keys, 0);
}

#[test]
fn failures_roll_back_withdrawal_and_completion_even_if_outer_transaction_commits() {
    let mut db = Connection::open_in_memory().unwrap();
    let (request, _) = fixture(&mut db);
    db.execute_batch("create trigger fail_intent before insert on agent_workspace_discards begin select raise(abort,'fixture'); end;").unwrap();
    let mut tx = db.transaction().unwrap();
    assert!(begin_in(&mut tx, &request).is_err());
    tx.commit().unwrap();
    assert_eq!(read(&db, &request).unwrap(), None);
    let state: String = db
        .query_row("select state from agent_execution_workspaces", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(state, "approved");
    db.execute_batch("drop trigger fail_intent;").unwrap();
    let mut tx = db.transaction().unwrap();
    begin_in(&mut tx, &request).unwrap();
    tx.commit().unwrap();
    db.execute_batch("create trigger fail_release before delete on agent_execution_workspaces begin select raise(abort,'fixture'); end;").unwrap();
    let mut tx = db.transaction().unwrap();
    assert!(complete_in(&mut tx, &request).is_err());
    tx.commit().unwrap();
    assert_eq!(read(&db, &request).unwrap(), Some(State::Pending));
    let count: i64 = db
        .query_row("select count(*) from agent_execution_workspaces", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(count, 1);
    db.execute_batch("drop trigger fail_release;").unwrap();
    let mut tx = db.transaction().unwrap();
    complete_in(&mut tx, &request).unwrap();
    tx.commit().unwrap();
}

#[test]
fn active_turn_and_identity_mismatch_do_not_withdraw_or_create_intent() {
    let mut db = Connection::open_in_memory().unwrap();
    let (request, _) = fixture(&mut db);
    let mut changed = request.clone();
    changed.environment_id = "different".into();
    let mut tx = db.transaction().unwrap();
    assert_eq!(begin_in(&mut tx, &changed), Err(Error::Conflict));
    tx.commit().unwrap();
    db.execute("insert into agent_items(conversation_id,ordinal,schema_version,payload_json) values(?1,0,1,?2)",
        params![request.binding.conversation_id.to_string(), serde_json::json!({"payload":{"type":"turnStarted","turnId":request.binding.turn_id}}).to_string()]).unwrap();
    let mut tx = db.transaction().unwrap();
    assert_eq!(begin_in(&mut tx, &request), Err(Error::Denied));
    tx.commit().unwrap();
    assert_eq!(read(&db, &request).unwrap(), None);
    db.execute("insert into agent_items(conversation_id,ordinal,schema_version,payload_json) values(?1,1,1,?2)",
        params![request.binding.conversation_id.to_string(), serde_json::json!({"payload":{"type":"turnFinished","turnId":request.binding.turn_id}}).to_string()]).unwrap();
    let mut tx = db.transaction().unwrap();
    assert_eq!(begin_in(&mut tx, &request).unwrap(), State::Pending);
    tx.commit().unwrap();
}
