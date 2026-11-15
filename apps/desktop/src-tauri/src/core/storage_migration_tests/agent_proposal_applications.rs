use super::*;
#[path = "agent_proposal_applications/pending_targets.rs"]
mod pending_targets;
#[path = "agent_proposal_applications/upgrade.rs"]
mod upgrade;
fn fixture() -> Connection {
    let mut db = super::agent_change_sets::fixture();
    super::agent_change_sets::add_file(&db, "main.tex");
    super::agent_change_sets::add_file(&db, "other.tex");
    crate::storage::migrate(&mut db).unwrap();
    db.execute("insert into agent_proposal_applications(id,change_set_id,direction) values('op','set','apply')",[]).unwrap();
    db
}
fn target(db: &Connection) {
    db.execute("insert into agent_proposal_application_files(application_id,change_set_id,relative_path,target_kind) values('op','set','main.tex','disk')",[]).unwrap();
}
#[test]
fn journal_upgrade_retains_frozen_files_objects_and_unresolved_intent() {
    let db = fixture();
    target(&db);
    assert!(
        db.execute(
            "update agent_proposal_application_files set phase='dispatched'",
            []
        )
        .is_err()
    );
    for sql in [
        "delete from agent_change_sets",
        "delete from agent_change_set_files",
        "delete from content_objects",
        "delete from agent_proposal_applications",
        "delete from agent_proposal_application_files",
    ] {
        assert!(db.execute(sql, []).is_err(), "{sql}");
    }
    assert_eq!(
        db.query_row(
            "select count(*) from content_object_gc_candidates",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    db.execute(
        "update agent_proposal_applications set status='recovery_required'",
        [],
    )
    .unwrap();
    assert!(
        db.execute("delete from agent_proposal_applications", [])
            .is_err()
    );
    db.execute(
        "update agent_proposal_applications set status='rolled_back'",
        [],
    )
    .unwrap();
    db.execute("delete from agent_proposal_applications", [])
        .unwrap();
    assert_eq!(
        db.query_row(
            "select count(*) from agent_proposal_application_files",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    // Explicit terminal-journal cleanup releases its restriction, not proposal ownership.
    assert_eq!(
        db.query_row(
            "select count(*) from content_object_gc_candidates",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    db.execute("delete from agent_change_sets", []).unwrap();
    assert_eq!(
        db.query_row(
            "select count(*) from content_object_gc_candidates",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        2
    );
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
fn journal_limits_one_active_attempt_and_seals_target_identity() {
    let db = fixture();
    target(&db);
    assert!(db.execute("insert into agent_proposal_applications(id,change_set_id,direction) values('two','set','undo')",[]).is_err());
    for sql in [
        "update agent_proposal_applications set direction='undo'",
        "update agent_proposal_application_files set relative_path='other.tex'",
        "update agent_proposal_application_files set target_kind='editor'",
        "update agent_proposal_applications set status='unknown'",
        "update agent_proposal_application_files set phase='unknown'",
    ] {
        assert!(db.execute(sql, []).is_err(), "{sql}");
    }
    assert!(
        db.execute(
            "update agent_proposal_applications set status='applying'",
            []
        )
        .is_err()
    );
    db.execute("insert into agent_proposal_application_files(application_id,change_set_id,relative_path,target_kind) values('op','set','other.tex','disk')",[]).unwrap();
    db.execute(
        "update agent_proposal_applications set status='applying'",
        [],
    )
    .unwrap();
    assert!(
        db.execute(
            "update agent_proposal_applications set status='prepared'",
            []
        )
        .is_err()
    );
    assert!(
        db.execute(
            "update agent_proposal_applications set status='applied'",
            []
        )
        .is_err()
    );
    assert!(
        db.execute(
            "update agent_proposal_application_files set phase='acknowledged'",
            []
        )
        .is_err()
    );
    db.execute(
        "update agent_proposal_application_files set phase='dispatched'",
        [],
    )
    .unwrap();
    assert!(
        db.execute(
            "update agent_proposal_applications set status='rolled_back'",
            []
        )
        .is_err()
    );
    db.execute(
        "update agent_proposal_application_files set phase='acknowledged'",
        [],
    )
    .unwrap();
    assert!(db.execute("insert into agent_proposal_application_files(application_id,change_set_id,relative_path,target_kind) values('op','set','other.tex','disk')",[]).is_err());
    db.execute(
        "update agent_proposal_applications set status='applied'",
        [],
    )
    .unwrap();
    db.execute("insert into agent_proposal_applications(id,change_set_id,direction) values('undo','set','undo')",[]).unwrap();
    assert!(db.execute("insert into agent_proposal_application_files(application_id,change_set_id,relative_path,target_kind) values('undo','set','missing.tex','disk')",[]).is_err());
}
#[test]
fn editor_targets_require_expected_identity_and_ack_revision() {
    let db = fixture();
    assert!(db.execute("insert into agent_proposal_application_files(application_id,change_set_id,relative_path,target_kind) values('op','set','main.tex','editor')",[]).is_err());
    db.execute("insert into agent_proposal_application_files(application_id,change_set_id,relative_path,target_kind,expected_editor_session,expected_editor_revision) values('op','set','main.tex','editor',?1,?2)",rusqlite::params![uuid::Uuid::new_v4().to_string(),uuid::Uuid::new_v4().to_string()]).unwrap();
    db.execute("insert into agent_proposal_application_files(application_id,change_set_id,relative_path,target_kind) values('op','set','other.tex','disk')",[]).unwrap();
    db.execute(
        "update agent_proposal_applications set status='applying'",
        [],
    )
    .unwrap();
    db.execute(
        "update agent_proposal_application_files set phase='dispatched'",
        [],
    )
    .unwrap();
    assert!(
        db.execute(
            "update agent_proposal_application_files set phase='acknowledged'",
            []
        )
        .is_err()
    );
    db.execute("update agent_proposal_application_files set phase='acknowledged',result_editor_revision=?1 where target_kind='editor'",[uuid::Uuid::new_v4().to_string()]).unwrap();
    assert!(
        db.execute(
            "update agent_proposal_application_files set result_editor_revision='bad'",
            []
        )
        .is_err()
    );
    assert!(
        db.execute(
            "update agent_proposal_application_files set expected_editor_revision=?1",
            [uuid::Uuid::new_v4().to_string()]
        )
        .is_err()
    );
}

#[test]
fn unresolved_journal_survives_database_reopen() {
    let db = fixture();
    target(&db);
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("journal.sqlite3");
    db.execute("vacuum into ?1", [path.to_str().unwrap()])
        .unwrap();
    drop(db);
    let reopened = Connection::open(path).unwrap();
    reopened.execute_batch("pragma foreign_keys=on").unwrap();
    assert_eq!(
        reopened
            .query_row(
                "select status from agent_proposal_applications where id='op'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "prepared"
    );
    assert_eq!(reopened.query_row("select phase from agent_proposal_application_files where application_id='op' and relative_path='main.tex'",[],|r|r.get::<_,String>(0)).unwrap(),"pending");
    assert!(
        reopened
            .execute("delete from agent_proposal_applications", [])
            .is_err()
    );
    assert!(
        reopened
            .execute("delete from agent_change_sets", [])
            .is_err()
    );
}
