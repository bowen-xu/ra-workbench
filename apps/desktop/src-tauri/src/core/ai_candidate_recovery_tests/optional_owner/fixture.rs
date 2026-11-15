use crate::agent::repository::projection::read_turn_outcome;
use crate::{
    data_interface::{Database, journal::JournalCommand},
    storage::migrate,
};
use research_agent_core::{
    conversation::ConversationId,
    protocol::{AgentEvent, EventPayload, ModelIdentity, TurnOutcome, TurnPhase},
};
use research_data_runtime::{WriteMode, WriteOptions};
use rusqlite::{Connection, params};
use sha2::{Digest, Sha256};
use std::path::Path;
use uuid::Uuid;

fn insert_stranded_overview_for_run(connection: &Connection, error_code: &str, run: &str) {
    let payload = "{\"kind\":\"paper_overview_unit_v1\",\"schemaVersion\":1}";
    let payload_hash = format!("{:x}", Sha256::digest(payload.as_bytes()));
    connection
        .execute_batch(
            "insert into bibliographic_items
             (id, item_type, title, created_at, updated_at)
             values ('stranded-paper', 'journalArticle', 'Stranded paper', 'now', 'now');
             insert into paper_context_snapshots
             (id, paper_id, context_hash, context_schema_version, created_at)
             values ('stranded-snapshot', 'stranded-paper',
                     'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
                     1, 'now');",
        )
        .expect("paper context");
    connection
        .execute(
            "insert into ai_runs
             (id, task_kind, status, provider_id, model_id, pipeline_version, prompt_version,
              output_schema_version, user_goal, scope_hash, input_item_count, max_input_tokens,
              max_output_tokens, timeout_ms, input_tokens, output_tokens, error_code,
              error_message, created_at, finished_at, updated_at, context_snapshot_id)
             values (?3, 'paper_overview', 'failed', 'fixture-generation',
                     'fixture-v1', 'pipeline', 'prompt', 1, '', ?1, 1, 1000, 1000, 5000,
                     100, 20, ?2, 'original local failure', 'now', 'now', 'now',
                     'stranded-snapshot')",
            params!["b".repeat(64), error_code, run],
        )
        .expect("failed run");
    connection
        .execute(
            "insert into ai_run_stages
             (id, run_id, stage_kind, attempt, status, error_code, error_message,
              started_at, finished_at, created_at)
             values ('stranded-stage', ?2, 'validating', 1, 'failed', ?1,
                     'original stage failure', 'now', 'now', 'now')",
            params![error_code, run],
        )
        .expect("failed stage");
    connection
        .execute(
            "insert into ai_evidence_bundles
             (id, run_id, scope_hash, retrieval_config_version, validation_status, created_at)
             values ('stranded-bundle', ?2, ?1, 'fixture', 'valid', 'now')",
            params!["b".repeat(64), run],
        )
        .expect("bundle");
    connection
        .execute(
            "insert into ai_candidates
             (id, run_id, evidence_bundle_id, candidate_kind, schema_version, current_revision,
              review_state, validation_status, original_payload_hash, created_at, updated_at)
             values ('stranded-candidate', ?2, 'stranded-bundle',
                     'paper_overview_unit_v1', 1, 1, 'unreviewed', 'valid', ?1, 'now', 'now')",
            params![payload_hash, run],
        )
        .expect("candidate");
    connection
        .execute(
            "insert into ai_candidate_revisions
             (id, candidate_id, revision, payload_json, payload_hash, editor_kind,
              source_run_id, parent_revision, change_reason, created_at)
             values ('stranded-revision', 'stranded-candidate', 1, ?1, ?2, 'ai',
                     ?3, null, 'provider_generation', 'now')",
            params![payload, payload_hash, run],
        )
        .expect("revision");
    connection
        .execute(
            "insert into paper_analysis_workflows
             (id, paper_id, context_snapshot_id, status, provider_id, model_id,
              prompt_set_version, revision, created_at, updated_at, completed_at)
             values ('stranded-workflow', 'stranded-paper', 'stranded-snapshot', 'failed',
                     'fixture-generation', 'fixture-v1', 'paper-analysis-v1', 1,
                     'now', 'now', 'now')",
            [],
        )
        .expect("workflow");
    connection
        .execute_batch(
            "insert into paper_analysis_turns
             (id, workflow_id, ordinal, turn_kind, status, created_at, finished_at)
             values ('stranded-overview-turn', 'stranded-workflow', 0, 'paper_overview',
                     'failed', 'now', 'now');
             insert into paper_analysis_turns
             (id, workflow_id, ordinal, turn_kind, status, created_at)
             values ('stranded-concept-turn', 'stranded-workflow', 1, 'paper_concept_map',
                     'queued', 'now');",
        )
        .expect("turns");
    connection
        .execute(
            "insert into paper_analysis_turn_attempts
             (id, turn_id, attempt, status, normalized_messages_json, provider_id, model_id,
              error_code, error_message, created_at, started_at, finished_at)
             values ('stranded-attempt', 'stranded-overview-turn', 1, 'failed', '[]',
                     'fixture-generation', 'fixture-v1', ?1, 'original workflow failure',
                     'now', 'now', 'now')",
            [error_code],
        )
        .expect("attempt");
}

