use super::*;
use crate::agent::repository::change_sets::publish_in;
use research_agent_core::workspace::{FileChange, WorkspaceFile, compare};
fn fixture() -> (Connection, Binding, Vec<FileChange>) {
    let mut db = Connection::open_in_memory().unwrap();
    let (request, manuscript, baseline) = setup(&mut db);
    let tx = db.transaction().unwrap();
    let binding = prepare_in(&tx, request, manuscript, &baseline).unwrap();
    tx.commit().unwrap();
    let mut current = baseline
        .files
        .iter()
        .map(|file| file.file.clone())
        .collect::<Vec<_>>();
    current[0].version.sha256 = "a".repeat(64);
    current.push(WorkspaceFile {
        path: "new.tex".into(),
        version: current[0].version.clone(),
    });
    let changes = compare(&baseline, &current).unwrap();
    for version in changes.iter().flat_map(|c| [&c.before, &c.after]).flatten() {
        db.execute("insert or ignore into content_objects(sha256,relative_path,mime_type,byte_size,created_at) values(?1,?1,'application/octet-stream',?2,'now')",params![version.sha256,version.byte_size]).unwrap();
    }
    (db, binding, changes)
}
#[test]
fn publish_is_atomic_idempotent_and_does_not_reopen_a_rejected_proposal() {
    let (mut db, binding, changes) = fixture();
    let id = Uuid::new_v4();
    let mut tx = db.transaction().unwrap();
    assert_eq!(publish_in(&mut tx, id, &binding, &changes).unwrap(), id);
    tx.rollback().unwrap();
    assert_eq!(
        db.query_row("select count(*) from agent_change_sets", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    let mut tx = db.transaction().unwrap();
    publish_in(&mut tx, id, &binding, &changes).unwrap();
    tx.commit().unwrap();
    db.execute("update agent_change_sets set status='rejected'", [])
        .unwrap();
    let mut tx = db.transaction().unwrap();
    assert_eq!(
        publish_in(&mut tx, Uuid::new_v4(), &binding, &changes).unwrap(),
        id
    );
    tx.commit().unwrap();
    assert_eq!(
        db.query_row("select status from agent_change_sets", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "rejected"
    );
    db.execute(
        "delete from agent_change_set_files where relative_path='new.tex'",
        [],
    )
    .unwrap();
    let mut tx = db.transaction().unwrap();
    assert_eq!(
        publish_in(&mut tx, id, &binding, &changes),
        Err(Error::Conflict)
    );
}
#[test]
fn invalid_sources_objects_and_mid_insert_failure_never_publish_partial_proposals() {
    let (mut db, binding, changes) = fixture();
    let mut wrong = changes.clone();
    wrong[0].source.as_mut().unwrap().revision = "wrong".into();
    let mut tx = db.transaction().unwrap();
    assert_eq!(
        publish_in(&mut tx, Uuid::new_v4(), &binding, &wrong),
        Err(Error::Conflict)
    );
    tx.rollback().unwrap();
    db.execute("update content_objects set byte_size=0", [])
        .unwrap();
    let mut tx = db.transaction().unwrap();
    assert_eq!(
        publish_in(&mut tx, Uuid::new_v4(), &binding, &changes),
        Err(Error::Conflict)
    );
    tx.rollback().unwrap();
    db.execute("update content_objects set byte_size=6", [])
        .unwrap();
    db.execute_batch("create trigger reject_second before insert on agent_change_set_files when new.relative_path='new.tex' begin select raise(abort,'fixture'); end;").unwrap();
    let mut tx = db.transaction().unwrap();
    assert!(publish_in(&mut tx, Uuid::new_v4(), &binding, &changes).is_err());
    tx.commit().unwrap();
    assert_eq!(
        db.query_row("select count(*) from agent_change_sets", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        db.query_row("select count(*) from agent_change_set_files", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn publication_rejects_noncanonical_inventory_and_mismatched_binding() {
    let (mut db, binding, changes) = fixture();
    let mut reversed = changes.clone();
    reversed.reverse();
    let mut duplicated = changes.clone();
    duplicated.push(changes[0].clone());
    let mut wrong_kind = changes.clone();
    wrong_kind[0].before = None;
    let mut tx = db.transaction().unwrap();
    for invalid in [vec![], reversed, duplicated, wrong_kind] {
        assert_eq!(
            publish_in(&mut tx, Uuid::new_v4(), &binding, &invalid),
            Err(Error::Conflict)
        );
    }
    let mut wrong_binding = binding.clone();
    wrong_binding.context_snapshot_id = Uuid::new_v4();
    assert_eq!(
        publish_in(&mut tx, Uuid::new_v4(), &wrong_binding, &changes),
        Err(Error::Conflict)
    );
    assert_eq!(
        publish_in(&mut tx, Uuid::nil(), &binding, &changes),
        Err(Error::Conflict)
    );
    tx.commit().unwrap();
    assert_eq!(
        db.query_row("select count(*) from agent_change_sets", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn released_execution_binding_preserves_frozen_proposal_and_content_references() {
    use crate::agent::repository::{change_sets, execution_workspaces::discard};
    let (mut db, binding, changes) = fixture();
    let (_, baseline) = read_for_review(&db, binding.conversation_id, binding.request_id)
        .unwrap()
        .unwrap();
    let proposal = Uuid::new_v4();
    let mut tx = db.transaction().unwrap();
    publish_in(&mut tx, proposal, &binding, &changes).unwrap();
    change_sets::reject_in(
        &tx,
        binding.conversation_id,
        binding.manuscript_id,
        proposal,
    )
    .unwrap();
    tx.commit().unwrap();
    let reference_count = |db: &Connection| {
        db.query_row("select count(*) from content_object_references", [], |r| {
            r.get::<_, i64>(0)
        })
        .unwrap()
    };
    let before = reference_count(&db);
    assert!(before > 0);
    let request = discard::Request {
        operation_id: Uuid::new_v4(),
        binding: binding.clone(),
        environment_id: baseline.identity.environment_id,
        inventory_hash: format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&changes).unwrap())
        ),
    };
    let mut tx = db.transaction().unwrap();
    discard::begin_in(&mut tx, &request).unwrap();
    tx.commit().unwrap();
    // Pending cleanup must not make already frozen proposals unreadable either.
    change_sets::read_in(
        &db,
        binding.conversation_id,
        binding.manuscript_id,
        proposal,
    )
    .unwrap();
    let mut tx = db.transaction().unwrap();
    discard::complete_in(&mut tx, &request).unwrap();
    tx.commit().unwrap();
    let saved = change_sets::read_in(
        &db,
        binding.conversation_id,
        binding.manuscript_id,
        proposal,
    )
    .unwrap();
    assert_eq!(saved.binding, binding);
    assert_eq!(saved.changes, changes);
    assert_eq!(saved.status, change_sets::ChangeSetStatus::Rejected);
    assert_eq!(reference_count(&db), before);
    assert!(
        db.execute("update agent_change_sets set baseline_json='null'", [])
            .is_err()
    );
    db.execute("delete from agent_conversation_bindings", [])
        .unwrap();
    assert!(
        change_sets::read_in(
            &db,
            binding.conversation_id,
            binding.manuscript_id,
            proposal
        )
        .is_err()
    );
}
