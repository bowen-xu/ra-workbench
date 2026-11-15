use super::*;
use research_agent_core::{
    conversation::ConversationId,
    protocol::{AgentEvent, EventPayload},
};

fn fixture(db: &mut Connection) -> (Uuid, WindowUsageSample) {
    crate::storage_migration_tests::migrate_through(db, 148);
    crate::storage_integrity::backfill_migration_checksums(db).unwrap();
    let hashes = hashes(db);
    crate::storage::migrate(db).unwrap();
    assert_eq!(self::hashes(db), hashes);
    let events: Vec<AgentEvent> = serde_json::from_str(include_str!(
        "../../../../../../../../tests/fixtures/agent-events-v1.json"
    ))
    .unwrap();
    let id = *events[0].conversation_id().as_uuid();
    super::super::create_conversation(db, ConversationId::from_uuid(id), "Usage").unwrap();
    super::super::append_batch(
        db,
        ConversationId::from_uuid(id),
        Uuid::new_v4(),
        0,
        &events[..8],
    )
    .unwrap();
    let turn = match events[0].payload() {
        EventPayload::TurnStarted { turn_id, .. } => *turn_id,
        _ => unreachable!(),
    };
    append(
        db,
        id,
        EventPayload::Usage {
            turn_id: turn,
            input_tokens: Some(120),
            output_tokens: Some(30),
        },
    );
    append(
        db,
        id,
        EventPayload::TurnFinished {
            turn_id: turn,
            outcome: research_agent_core::protocol::TurnOutcome::Completed,
        },
    );
    let (turn_id, request_id, input_tokens, output_tokens) = events
        .iter()
        .rev()
        .find_map(|event| match event.payload() {
            EventPayload::ModelRequestFinished {
                turn_id,
                attempt_id,
                input_tokens,
                output_tokens,
            } => Some((*turn_id, *attempt_id, *input_tokens, *output_tokens)),
            _ => None,
        })
        .unwrap();
    let model = events
        .iter()
        .rev()
        .find_map(|event| match event.payload() {
            EventPayload::ModelResponseStarted {
                attempt_id, model, ..
            } if *attempt_id == request_id => model.clone(),
            _ => None,
        })
        .unwrap();
    (
        id,
        WindowUsageSample {
            turn_id,
            request_id,
            window_id: None,
            model: model.clone(),
            actual_model: model,
            actual_model_observed: true,
            input_tokens,
            output_tokens,
            estimated_tokens: None,
            budget: None,
            scope: Default::default(),
        },
    )
}

fn hashes(db: &Connection) -> Vec<(u64, String)> {
    db.prepare("select version,sha256 from migration_checksums where version<=148 order by version")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
}

#[test]
fn usage_migration_failure_rolls_back_and_preserves_deployed_checksums() {
    let mut db = Connection::open_in_memory().unwrap();
    crate::storage_migration_tests::migrate_through(&mut db, 148);
    crate::storage_integrity::backfill_migration_checksums(&db).unwrap();
    let before = hashes(&db);
    db.execute_batch("create trigger reject_149 before insert on schema_migrations when new.version=149 begin select raise(abort,'fault'); end;").unwrap();
    assert!(crate::storage::migrate(&mut db).is_err());
    assert_eq!(hashes(&db), before);
    assert_eq!(
        db.query_row(
            "select count(*) from sqlite_master where name='agent_context_usage_observations'",
            [],
            |row| row.get::<_, u64>(0)
        )
        .unwrap(),
        0
    );
    db.execute_batch("drop trigger reject_149").unwrap();
    crate::storage::migrate(&mut db).unwrap();
    assert_eq!(hashes(&db), before);
    assert_eq!(
        db.query_row("select count(*) from pragma_foreign_key_check", [], |row| {
            row.get::<_, u64>(0)
        })
        .unwrap(),
        0
    );
}

