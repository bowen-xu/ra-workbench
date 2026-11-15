use super::*;
use rusqlite::{StatementStatus, backup::Backup, params};
use sha2::{Digest, Sha256};
use std::time::{Duration, Instant};

fn append(sql: &mut Connection, id: ConversationId, values: &[AgentEvent]) {
    let mut count = sql
        .query_row(
            "select durable_count from agent_conversations where id=?1",
            [id.as_uuid().to_string()],
            |r| r.get::<_, u64>(0),
        )
        .unwrap();
    for chunk in values.chunks(500) {
        count = journal::append_batch(sql, id, Uuid::new_v4(), count, chunk).unwrap();
    }
}

fn profile_history(sql: &mut Connection, id: ConversationId) -> Vec<AgentEvent> {
    let tx = sql.transaction().unwrap();
    let mut values = Vec::new();
    let mut offset = 0;
    let mut durable_count = None;
    loop {
        let page = journal::read_history_in::<AgentEvent>(
            &tx,
            HistoryQuery::new(id, offset, 500).unwrap(),
        )
        .unwrap();
        let total = *durable_count.get_or_insert(page.durable_count);
        assert_eq!(
            page.durable_count, total,
            "profile history watermark changed"
        );
        for (index, event) in page.items.iter().enumerate() {
            assert_eq!(event.seq(), offset + index as u64 + 1);
        }
        values.extend(page.items);
        let end = values.len() as u64;
        match page.next_offset {
            Some(next) => {
                assert_eq!(next, end, "profile history cursor skipped events");
                assert!(
                    next > offset && next < total,
                    "invalid profile history cursor"
                );
                offset = next;
            }
            None => {
                assert_eq!(end, total, "profile history ended before its watermark");
                break;
            }
        }
    }
    values
}

struct Sample {
    schema: i64,
    parts: usize,
    bytes: usize,
    steps: u64,
    elapsed: Duration,
    engine: String,
    logical: [u8; 32],
    physical: [u8; 32],
    slices: usize,
    metadata: Vec<MetadataRow>,
}

#[derive(Clone, Copy)]
struct ProfileIdentity {
    conversation: ConversationId,
    turn: Uuid,
    item: Uuid,
}

