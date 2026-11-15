use super::{
    candidate_repository::{get_candidate, list_candidates},
    candidate_test_fixture::{
        insert_selection_source, save as save_generated_candidate, seed, selection_payload,
    },
};
fn database() -> rusqlite::Connection {
    let mut db = rusqlite::Connection::open_in_memory().unwrap();
    crate::storage::migrate(&mut db).unwrap();
    db
}

#[test]
fn committed_candidate_does_not_depend_on_optional_view_assembly() {
    let mut connection = database();
    let (anchor_id, _) = insert_selection_source(&connection);
    let (run, bundle) = seed(&connection, &anchor_id);
    let content = selection_payload("候选提交不能依赖可选摘要读取。").to_string();
    connection
        .execute_batch("drop table ai_candidate_summary_versions;")
        .expect("inject optional view failure");

    let committed = save_generated_candidate(&mut connection, &run.id, &bundle, &content)
        .expect("candidate commit");
    assert!(get_candidate(&connection, &committed.id).is_err());
    assert!(list_candidates(&connection, None, None).is_err());
    let repeated = save_generated_candidate(&mut connection, &run.id, &bundle, &content)
        .expect("idempotent candidate commit");

    assert_eq!(repeated.id, committed.id);
    assert_eq!(repeated.payload_hash(), committed.payload_hash());
    assert_eq!(
        connection
            .query_row(
                "select count(*) from ai_candidates where run_id = ?1",
                [&run.id],
                |row| row.get::<_, i64>(0),
            )
            .expect("candidate count"),
        1
    );
    assert_eq!(
        connection
            .query_row(
                "select count(*) from ai_candidate_revisions where candidate_id = ?1",
                [&committed.id],
                |row| row.get::<_, i64>(0),
            )
            .expect("revision count"),
        1
    );
}
