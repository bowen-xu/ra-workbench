//! Deployed 212 pairs must drain before 213, without refencing old submissions.
use super::*;
use research_agent_core::submission::{
    ContextSourceKind, ContextText, SubmissionInput, SubmissionRecord,
};
use sha2::{Digest, Sha256};

fn legacy_input() -> SubmissionInput {
    SubmissionInput {
        remote_compaction_v2: None,
        text: "/compact".into(),
        user_input: vec![],
        skill_instructions: vec![],
        model: research_agent_core::protocol::ModelIdentity {
            provider_id: "synthetic-migration".into(),
            model_id: "original-frozen-model".into(),
            revision: Some("v1".into()),
        },
        reasoning_effort: None,
        service_tier: None,
        context: vec![ContextText {
            source_id: "legacy-note".into(),
            kind: ContextSourceKind::Note,
            resource_id: "rollback-search-note".into(),
            revision: "saved-before-213".into(),
            anchor: Some("legacy-note:1".into()),
            text: "Frozen synthetic source before migration".into(),
        }],
        instructions: vec![],
        instruction_configs: vec![],
        skills: vec![],
    }
}

fn submission_rows(sql: &Connection, id: ConversationId) -> Vec<String> {
    rows(sql,
        "select json_array(conversation_id,request_id,turn_id,context_snapshot_id,schema_version,input_json,created_at)
         from agent_submissions where conversation_id=?1 order by request_id", id)
}

fn fixture() -> (Deployed, SubmissionRecord, Vec<String>) {
    let mut record = None;
    let fixture = deployed_with(212, |database, conversation| {
        let mut sql = database.lock().unwrap();
        let tx = sql.transaction().unwrap();
        record = Some(
            crate::agent::repository::submissions::insert_in(
                &tx,
                *conversation.as_uuid(),
                uuid(777),
                legacy_input(),
            )
            .unwrap(),
        );
        tx.commit().unwrap();
    });
    let sql = fixture.sql();
    assert_eq!(latest(&sql), 212);
    assert!(fixture.accepted_position > fixture.prefix_position);
    assert_eq!(table_count(&sql), 0);
    let stored = submission_rows(&sql, fixture.ledger.id);
    assert_eq!(stored.len(), 1);
    (fixture, record.unwrap(), stored)
}

fn table_count(sql: &Connection) -> u64 {
    sql.query_row(
        "select count(*) from sqlite_schema where type='table' and name='agent_compaction_preparations'",
        [], |row| row.get(0),
    ).unwrap()
}

fn assert_original_submission(sql: &Connection, original: &SubmissionRecord, rows: &[String]) {
    let id = ConversationId::from_uuid(original.conversation_id);
    assert_eq!(submission_rows(sql, id), rows);
    assert_eq!(
        crate::agent::repository::submissions::read(
            sql,
            original.conversation_id,
            original.request_id,
        )
        .unwrap()
        .as_ref(),
        Some(original)
    );
}

fn assert_213(
    sql: &mut Connection,
    fixture: &Deployed,
    original: &SubmissionRecord,
    rows: &[String],
) {
    assert_upgraded(sql, fixture);
    assert_original_submission(sql, original, rows);
    assert_eq!(table_count(sql), 1);
    assert_eq!(
        sql.query_row(
            "select count(*) from agent_compaction_preparations",
            [],
            |row| row.get::<_, u64>(0),
        )
        .unwrap(),
        0,
        "old preparations must not acquire an invented history fence"
    );
    assert_eq!(
        sql.query_row(
            "select sha256 from migration_checksums where version=213",
            [],
            |row| row.get::<_, String>(0),
        )
        .unwrap(),
        "8504c5569c9f2f9ffa4d6daed953a5fd4fa067bec98a35f503e55e34f5c8aff4"
    );
    assert_eq!(
        crate::agent::writing_submission::read_compaction_revision_in(
            sql,
            original.conversation_id,
            original.request_id,
        )
        .unwrap(),
        None
    );
}

fn assert_frozen_definitions() {
    for (source, expected) in [
        (
            include_str!("../../../migrations/0207_data_runtime.sql"),
            "0cf807e4c7f6463cb687267e2cb48f994527a3c2e160ae7ead5b908ec943da5d",
        ),
        (
            include_str!("../../../migrations/0208_agent_reasoning_summary.sql"),
            "9a05ad3968bce84d823783318f0f2df7ab16549bf5ecb13655b52eb0f2015b25",
        ),
        (
            include_str!("../../../migrations/0209_agent_reasoning_part_search.sql"),
            "3dad4a960435f1c7067fb0ee88fcf6715fb29c53894100287dbaed575637cca8",
        ),
        (
            include_str!("../../../migrations/0210_agent_reasoning_slice_lookup.sql"),
            "e4440f8282d2ccc822a5931aca0fda7998edb1014a43f3afd4f30e3c5d4bbe61",
        ),
        (
            include_str!("../../../migrations/0211_sql_immediate_receipts.sql"),
            "7fe10796a60436d144c7f79f084494b07b89f262e543028b45048ca2a6c29153",
        ),
        (
            include_str!("../../../migrations/0212_agent_search_rollback_visibility.sql"),
            "596c2cc65a71b8ac999f096d66ff3c0c83f9fe5a6de728630c9852432d0f7182",
        ),
    ] {
        assert_eq!(format!("{:x}", Sha256::digest(source)), expected);
    }
}

#[tokio::test]
async fn real_212_pending_pair_upgrades_to_213_and_restores_old_backup_without_refencing() {
    assert_frozen_definitions();
    let (fixture, original, stored) = fixture();
    let store = fixture.factory().unwrap();
    let database = store.shared_agent_connection();
    {
        let mut sql = database.lock().unwrap();
        assert_213(&mut sql, &fixture, &original, &stored);
        // The same SQL owner enforces both the parent identity and integer fence.
        assert!(sql.execute(
            "insert into agent_compaction_preparations(conversation_id,request_id,expected_revision) values(?1,?2,0)",
            params![original.conversation_id.to_string(), uuid(778).to_string()],
        ).is_err());
        for invalid in [-1_i64, 9_007_199_254_740_992] {
            assert!(sql.execute(
                "insert into agent_compaction_preparations(conversation_id,request_id,expected_revision) values(?1,?2,?3)",
                params![original.conversation_id.to_string(), original.request_id.to_string(), invalid],
            ).is_err());
        }
        assert_213(&mut sql, &fixture, &original, &stored);
    }
    assert!(RaftLog::open(fixture.log_path(), Some(fixture.epoch)).is_err());
    let backup = fixture.backup();
    {
        let old = Connection::open(&backup).unwrap();
        assert_eq!(latest(&old), 212);
        assert_eq!(table_count(&old), 0);
        assert_original_submission(&old, &original, &stored);
    }
    // This helper reads the original old-schema SQL prefix and paired accepted
    // suffix, then retains its single LOCK through drain, migrate and reopen.
    restore_backup(&fixture, &backup).await;
    {
        let mut restored = Connection::open(&backup).unwrap();
        assert_213(&mut restored, &fixture, &original, &stored);
    }
    database.shutdown().unwrap();
    drop(database);
    drop(store);
    let store = fixture.factory().unwrap();
    let database = store.shared_agent_connection();
    assert_213(&mut database.lock().unwrap(), &fixture, &original, &stored);
    database.shutdown().unwrap();
}

fn schema(sql: &Connection) -> Vec<(String, String, Option<String>)> {
    sql.prepare("select type,name,sql from sqlite_schema order by type,name")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

fn assert_failed_213(
    sql: &mut Connection,
    fixture: &Deployed,
    original: &SubmissionRecord,
    stored: &[String],
) {
    assert_eq!(latest(sql), 212);
    assert_eq!(checksums(sql, 212), fixture.checksums);
    assert_eq!(sql_receipt(sql, 212), fixture.sql_receipt);
    assert_original_submission(sql, original, stored);
    assert_eq!(table_count(sql), 0);
    assert_eq!(
        sql.query_row(
            "select count(*) from migration_checksums where version=213",
            [],
            |row| row.get::<_, u64>(0),
        )
        .unwrap(),
        0
    );
    assert_eq!(
        sql.query_row(
            "select epoch,materialized_position from data_runtime_state",
            [],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?)),
        )
        .unwrap(),
        (fixture.epoch.to_string(), fixture.accepted_position)
    );
    assert_eq!(
        hidden(sql, fixture.ledger.id),
        [10, 20, 30, 40].map(|turn| uuid(turn).to_string())
    );
    assert_preserved(&fixture.stored_prefix, &preserved(sql, fixture.ledger.id));
    assert_history(sql, fixture.ledger.id, &fixture.ledger.events);
}

