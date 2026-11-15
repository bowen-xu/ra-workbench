//! Exercise the real tool/message SQL split, scheduled queries and cold replay.
use super::*;

#[test]
fn complete_write_reports_observed_codec_work_without_inventing_legacy_values() {
    for mode in [Mode::LegacyNormal, Mode::AutoProcess] {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(
            root.path().join(MARKER),
            b"research-assistant-synthetic-io-v2",
        )
        .unwrap();
        let result = run(
            root.path(),
            Job {
                mode,
                shape: Shape::Short,
                query_every: None,
                fragments: 128,
                fragment_bytes: 128,
            },
            false,
        )
        .unwrap();
        assert_eq!(result.serialization_cpu_us, None);
        if mode.logged() {
            let codec = result.complete_write_codec.unwrap();
            assert_eq!(codec.record_encodes, result.measured_event_count as u64);
            assert_eq!(codec.record_decodes, codec.record_encodes);
            assert_eq!(codec.command_encodes, codec.record_encodes * 2);
            assert_eq!(codec.command_round_trips, codec.record_encodes);
            assert_eq!(codec.fingerprints, codec.record_encodes * 2);
            assert_eq!(
                codec.command_encoded_bytes,
                codec.command_round_trip_bytes * 2
            );
            assert_eq!(codec.record_encoded_bytes, codec.record_decoded_bytes);
            assert_eq!(
                result.serialized_command_bytes,
                Some(codec.command_encoded_bytes)
            );
            assert_eq!(result.append_loop_codec, result.complete_write_codec);
        } else {
            assert_eq!(result.initialization_codec, None);
            assert_eq!(result.append_loop_codec, None);
            assert_eq!(result.complete_write_codec, None);
            assert_eq!(result.serialized_command_bytes, None);
        }
    }
}

fn check_storage(shape: Shape, mode: Mode, every: usize) {
    let root = tempfile::tempdir().unwrap();
    let job = Job {
        mode,
        shape,
        query_every: Some(every),
        fragments: 128,
        fragment_bytes: 128,
    };
    let data = dataset(&job);
    let counts = Arc::new(SqlCounters::default());
    let observation = Observation::new(mode);
    let (mut target, setup_acknowledgements) =
        fixture(root.path(), &job, &data, &counts, &observation);
    let mut prefix = Prefix::default();
    for mutation in &data.setup {
        prefix.accept(mutation);
    }
    let mut acknowledgements = vec![];
    let mut queries = 0;
    for (index, mutation) in data.measured.iter().enumerate() {
        acknowledgements.push(target.append(mutation, mode));
        prefix.accept(mutation);
        if (index + 1) % every == 0 {
            target.sql(|db| prefix.verify(db));
            queries += 1;
        }
    }
    assert!(
        queries > 0,
        "{shape:?}/{mode:?}/query{every} must run its scheduled SQL query"
    );
    let facts = target.sql(|db| verify(db, &data));
    assert_eq!(facts, expected(&data));
    let expected_tools = match shape {
        Shape::MultiTools => 8_u64,
        Shape::LargeToolResult => 2,
        _ => 0,
    };
    let expected_messages = if shape == Shape::EightConversations {
        8_u64
    } else {
        1
    };
    target.sql(|db|{
        let tools:u64=db.query_row("select count(*) from agent_journal_records where json_extract(payload_json,'$.payload.type')='itemAccepted' and json_extract(payload_json,'$.payload.kind.type') in ('toolCall','toolResult')",[],|row|row.get(0)).unwrap();
        assert_eq!(tools,expected_tools);
        assert_eq!(db.query_row("select count(*) from agent_messages",[],|row|row.get::<_,u64>(0)).unwrap(),expected_messages);
    });
    let ledger = Ledger {
        job: job.clone(),
        setup_acknowledgements,
        acknowledgements,
        facts,
    };
    verify_original(&mut target, &data, &ledger);
    target.finish();
    drop(target);
    // Reopen both real stores, then check complete canonical facts and every
    // original operation without allocating another ID or executing any tool.
    let cold_observation = Observation::new(mode);
    let mut cold = open(root.path(), mode, &cold_observation);
    assert_eq!(cold.sql(|db| verify(db, &data)), ledger.facts);
    verify_original(&mut cold, &data, &ledger);
    cold.finish();
    drop(cold);
}
#[test]
fn canonical_tools_and_large_results_survive_real_query_points_and_cold_original_replay_in_all_modes()
 {
    for shape in [Shape::MultiTools, Shape::LargeToolResult] {
        for mode in Mode::all() {
            for every in [16, 128] {
                check_storage(shape, mode, every);
            }
        }
    }
}
#[test]
fn eight_conversation_query16_preserves_scoped_message_bodies_terminal_facts_and_cold_operations() {
    for mode in [Mode::LegacyNormal, Mode::AutoProcess] {
        check_storage(Shape::EightConversations, mode, 16);
    }
}