fn append(db: &mut Connection, id: Uuid, payload: EventPayload) {
    let seq: u64 = db
        .query_row(
            "select durable_count from agent_conversations where id=?1",
            [id.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    let event = AgentEvent::new(
        ConversationId::from_uuid(id),
        seq + 1,
        0,
        Uuid::new_v4(),
        payload,
    )
    .unwrap();
    super::super::append_batch(
        db,
        ConversationId::from_uuid(id),
        Uuid::new_v4(),
        seq,
        &[event],
    )
    .unwrap();
}

#[test]
fn upgrade_reopen_and_read_keep_actual_request_usage_separate_from_billing() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("usage.sqlite");
    let mut db = Connection::open(&path).unwrap();
    let (id, sample) = fixture(&mut db);
    let before: u64 = db
        .query_row("select count(*) from agent_items", [], |row| row.get(0))
        .unwrap();
    record(&mut db, id, &sample).unwrap();
    let value = read(&mut db, id, sample.model.clone()).unwrap();
    assert_eq!(value.status, WindowUsageStatus::Observed);
    assert_eq!(value.current_tokens, sample.observed_tokens());
    assert_ne!(value.current_tokens, Some(150)); // The journal's cumulative billing counter.
    assert_eq!(value.watermark, before);
    assert_eq!(value.observation.unwrap().sample, sample);
    let wire = serde_json::to_value(read(&mut db, id, sample.model.clone()).unwrap()).unwrap();
    assert_eq!(wire["schemaVersion"], 1);
    assert_eq!(wire["status"], "observed");
    assert!(wire["windowId"].is_null());
    assert!(wire["observation"]["sample"].get("inputTokens").is_some());
    assert!(wire["observation"]["sample"].get("input_tokens").is_none());
    drop(db);
    let mut db = Connection::open(path).unwrap();
    crate::storage::migrate(&mut db).unwrap();
    assert_eq!(
        read(&mut db, id, sample.model.clone())
            .unwrap()
            .current_tokens,
        sample.observed_tokens()
    );
    assert_eq!(
        db.query_row("select count(*) from agent_items", [], |row| row
            .get::<_, u64>(0))
            .unwrap(),
        before
    );
}

#[test]
fn actual_model_alias_requires_an_estimate_instead_of_claiming_observed_usage() {
    let mut db = Connection::open_in_memory().unwrap();
    let (id, mut sample) = fixture(&mut db);
    sample.actual_model.revision = Some("resolved alias".into());
    record(&mut db, id, &sample).unwrap();
    let unknown = read(&mut db, id, sample.model.clone()).unwrap();
    assert_eq!(unknown.status, WindowUsageStatus::Unknown);
    assert!(unknown.current_tokens.is_none());
    sample.estimated_tokens = Some(80);
    record(&mut db, id, &sample).unwrap();
    let estimated = read(&mut db, id, sample.model).unwrap();
    assert_eq!(estimated.status, WindowUsageStatus::Estimated);
    assert_eq!(estimated.current_tokens, Some(80));
}

#[test]
fn new_input_model_change_and_rollback_never_reuse_a_stale_measurement() {
    let mut db = Connection::open_in_memory().unwrap();
    let (id, sample) = fixture(&mut db);
    record(&mut db, id, &sample).unwrap();
    let mut changed = sample.model.clone();
    changed.revision = Some("new revision".into());
    let other = read(&mut db, id, changed).unwrap();
    assert_eq!(other.status, WindowUsageStatus::Unknown);
    assert!(other.current_tokens.is_none());
    append(
        &mut db,
        id,
        EventPayload::TurnStarted {
            turn_id: Uuid::new_v4(),
            context_snapshot_id: Uuid::new_v4(),
            model: sample.model.clone(),
            task_kind: Default::default(),
        },
    );
    let stale = read(&mut db, id, sample.model.clone()).unwrap();
    assert_eq!(stale.status, WindowUsageStatus::Stale);
    assert!(stale.current_tokens.is_none());
    assert!(stale.observation.is_some());
    assert!(matches!(record(&mut db, id, &sample), Err(Error::Conflict)));
    let mut db = Connection::open_in_memory().unwrap();
    let (id, sample) = fixture(&mut db);
    record(&mut db, id, &sample).unwrap();
    append(
        &mut db,
        id,
        EventPayload::ConversationRolledBack {
            turn_id: sample.turn_id,
            user_turns: 1,
        },
    );
    let rolled = read(&mut db, id, sample.model.clone()).unwrap();
    assert_eq!(rolled.rollback_epoch, 1);
    assert_eq!(rolled.status, WindowUsageStatus::Unknown);
    assert!(rolled.observation.is_none());
}

#[test]
fn invalid_window_or_counts_cannot_replace_a_committed_observation() {
    let mut db = Connection::open_in_memory().unwrap();
    let (id, sample) = fixture(&mut db);
    record(&mut db, id, &sample).unwrap();
    let mut forged = sample.clone();
    forged.window_id = Some(Uuid::new_v4());
    assert!(matches!(record(&mut db, id, &forged), Err(Error::Conflict)));
    forged.window_id = None;
    forged.input_tokens = Some(999);
    assert!(matches!(record(&mut db, id, &forged), Err(Error::Conflict)));
    assert_eq!(
        read(&mut db, id, sample.model.clone())
            .unwrap()
            .observation
            .unwrap()
            .sample,
        sample
    );
    db.execute(
        "delete from agent_conversations where id=?1",
        [id.to_string()],
    )
    .unwrap();
    assert!(matches!(
        read(&mut db, id, sample.model),
        Err(Error::NotFound)
    ));
    assert_eq!(
        db.query_row(
            "select count(*) from agent_context_usage_observations",
            [],
            |row| row.get::<_, u64>(0)
        )
        .unwrap(),
        0
    );
}
