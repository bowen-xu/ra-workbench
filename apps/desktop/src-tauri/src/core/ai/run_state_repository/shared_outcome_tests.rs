use super::*;
use research_agent_core::protocol::TurnOutcome;

fn guarded_fixture() -> (
    tempfile::TempDir,
    crate::data_interface::SharedDatabase,
    String,
) {
    let root = tempfile::tempdir().unwrap();
    let mut sql = Connection::open(root.path().join("data.sqlite")).unwrap();
    sql.execute_batch("pragma foreign_keys=on").unwrap();
    crate::storage::migrate(&mut sql).unwrap();
    let run = Uuid::new_v4().to_string();
    sql.execute("insert into ai_runs (id,task_kind,status,provider_id,model_id,pipeline_version,prompt_version,output_schema_version,user_goal,scope_hash,input_item_count,max_input_tokens,max_output_tokens,timeout_ms,created_at,updated_at) values(?1,'paper_overview','generating','fixture-generation','fixture-v1','test','test',1,'',?2,0,100,100,1000,'now','now')", params![run, "a".repeat(64)]).unwrap();
    let db = crate::data_interface::Database::open(sql, &root.path().join("log")).unwrap();
    (root, db, run)
}

#[test]
fn guarded_terminal_commit_failure_blocks_the_data_owner() {
    let (root, db, run) = guarded_fixture();
    let mut guard = db.lock().unwrap();
    guard.execute_batch("create temp table fault_parent(id text primary key); create temp table fault_child(id text references fault_parent(id) deferrable initially deferred); create temp trigger fault_terminal after update on main.ai_runs begin insert into fault_child values('missing'); end;").unwrap();
    let error = finish_run_guarded(&mut guard, &run, "failed", None, None, None).unwrap_err();
    assert!(matches!(
        &error,
        AiError::RunTerminalUnknown {
            phase: "commit",
            safe_cause: "sql_commit_outcome_unknown",
            sqlite_extended_code: Some(787),
            ..
        }
    ));
    assert_eq!(error.code(), "ai_run_terminal_outcome_unknown");
    assert!(error.user_message().contains(&run));
    drop(guard);
    assert!(db.lock().is_err());
    drop(db);
    let sql = Connection::open(root.path().join("data.sqlite")).unwrap();
    let reopened = crate::data_interface::Database::open(sql, &root.path().join("log")).unwrap();
    let guard = reopened.lock().unwrap();
    let status: String = guard
        .query_row("select status from ai_runs where id=?1", [&run], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(status, "generating");
}

#[test]
fn guarded_terminal_rollback_failure_preserves_original_error_and_blocks() {
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization, TransactionOperation};
    let (_root, db, run) = guarded_fixture();
    let mut guard = db.lock().unwrap();
    guard.execute_batch("create temp trigger fault_terminal before update on main.ai_runs begin select raise(abort,'fixture failure'); end;").unwrap();
    guard.authorizer(Some(|context: AuthContext<'_>| match context.action {
        AuthAction::Transaction {
            operation: TransactionOperation::Rollback,
        } => Authorization::Deny,
        _ => Authorization::Allow,
    }));
    let error = finish_run_guarded(&mut guard, &run, "failed", None, None, None).unwrap_err();
    assert!(matches!(
        &error,
        AiError::RunTerminalUnknown {
            phase: "rollback",
            safe_cause: "sql_rollback_failed",
            prior_code: Some("storage_error"),
            ..
        }
    ));
    guard.authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
    guard.execute_batch("rollback").unwrap();
    drop(guard);
    assert!(db.lock().is_err());
}

#[test]
fn guarded_terminal_known_conflict_does_not_block_the_data_owner() {
    let (_root, db, run) = guarded_fixture();
    let mut guard = db.lock().unwrap();
    finish_run_guarded(&mut guard, &run, "failed", None, None, None).unwrap();
    assert!(matches!(
        finish_run_guarded(&mut guard, &run, "failed", None, None, None),
        Err(AiError::RunTerminalConflict { .. })
    ));
    drop(guard);
    assert!(db.lock().is_ok());
}

#[test]
fn late_terminal_update_does_not_rewrite_an_existing_outcome_or_stage() {
    let mut db = Connection::open_in_memory().unwrap();
    crate::storage::migrate(&mut db).unwrap();
    let run = Uuid::new_v4().to_string();
    let stage = Uuid::new_v4().to_string();
    db.execute("insert into ai_runs (id,task_kind,status,provider_id,model_id,pipeline_version,prompt_version,output_schema_version,user_goal,scope_hash,input_item_count,max_input_tokens,max_output_tokens,timeout_ms,created_at,updated_at) values(?1,'paper_overview','failed','fixture-generation','fixture-v1','test','test',1,'',?2,0,100,100,1000,'now','now')", params![run, "a".repeat(64)]).unwrap();
    db.execute("insert into ai_run_stages (id,run_id,stage_kind,attempt,status,created_at) values(?1,?2,'generating',1,'running','now')", params![stage,run]).unwrap();
    let result = finish_run(&mut db, &run, "canceled", Some("late_cancel"), None, None);
    let error = result.expect_err("a late terminal write must not report success");
    assert_eq!(error.code(), "ai_run_terminal_conflict");
    assert!(error.user_message().contains(&run));
    assert!(matches!(
        finish_run(&mut db, &run, "failed", None, None, None),
        Err(AiError::RunTerminalConflict { .. })
    ));
    let (run_status, stage_status): (String, String) = db.query_row(
        "select run.status, stage.status from ai_runs run join ai_run_stages stage on stage.run_id=run.id where run.id=?1",
        [&run], |row| Ok((row.get(0)?, row.get(1)?))).unwrap();
    assert_eq!(
        (run_status.as_str(), stage_status.as_str()),
        ("failed", "running")
    );
}