fn sample(part_count: usize, target_schema: i64, identity: ProfileIdentity) -> Sample {
    // Every sample starts from an independent database and the actual numbered
    // migration prefix. Paired A/B samples share only their synthetic identities.
    let mut sql = Connection::open_in_memory().unwrap();
    crate::storage_migration_tests::migrate_through(&mut sql, target_schema);
    assert_eq!(
        sql.query_row("select max(version) from schema_migrations", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        target_schema
    );
    let ProfileIdentity {
        conversation: id,
        turn,
        item,
    } = identity;
    journal::create_conversation(&mut sql, id, "slice scaling").unwrap();
    let mut prefix = vec![P::ItemStarted {
        turn_id: turn,
        item_id: item,
        kind: summary_kind(json!([])),
    }];
    prefix.extend((0..part_count).map(|i| delta(turn, item, &i.to_string(), "x")));
    let mut expected = events(id, 0, prefix);
    append(&mut sql, id, &expected);
    let parts = (0..part_count)
        .map(|i| part(&i.to_string(), i, 1))
        .collect::<Vec<_>>();
    let terminal = events(
        id,
        expected.len() as u64,
        vec![accepted(turn, item, &"x".repeat(part_count), json!(parts))],
    )
    .pop()
    .unwrap();
    let encoded = serde_json::to_string(&terminal).unwrap();
    let engine = sql
        .query_row("select sqlite_version()", [], |r| r.get::<_, String>(0))
        .unwrap();
    let tx = sql.transaction().unwrap();
    // The safe API is present in pinned rusqlite 0.37.0. Measure the real
    // acceptance statement and its trigger subprograms, excluding commit I/O.
    let (steps, elapsed) = {
        let mut stmt=tx.prepare("insert into agent_items(conversation_id,ordinal,schema_version,payload_json) values(?1,?2,1,?3)").unwrap();
        stmt.reset_status(StatementStatus::VmStep);
        let start = Instant::now();
        stmt.execute(params![
            id.as_uuid().to_string(),
            expected.len() as u64,
            &encoded
        ])
        .unwrap();
        let elapsed = start.elapsed();
        (
            u64::try_from(stmt.get_status(StatementStatus::VmStep)).unwrap(),
            elapsed,
        )
    };
    tx.execute(
        "update agent_conversations set durable_count=?2 where id=?1",
        params![id.as_uuid().to_string(), terminal.seq()],
    )
    .unwrap();
    tx.commit().unwrap();
    expected.push(terminal);
    let actual = profile_history(&mut sql, id);
    assert_eq!(
        actual.len(),
        expected.len(),
        "schema={target_schema}, parts={part_count}: profile history length"
    );
    let difference = actual
        .iter()
        .zip(&expected)
        .position(|(actual, expected)| actual != expected);
    assert!(
        difference.is_none(),
        "schema={target_schema}, parts={part_count}: first differing event={difference:?}"
    );
    let logical = Sha256::digest(serde_json::to_vec(&actual).unwrap()).into();
    let physical = Sha256::digest(serde_json::to_vec(&physical(&sql, id)).unwrap()).into();
    let slices=sql.query_row("select count(*) from agent_journal_records where json_extract(payload_json,'$.payload.type')='storedReasoningSlice'",[],|r|r.get::<_,usize>(0)).unwrap();
    assert_eq!(slices, part_count);
    let metadata = metadata(&sql).unwrap();
    assert_eq!(metadata.len(), part_count);
    Sample {
        schema: target_schema,
        parts: part_count,
        bytes: encoded.len(),
        steps,
        elapsed,
        engine,
        logical,
        physical,
        slices,
        metadata,
    }
}

fn report_sample(s: &Sample, round: usize) {
    eprintln!(
        "{}",
        json!({"kind":"reasoningSliceProfile","engine":s.engine,
        "targetSchema":s.schema,"parts":s.parts,"canonicalBytes":s.bytes,
        "vmSteps":s.steps,"trigger_us":s.elapsed.as_micros(),"round":round,
        "debugAssertions":cfg!(debug_assertions),
        "measurement":"acceptance statement and triggers; excludes prepare and commit I/O"})
    );
}

#[test]
fn summary_slice_lookup_vm_steps_scale_with_parts_and_deltas() {
    // VM growth catches a parts-by-deltas join regression. JSON parsing work
    // inside one opcode is not counted, so report time without a brittle limit.
    let mut optimized_medians = Vec::new();
    for count in [250, 500, 1000] {
        let mut steps = Vec::new();
        let mut shortened = false;
        for round in 1..=3 {
            let identity = ProfileIdentity {
                conversation: ConversationId::from_uuid(Uuid::new_v4()),
                turn: Uuid::new_v4(),
                item: Uuid::new_v4(),
            };
            let old = sample(count, 209, identity);
            report_sample(&old, round);
            let optimized = sample(count, 210, identity);
            report_sample(&optimized, round);
            assert_eq!(old.engine, optimized.engine);
            assert_eq!(old.bytes, optimized.bytes);
            assert_eq!(old.logical, optimized.logical);
            assert_eq!(old.physical, optimized.physical);
            assert_eq!(old.slices, optimized.slices);
            assert_eq!(old.metadata.len(), optimized.metadata.len());
            let metadata_difference = old
                .metadata
                .iter()
                .zip(&optimized.metadata)
                .position(|(old, optimized)| old != optimized);
            assert!(
                metadata_difference.is_none(),
                "parts={count}, round={round}: first differing metadata row={metadata_difference:?}"
            );
            assert!(old.steps > 0 && optimized.steps > 0);
            steps.push(optimized.steps);
            if old.elapsed > Duration::from_secs(10) || optimized.elapsed > Duration::from_secs(10)
            {
                shortened = true;
                break;
            }
        }
        eprintln!(
            "{}",
            json!({"kind":"reasoningSliceProfileSamples","parts":count,
            "roundsPerSchema":steps.len(),"shortenedAfterSlowSample":shortened,
            "equivalenceChecked":["logicalEvents","physicalSlices","partsMetadata"]})
        );
        steps.sort_unstable();
        optimized_medians.push(steps[steps.len() / 2]);
    }
    for pair in optimized_medians.windows(2) {
        assert!(
            pair[1] < pair[0] * 3,
            "doubling parts/deltas exceeded linear/log-linear allowance: {} -> {} VM steps",
            pair[0],
            pair[1]
        );
    }
}

fn edge_payloads(turn: Uuid) -> Vec<P> {
    let assistant = Uuid::new_v4();
    let summary = Uuid::new_v4();
    let replaced = Uuid::new_v4();
    let empty = Uuid::new_v4();
    vec![
        P::ItemStarted {
            turn_id: turn,
            item_id: assistant,
            kind: ItemKind::AssistantMessage,
        },
        P::TextDelta {
            turn_id: turn,
            item_id: assistant,
            text: "旧🙂\0".into(),
        },
        P::ItemAccepted {
            turn_id: turn,
            item_id: assistant,
            kind: ItemKind::AssistantMessage,
            text: "旧🙂\0".into(),
            phase: None,
            source: None,
        },
        P::ItemStarted {
            turn_id: turn,
            item_id: summary,
            kind: summary_kind(json!([])),
        },
        delta(turn, summary, "0", "你\0"),
        delta(turn, summary, "1", "tail"),
        delta(turn, summary, "0", "🙂"),
        accepted(
            turn,
            summary,
            "你\0🙂tail",
            json!([part("0", 0, 8), part("1", 8, 4)]),
        ),
        P::ItemStarted {
            turn_id: turn,
            item_id: replaced,
            kind: summary_kind(json!([])),
        },
        delta(turn, replaced, "-1", "same"),
        delta(turn, replaced, "-1", "wrong"),
        accepted(turn, replaced, "samefixed", json!([part("-1", 0, 9)])),
        P::ItemStarted {
            turn_id: turn,
            item_id: empty,
            kind: summary_kind(json!([])),
        },
        delta(turn, empty, "9007199254740993", "discard"),
        accepted(turn, empty, "", json!([])),
        accepted(
            turn,
            Uuid::new_v4(),
            "done",
            json!([part("9223372036854775807", 0, 4)]),
        ),
    ]
}

fn sql208() -> Connection {
    let mut sql = Connection::open_in_memory().unwrap();
    crate::storage_migration_tests::migrate_through(&mut sql, 208);
    crate::storage_integrity::backfill_migration_checksums(&sql).unwrap();
    sql
}
type MetadataRow = (String, String, u64, String, u64, u64);
fn metadata(sql: &Connection) -> Option<Vec<MetadataRow>> {
    let exists: bool = sql
        .query_row(
            "select exists(select 1 from sqlite_schema where name='agent_reasoning_parts')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    exists.then(||sql.prepare("select conversation_id,item_id,part_ordinal,summary_index,start_byte,len_byte from agent_reasoning_parts order by conversation_id,item_id,part_ordinal")
        .unwrap().query_map([],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?)))
        .unwrap().collect::<Result<_,_>>().unwrap())
}

