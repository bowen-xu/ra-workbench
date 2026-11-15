//! Migration permits only recovery of wholly undispatched apply plans.
use super::*;

#[test]
fn pending_target_rebinding_keeps_plan_identity_and_dispatched_receipts_sealed() {
    let db = fixture();
    target(&db);
    db.execute("insert into agent_proposal_application_files(application_id,change_set_id,relative_path,target_kind) values('op','set','other.tex','disk')", []).unwrap();
    let bind = "update agent_proposal_application_files set target_kind='editor',expected_editor_session='11111111-1111-4111-8111-111111111111',expected_editor_revision='22222222-2222-4222-8222-222222222222' where relative_path='main.tex'";
    assert!(db.execute(bind, []).is_err());
    db.execute(
        "update agent_proposal_applications set status='applying'",
        [],
    )
    .unwrap();
    assert!(db.execute(bind, []).is_err());
    db.execute(
        "update agent_proposal_applications set status='recovery_required'",
        [],
    )
    .unwrap();
    db.execute(bind, []).unwrap();
    assert!(db.execute("update agent_proposal_application_files set relative_path='new.tex' where relative_path='main.tex'", []).is_err());
    assert!(db.execute("update agent_proposal_application_files set target_kind='disk',expected_editor_session=null,expected_editor_revision=null where relative_path='main.tex'", []).is_err());
    db.execute(
        "update agent_proposal_applications set status='applying'",
        [],
    )
    .unwrap();
    db.execute("update agent_proposal_application_files set phase='dispatched' where relative_path='other.tex'", []).unwrap();
    db.execute(
        "update agent_proposal_applications set status='recovery_required'",
        [],
    )
    .unwrap();
    assert!(db.execute("update agent_proposal_application_files set expected_editor_revision='33333333-3333-4333-8333-333333333333' where relative_path='main.tex'", []).is_err());
    let revision: String = db.query_row("select expected_editor_revision from agent_proposal_application_files where relative_path='main.tex'", [], |row|row.get(0)).unwrap();
    assert_eq!(revision, "22222222-2222-4222-8222-222222222222");
    assert_eq!(
        db.query_row("select count(*) from pragma_foreign_key_check", [], |r| r
            .get::<_, i64>(
            0
        ))
        .unwrap(),
        0
    );
}

#[test]
fn undo_targets_cannot_be_rebound_by_application_recovery() {
    let db = fixture();
    // The apply fixture has no targets yet; a distinct undo tests the trigger
    // without introducing a native recovery path for another direction.
    db.execute(
        "update agent_proposal_applications set status='rolled_back'",
        [],
    )
    .unwrap();
    db.execute("insert into agent_proposal_applications(id,change_set_id,direction) values('undo','set','undo')", []).unwrap();
    db.execute("insert into agent_proposal_application_files(application_id,change_set_id,relative_path,target_kind) values('undo','set','main.tex','disk')", []).unwrap();
    db.execute(
        "update agent_proposal_applications set status='recovery_required' where id='undo'",
        [],
    )
    .unwrap();
    assert!(db.execute("update agent_proposal_application_files set target_kind='editor',expected_editor_session='11111111-1111-4111-8111-111111111111',expected_editor_revision='22222222-2222-4222-8222-222222222222' where application_id='undo'", []).is_err());
}