#[test]
fn terminal_projection_rejects_a_changed_shared_binding_snapshot() {
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("terminal-snapshot.sqlite");
    let mut db = Connection::open(&path).unwrap();
    crate::storage::migrate(&mut db).unwrap();
    db.pragma_update(None, "journal_mode", "WAL").unwrap();
    let run = Uuid::new_v4().to_string();
    let conversation = Uuid::new_v4().to_string();
    let request = Uuid::new_v4().to_string();
    db.execute("insert into ai_runs (id,task_kind,status,provider_id,model_id,pipeline_version,prompt_version,output_schema_version,user_goal,scope_hash,input_item_count,max_input_tokens,max_output_tokens,timeout_ms,created_at,updated_at) values(?1,'paper_overview','generating','fixture-generation','fixture-v1','test','test',1,'',?2,0,100,100,1000,'now','now')", params![run, "a".repeat(64)]).unwrap();
    db.execute(
        "insert into agent_conversations(id,title) values(?1,'fixture')",
        [&conversation],
    )
    .unwrap();
    let peer = Connection::open(&path).unwrap();
    let injected = Arc::new(AtomicBool::new(false));
    let changed = injected.clone();
    let (peer_run, peer_conversation, peer_request) =
        (run.clone(), conversation.clone(), request.clone());
    db.authorizer(Some(move |context: AuthContext<'_>| {
        if matches!(context.action, AuthAction::Update {table_name: "ai_runs", ..})
            && !changed.swap(true, Ordering::SeqCst) {
            peer.execute("insert into agent_legacy_ai_run_bindings(ai_run_id,conversation_id,request_id,task_kind) values(?1,?2,?3,'paper_overview')", params![peer_run, peer_conversation, peer_request]).unwrap();
        }
        Authorization::Allow
    }));
    let result = finish_run(&mut db, &run, "failed", None, None, None);
    crate::ai::legacy_binding_fault_tests::clear_authorizer(&db);
    assert!(injected.load(Ordering::SeqCst));
    assert!(
        matches!(result, Err(AiError::Database(rusqlite::Error::SqliteFailure(error, _)))
        if error.extended_code == rusqlite::ffi::SQLITE_BUSY_SNAPSHOT)
    );
    let (status, terminal): (String, Option<String>) = db.query_row(
        "select run.status, binding.terminal_status from ai_runs run join agent_legacy_ai_run_bindings binding on binding.ai_run_id=run.id where run.id=?1",
        [&run], |row| Ok((row.get(0)?, row.get(1)?))).unwrap();
    assert_eq!((status.as_str(), terminal), ("generating", None));
    assert!(matches!(
        finish_run(&mut db, &run, "failed", None, None, None),
        Err(AiError::TemporaryUnavailable)
    ));
}