pub(super) fn pending_completion(
    directory: &Path,
    original_cause: bool,
) -> (String, ConversationId, Uuid) {
    let run = Uuid::new_v4().to_string();
    let conversation = ConversationId::from_uuid(Uuid::new_v4());
    let request = Uuid::new_v4();
    let turn = Uuid::new_v4();
    let mut sql = Connection::open(directory.join("research-assistant.sqlite3")).unwrap();
    migrate(&mut sql).unwrap();
    insert_stranded_overview_for_run(&sql, "storage_error", &run);
    sql.execute(
        "update ai_runs set status='validating',finished_at=null,
             error_code=case when ?2 then error_code else null end,
             error_message=case when ?2 then error_message else null end where id=?1",
        params![run, original_cause],
    )
    .unwrap();
    sql.execute_batch(
        "update ai_run_stages set status='running',finished_at=null;
         update paper_analysis_workflows set status='overview_running',completed_at=null;
         update paper_analysis_turns set status='running',finished_at=null
             where id='stranded-overview-turn';
         update paper_analysis_turn_attempts set status='running',finished_at=null;",
    )
    .unwrap();
    if !original_cause {
        sql.execute_batch("update ai_run_stages set error_code=null,error_message=null;")
            .unwrap();
    }
    sql.execute(
        "insert into agent_conversations(id,title) values(?1,'optional owner recovery')",
        [conversation.as_uuid().to_string()],
    )
    .unwrap();
    sql.execute(
        "insert into agent_submissions(conversation_id,request_id,turn_id,
             context_snapshot_id,schema_version,input_json) values(?1,?2,?3,?4,1,'{}')",
        params![
            conversation.as_uuid().to_string(),
            request.to_string(),
            turn.to_string(),
            Uuid::new_v4().to_string()
        ],
    )
    .unwrap();
    sql.execute(
        "insert into agent_legacy_ai_run_bindings
             (ai_run_id,conversation_id,request_id,task_kind,candidate_id)
             values(?1,?2,?3,'paper_overview','stranded-candidate')",
        params![run, conversation.as_uuid().to_string(), request.to_string()],
    )
    .unwrap();
    let database = Database::open(sql, &directory.join("data-log")).unwrap();
    let event = |sequence, payload| {
        AgentEvent::new(conversation, sequence, sequence, request, payload).unwrap()
    };
    for (expected, events, title, mode) in [
        (
            0,
            vec![
                event(
                    1,
                    EventPayload::TurnStarted {
                        task_kind: research_agent_core::protocol::TaskKind::Regular,
                        turn_id: turn,
                        context_snapshot_id: Uuid::new_v4(),
                        model: ModelIdentity {
                            provider_id: "fixture-generation".into(),
                            model_id: "fixture-v1".into(),
                            revision: None,
                        },
                    },
                ),
                event(
                    2,
                    EventPayload::PhaseChanged {
                        turn_id: turn,
                        phase: TurnPhase::RunningModel,
                    },
                ),
            ],
            None,
            WriteMode::Immediate,
        ),
        (
            2,
            vec![event(
                3,
                EventPayload::TurnFinished {
                    turn_id: turn,
                    outcome: TurnOutcome::Completed,
                },
            )],
            None,
            WriteMode::Automatic,
        ),
    ] {
        database
            .write(
                JournalCommand::new(
                    conversation,
                    Uuid::new_v4(),
                    expected,
                    events,
                    title,
                    1_790_000_000_000,
                )
                .unwrap(),
                WriteOptions {
                    mode,
                    ..WriteOptions::default()
                },
            )
            .unwrap();
    }
    // Deliberately leave the accepted suffix unmaterialized, as on a crash.
    drop(database);
    let observer = Connection::open(directory.join("research-assistant.sqlite3")).unwrap();
    assert_eq!(
        observer
            .query_row(
                "select materialized_position from data_runtime_state",
                [],
                |row| row.get::<_, u64>(0)
            )
            .unwrap(),
        1
    );
    assert_eq!(
        read_turn_outcome(&observer, conversation, turn).unwrap(),
        None
    );
    (run, conversation, turn)
}