#[test]
fn indexed_lookup_matches_209_for_interleaving_replacement_unicode_and_empty_summaries() {
    let mut upgraded = sql208();
    let id = ConversationId::from_uuid(Uuid::new_v4());
    journal::create_conversation(&mut upgraded, id, "lookup equivalence").unwrap();
    let mut expected = events(id, 0, edge_payloads(Uuid::new_v4()));
    append(&mut upgraded, id, &expected);
    let before = physical(&upgraded, id);
    apply_migration(
        &mut upgraded,
        209,
        include_str!("../../migrations/0209_agent_reasoning_part_search.sql"),
    )
    .unwrap();
    assert_eq!(history(&mut upgraded, id), expected);
    assert_eq!(physical(&upgraded, id), before);
    let old_metadata = metadata(&upgraded);
    assert_eq!(old_metadata.as_ref().unwrap().len(), 4);
    let mut baseline = Connection::open_in_memory().unwrap();
    Backup::new(&upgraded, &mut baseline)
        .unwrap()
        .run_to_completion(256, Duration::from_millis(1), None)
        .unwrap();
    baseline.execute_batch("pragma foreign_keys=on").unwrap();
    apply_migration(
        &mut upgraded,
        210,
        include_str!("../../migrations/0210_agent_reasoning_slice_lookup.sql"),
    )
    .unwrap();
    assert_eq!(metadata(&upgraded), old_metadata);
    assert_eq!(physical(&upgraded, id), before);
    let tail = events(id, expected.len() as u64, edge_payloads(Uuid::new_v4()));
    append(&mut baseline, id, &tail);
    append(&mut upgraded, id, &tail);
    expected.extend(tail);
    assert_eq!(history(&mut baseline, id), expected);
    assert_eq!(history(&mut upgraded, id), expected);
    assert_eq!(physical(&baseline, id), physical(&upgraded, id));
    assert_eq!(metadata(&baseline), metadata(&upgraded));
    assert_fk(&upgraded);
}

type SchemaRow = (String, String, String, Option<String>);
fn schema(sql: &Connection) -> Vec<SchemaRow> {
    sql.prepare("select type,name,tbl_name,sql from sqlite_schema order by type,name")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

#[test]
fn real_209_backfill_and_210_index_migrations_roll_back_without_changing_facts() {
    let mut sql = sql208();
    let id = ConversationId::from_uuid(Uuid::new_v4());
    journal::create_conversation(&mut sql, id, "migration rollback").unwrap();
    let expected = events(id, 0, edge_payloads(Uuid::new_v4()));
    append(&mut sql, id, &expected);
    let facts = physical(&sql, id);
    for (version, source) in [
        (
            209,
            include_str!("../../migrations/0209_agent_reasoning_part_search.sql"),
        ),
        (
            210,
            include_str!("../../migrations/0210_agent_reasoning_slice_lookup.sql"),
        ),
    ] {
        sql.execute_batch(&format!("create trigger fail_lookup_migration before insert on schema_migrations when new.version={version} begin select raise(abort,'controlled lookup rollback'); end")).unwrap();
        let old_schema = schema(&sql);
        let old_metadata = metadata(&sql);
        assert!(apply_migration(&mut sql, version, source).is_err());
        assert_eq!(schema(&sql), old_schema);
        assert_eq!(metadata(&sql), old_metadata);
        assert_eq!(physical(&sql, id), facts);
        assert_eq!(history(&mut sql, id), expected);
        assert_fk(&sql);
        sql.execute_batch("drop trigger fail_lookup_migration")
            .unwrap();
        apply_migration(&mut sql, version, source).unwrap();
    }
    assert_eq!(physical(&sql, id), facts);
    assert_eq!(metadata(&sql).unwrap().len(), 4);
    assert_eq!(history(&mut sql, id), expected);
}