#[test]
fn binding_catalog_read_failure_rolls_back_terminal_projection() {
    use crate::ai::legacy_binding_fault_tests::{clear_authorizer, deny_schema_after_write};
    use std::sync::atomic::Ordering;
    let mut db = rusqlite::Connection::open_in_memory().unwrap();
    crate::storage::migrate(&mut db).unwrap();
    let id = Uuid::new_v4().to_string();
    db.execute("insert into ai_runs (id,task_kind,status,provider_id,model_id,pipeline_version,prompt_version,output_schema_version,user_goal,scope_hash,input_item_count,max_input_tokens,max_output_tokens,timeout_ms,created_at,updated_at) values(?1,'paper_overview','generating','fixture-generation','fixture-v1','test','test',1,'',?2,0,100,100,1000,'now','now')", params![id, "a".repeat(64)]).unwrap();
    let denied = deny_schema_after_write(&db, "ai_runs");
    let result = finish_run(
        &mut db,
        &id,
        "failed",
        Some("fixture"),
        Some("failure"),
        None,
    );
    clear_authorizer(&db);
    assert!(denied.load(Ordering::SeqCst));
    assert!(matches!(result, Err(AiError::Database(_))));
    let status: String = db
        .query_row("select status from ai_runs where id=?1", [&id], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(status, "generating");
}

#[test]
fn terminal_projection_requires_the_matching_shared_outcome() {
    use crate::agent::repository::legacy_ai_runs::status_for_outcome;

    assert_eq!(
        status_for_outcome(TurnOutcome::Completed, true, false),
        Some("needs_review")
    );
    assert_eq!(
        status_for_outcome(TurnOutcome::Failed, false, false),
        Some("failed")
    );
    assert_eq!(
        status_for_outcome(TurnOutcome::Canceled, false, false),
        Some("canceled")
    );
    assert_eq!(
        status_for_outcome(TurnOutcome::Interrupted, false, false),
        Some("partial")
    );
    assert_eq!(
        status_for_outcome(TurnOutcome::Completed, false, false),
        None
    );
}
#[test]
fn completed_shared_turn_without_candidate_cannot_finish_as_reviewable() {
    use research_agent_core::protocol::{AgentEvent, EventPayload, ModelIdentity};

    let mut connection = rusqlite::Connection::open_in_memory().unwrap();
    crate::storage::migrate(&mut connection).unwrap();
    let run_id = Uuid::new_v4();
    let conversation = Uuid::new_v4();
    let request = Uuid::new_v4();
    let turn = Uuid::new_v4();
    let snapshot = Uuid::new_v4();
    connection
        .execute(
            "insert into agent_conversations(id,title) values(?1,'legacy')",
            [conversation.to_string()],
        )
        .unwrap();
    connection
        .execute(
            "insert into agent_submissions(conversation_id,request_id,turn_id,context_snapshot_id,schema_version,input_json) values(?1,?2,?3,?4,1,'{}')",
            params![conversation.to_string(), request.to_string(), turn.to_string(), snapshot.to_string()],
        )
        .unwrap();
    connection
        .execute(
            "insert into ai_runs (id,task_kind,status,provider_id,model_id,pipeline_version,prompt_version,output_schema_version,user_goal,scope_hash,input_item_count,max_input_tokens,max_output_tokens,timeout_ms,created_at,updated_at) values(?1,'paper_overview','generating','fixture-generation','fixture-v1','test','test',1,'',?2,0,100,100,1000,'now','now')",
            params![run_id.to_string(), "a".repeat(64)],
        )
        .unwrap();
    connection
        .execute(
            "insert into agent_legacy_ai_run_bindings(ai_run_id,conversation_id,request_id,task_kind) values(?1,?2,?3,'paper_overview')",
            params![run_id.to_string(), conversation.to_string(), request.to_string()],
        )
        .unwrap();
    let model = ModelIdentity {
        provider_id: "fixture-generation".into(),
        model_id: "fixture-v1".into(),
        revision: None,
    };
    for (ordinal, payload) in [
        EventPayload::TurnStarted {
            task_kind: research_agent_core::protocol::TaskKind::Regular,
            turn_id: turn,
            context_snapshot_id: snapshot,
            model,
        },
        EventPayload::TurnFinished {
            turn_id: turn,
            outcome: TurnOutcome::Completed,
        },
    ]
    .into_iter()
    .enumerate()
    {
        let event = AgentEvent::new(
            ConversationId::from_uuid(conversation),
            (ordinal + 1) as u64,
            1,
            request,
            payload,
        )
        .unwrap();
        connection
            .execute(
                "insert into agent_items(conversation_id,ordinal,schema_version,payload_json) values(?1,?2,1,?3)",
                params![conversation.to_string(), ordinal as i64, serde_json::to_string(&event).unwrap()],
            )
            .unwrap();
    }
    assert!(matches!(
        ensure_shared_run_outcome(&connection, &run_id.to_string(), "needs_review"),
        Err(AiError::TemporaryUnavailable)
    ));

    connection
        .execute(
            "insert into ai_run_stages
             (id, run_id, stage_kind, attempt, status, error_code, error_message,
              started_at, finished_at, created_at)
             values (?1, ?2, 'validating', 1, 'failed', 'invalid_input',
                     'structured output rejected', 'now', 'now', 'now')",
            params![Uuid::new_v4().to_string(), run_id.to_string()],
        )
        .unwrap();
    finish_run(
        &mut connection,
        &run_id.to_string(),
        "failed",
        Some("invalid_input"),
        Some("structured output rejected"),
        None,
    )
    .expect("project downstream validation failure");
    let (run_status, binding_status): (String, String) = connection
        .query_row(
            "select run.status, binding.terminal_status
             from ai_runs run
             join agent_legacy_ai_run_bindings binding on binding.ai_run_id = run.id
             where run.id = ?1",
            [run_id.to_string()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(run_status, "failed");
    assert_eq!(binding_status, "failed");
    assert!(matches!(
        finish_run(
            &mut connection,
            &run_id.to_string(),
            "failed",
            None,
            None,
            None
        ),
        Err(AiError::RunTerminalConflict { .. })
    ));
    let preserved: String = connection
        .query_row(
            "select terminal_status from agent_legacy_ai_run_bindings where ai_run_id=?1",
            [run_id.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(preserved, "failed");
}
