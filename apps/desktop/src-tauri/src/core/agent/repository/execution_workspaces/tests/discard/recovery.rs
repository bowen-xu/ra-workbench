use super::super::super::discard::recovery::find;
use super::*;

fn lookup(db: &Connection, request: &Request) -> Result<Option<(Request, State)>, Error> {
    let b = &request.binding;
    find(
        db,
        b.conversation_id,
        b.request_id,
        b.manuscript_id,
        b.context_snapshot_id,
    )
}

#[test]
fn recovery_discovers_original_identity_after_reopen_and_binding_release() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("recovery.sqlite");
    let mut db = Connection::open(&path).unwrap();
    let (request, _) = fixture(&mut db);
    assert_eq!(lookup(&db, &request).unwrap(), None);
    let mut tx = db.transaction().unwrap();
    begin_in(&mut tx, &request).unwrap();
    tx.commit().unwrap();
    db.execute("update agent_conversations set archived=1", [])
        .unwrap();
    drop(db);
    let mut db = Connection::open(&path).unwrap();
    assert_eq!(
        lookup(&db, &request).unwrap(),
        Some((request.clone(), State::Pending))
    );
    assert_eq!(read(&db, &request).unwrap(), Some(State::Pending));
    let mut tx = db.transaction().unwrap();
    complete_in(&mut tx, &request).unwrap();
    tx.commit().unwrap();
    assert_eq!(
        lookup(&db, &request).unwrap(),
        Some((request.clone(), State::Complete))
    );
}

#[test]
fn recovery_requires_current_host_and_exact_submission_scope() {
    let mut db = Connection::open_in_memory().unwrap();
    let (request, _) = fixture(&mut db);
    let mut tx = db.transaction().unwrap();
    begin_in(&mut tx, &request).unwrap();
    tx.commit().unwrap();
    let b = &request.binding;
    for index in 0..4 {
        let mut ids = [
            b.conversation_id,
            b.request_id,
            b.manuscript_id,
            b.context_snapshot_id,
        ];
        ids[index] = Uuid::new_v4();
        assert_eq!(
            find(&db, ids[0], ids[1], ids[2], ids[3]),
            Err(Error::Denied)
        );
        ids[index] = Uuid::nil();
        assert_eq!(
            find(&db, ids[0], ids[1], ids[2], ids[3]),
            Err(Error::Conflict)
        );
    }
    db.execute("delete from agent_conversation_bindings", [])
        .unwrap();
    assert_eq!(lookup(&db, &request), Err(Error::Denied));
    // Losing UI visibility cannot lose the exact native recovery identity.
    assert_eq!(read(&db, &request).unwrap(), Some(State::Pending));
}

#[test]
fn recovery_does_not_discover_receipts_after_conversation_deletion() {
    let mut db = Connection::open_in_memory().unwrap();
    let (request, _) = fixture(&mut db);
    let mut tx = db.transaction().unwrap();
    begin_in(&mut tx, &request).unwrap();
    complete_in(&mut tx, &request).unwrap();
    tx.commit().unwrap();
    db.execute("delete from agent_conversations", []).unwrap();
    assert_eq!(lookup(&db, &request), Err(Error::Denied));
    assert_eq!(read(&db, &request).unwrap(), Some(State::Complete));
}