#[tokio::test]
async fn failed_213_registration_rolls_back_ddl_after_212_drain_and_reopens_safely() {
    let (fixture, original, stored) = fixture();
    let original_schema = {
        let sql = fixture.sql();
        sql.execute_batch("create trigger reject_213 before insert on schema_migrations when new.version=213 begin select raise(abort,'controlled 213 registration failure'); end").unwrap();
        schema(&sql)
    };
    let mut first_backup = None;
    for _ in 0..2 {
        assert!(fixture.factory().is_err());
        first_backup.get_or_insert_with(|| fixture.backup());
        let mut sql = fixture.sql();
        assert_failed_213(&mut sql, &fixture, &original, &stored);
        assert_eq!(schema(&sql), original_schema);
        drop(sql);
        let log = RaftLog::open(fixture.log_path(), Some(fixture.epoch)).unwrap();
        assert_eq!(
            log.head().unwrap().accepted_position,
            fixture.accepted_position
        );
    }
    let backup = first_backup.unwrap();
    {
        let old = Connection::open(&backup).unwrap();
        assert_eq!(latest(&old), 212);
        assert_eq!(table_count(&old), 0);
        assert_original_submission(&old, &original, &stored);
        // Only remove the test-owned injection from this synthetic backup.
        old.execute_batch("drop trigger reject_213").unwrap();
    }
    restore_backup(&fixture, &backup).await;
    assert_213(
        &mut Connection::open(&backup).unwrap(),
        &fixture,
        &original,
        &stored,
    );
    fixture
        .sql()
        .execute_batch("drop trigger reject_213")
        .unwrap();
    let store = fixture.factory().unwrap();
    let database = store.shared_agent_connection();
    assert_213(&mut database.lock().unwrap(), &fixture, &original, &stored);
    database.shutdown().unwrap();
}
