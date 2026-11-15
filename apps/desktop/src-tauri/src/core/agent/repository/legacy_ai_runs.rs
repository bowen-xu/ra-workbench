//! Compatibility identity mapping for old AI Run projections.
//!
//! This table deliberately contains no model output or credentials. It gives
//! legacy candidate workflows a durable shared-conversation identity while
//! their business projections are migrated to Agent turns.

use research_agent_core::conversation::ConversationId;
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Binding {
    pub ai_run_id: Uuid,
    pub conversation_id: ConversationId,
    pub request_id: Uuid,
    pub candidate_id: Option<String>,
    pub terminal_status: Option<String>,
}

pub fn insert_in(
    tx: &Transaction<'_>,
    ai_run_id: Uuid,
    task_kind: &str,
) -> rusqlite::Result<Binding> {
    insert_in_with_ids(tx, ai_run_id, task_kind, Uuid::new_v4(), Uuid::new_v4())
}

pub(crate) fn insert_in_with_ids(
    tx: &Transaction<'_>,
    ai_run_id: Uuid,
    task_kind: &str,
    conversation_id: Uuid,
    request_id: Uuid,
) -> rusqlite::Result<Binding> {
    tx.execute(
        "insert into agent_conversations(id,title) values(?1,?2)",
        params![conversation_id.to_string(), format!("AI {task_kind}")],
    )?;
    insert_existing_in(tx, ai_run_id, task_kind, conversation_id, request_id)
}

pub(crate) fn insert_existing_in(
    tx: &Transaction<'_>,
    ai_run_id: Uuid,
    task_kind: &str,
    conversation_id: Uuid,
    request_id: Uuid,
) -> rusqlite::Result<Binding> {
    tx.execute(
        "insert into agent_legacy_ai_run_bindings(ai_run_id,conversation_id,request_id,task_kind)
         values(?1,?2,?3,?4)",
        params![
            ai_run_id.to_string(),
            conversation_id.to_string(),
            request_id.to_string(),
            task_kind
        ],
    )?;
    Ok(Binding {
        ai_run_id,
        conversation_id: ConversationId::from_uuid(conversation_id),
        request_id,
        candidate_id: None,
        terminal_status: None,
    })
}

pub fn read(connection: &Connection, ai_run_id: Uuid) -> rusqlite::Result<Option<Binding>> {
    connection
        .query_row(
            "select conversation_id,request_id,candidate_id,terminal_status from agent_legacy_ai_run_bindings where ai_run_id=?1",
            [ai_run_id.to_string()],
            |row| {
                let conversation = Uuid::parse_str(&row.get::<_, String>(0)?)
                    .map_err(|_| rusqlite::Error::InvalidQuery)?;
                let request = Uuid::parse_str(&row.get::<_, String>(1)?)
                    .map_err(|_| rusqlite::Error::InvalidQuery)?;
                Ok(Binding {
                    ai_run_id,
                    conversation_id: ConversationId::from_uuid(conversation),
                    request_id: request,
                    candidate_id: row.get(2)?,
                    terminal_status: row.get(3)?,
                })
            },
        )
        .optional()
}

/// Reconciles migrated legacy runs whose shared turn reached a terminal state
/// while the old business owner was unavailable. The Agent journal remains the
/// source of truth; this only advances an active legacy projection and never
/// replays a model request or creates a candidate.
fn startup_targets(connection: &Connection) -> rusqlite::Result<Vec<(Uuid, &'static str)>> {
    let has_bindings: bool = connection.query_row(
        "select exists(select 1 from sqlite_master where type='table' and name='agent_legacy_ai_run_bindings')",
        [],
        |row| row.get(0),
    )?;
    if !has_bindings {
        return Ok(Vec::new());
    }
    let mut statement = connection.prepare(
        "select binding.ai_run_id, binding.candidate_id, submission.turn_id,
                binding.conversation_id,
                exists(
                    select 1 from ai_run_stages stage
                    where stage.run_id = binding.ai_run_id
                      and stage.stage_kind = 'validating'
                      and stage.status = 'failed'
                )
         from agent_legacy_ai_run_bindings binding
         join agent_submissions submission
           on submission.conversation_id = binding.conversation_id
          and submission.request_id = binding.request_id
         join ai_runs run on run.id = binding.ai_run_id
         where run.status in ('queued', 'resolving_scope', 'retrieving',
             'extracting_evidence', 'generating', 'validating')",
    )?;
    let candidates = statement
        .query_map([], |row| {
            Ok((
                Uuid::parse_str(&row.get::<_, String>(0)?)
                    .map_err(|_| rusqlite::Error::InvalidQuery)?,
                row.get::<_, Option<String>>(1)?,
                Uuid::parse_str(&row.get::<_, String>(2)?)
                    .map_err(|_| rusqlite::Error::InvalidQuery)?,
                Uuid::parse_str(&row.get::<_, String>(3)?)
                    .map_err(|_| rusqlite::Error::InvalidQuery)?,
                row.get::<_, bool>(4)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    drop(statement);

    let mut targets = Vec::new();
    for (run_id, candidate_id, turn_id, conversation_id, validation_failed) in candidates {
        let Some(outcome) = crate::agent::repository::projection::read_turn_outcome(
            connection,
            ConversationId::from_uuid(conversation_id),
            turn_id,
        )?
        else {
            continue;
        };
        let Some(status) = status_for_outcome(outcome, candidate_id.is_some(), validation_failed)
        else {
            continue;
        };
        targets.push((run_id, status));
    }
    Ok(targets)
}

pub(crate) fn reconcile_on_startup_guarded(
    guard: &mut crate::data_interface::DatabaseGuard<'_>,
) -> Result<usize, crate::ai::error::AiError> {
    let targets = startup_targets(guard)?;
    for (run_id, status) in &targets {
        crate::ai::finish_reconciled_run_guarded(guard, &run_id.to_string(), status)?;
    }
    Ok(targets.len())
}

#[cfg(test)]
fn reconcile_on_startup(connection: &mut Connection) -> rusqlite::Result<usize> {
    let targets = startup_targets(connection)?;
    for (run_id, status) in &targets {
        crate::ai::finish_reconciled_run(connection, &run_id.to_string(), status)
            .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
    }
    Ok(targets.len())
}

pub(crate) fn status_for_outcome(
    outcome: research_agent_core::protocol::TurnOutcome,
    has_candidate: bool,
    validation_failed: bool,
) -> Option<&'static str> {
    match outcome {
        research_agent_core::protocol::TurnOutcome::Completed if has_candidate => {
            Some("needs_review")
        }
        // The shared turn owns model execution only. A model response can be
        // complete while the legacy business adapter rejects its structured
        // payload. Keep the shared journal truthful and project the persisted
        // validation-stage failure into the compatibility run.
        research_agent_core::protocol::TurnOutcome::Completed if validation_failed => {
            Some("failed")
        }
        research_agent_core::protocol::TurnOutcome::Failed => Some("failed"),
        research_agent_core::protocol::TurnOutcome::Canceled => Some("canceled"),
        research_agent_core::protocol::TurnOutcome::Interrupted => Some("partial"),
        research_agent_core::protocol::TurnOutcome::Completed => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binding_ids_are_independent_and_round_trip() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "create table ai_runs(id text primary key);
                 create table agent_conversations(id text primary key,title text not null);
                 create table agent_legacy_ai_run_bindings(
                   ai_run_id text primary key, conversation_id text not null,
                   request_id text not null, task_kind text not null,
                   candidate_id text, terminal_status text,
                   unique(conversation_id,request_id));",
            )
            .unwrap();
        let run = Uuid::new_v4();
        connection
            .execute("insert into ai_runs(id) values(?1)", [run.to_string()])
            .unwrap();
        let tx = connection.unchecked_transaction().unwrap();
        let binding = insert_in(&tx, run, "paper_overview").unwrap();
        tx.commit().unwrap();
        let loaded = read(&connection, run).unwrap().unwrap();
        assert_eq!(loaded, binding);
        assert_ne!(loaded.ai_run_id, *loaded.conversation_id.as_uuid());
        assert_eq!(loaded.candidate_id, None);
        assert_eq!(loaded.terminal_status, None);
    }

    #[test]
    fn terminal_outcome_requires_candidate_for_reviewable_completion() {
        use research_agent_core::protocol::TurnOutcome;

        assert_eq!(
            status_for_outcome(TurnOutcome::Completed, true, false),
            Some("needs_review")
        );
        assert_eq!(
            status_for_outcome(TurnOutcome::Completed, false, false),
            None
        );
        assert_eq!(
            status_for_outcome(TurnOutcome::Completed, false, true),
            Some("failed")
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
    }

    #[test]
    fn startup_reconciles_failed_shared_turn_without_replaying_it() {
        use crate::storage::migrate;
        use research_agent_core::protocol::{AgentEvent, EventPayload, ModelIdentity, TurnPhase};

        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("data.sqlite");
        let log = directory.path().join("log");
        let mut db = Connection::open(&path).unwrap();
        db.execute_batch("pragma foreign_keys=on").unwrap();
        migrate(&mut db).unwrap();
        let run = Uuid::new_v4();
        let conversation = Uuid::new_v4();
        let request = Uuid::new_v4();
        let turn = Uuid::new_v4();
        let context = Uuid::new_v4();
        db.execute(
            "insert into ai_runs
             (id,task_kind,status,provider_id,model_id,pipeline_version,prompt_version,
              output_schema_version,user_goal,scope_hash,input_item_count,max_input_tokens,
              max_output_tokens,timeout_ms,created_at,updated_at)
             values(?1,'paper_overview','generating','fixture-generation','fixture-v1',
                    'test','test',1,'',?2,0,100,100,1000,'now','now')",
            params![run.to_string(), "a".repeat(64)],
        )
        .unwrap();
        db.execute(
            "insert into agent_conversations(id,title) values(?1,'legacy')",
            [conversation.to_string()],
        )
        .unwrap();
        db.execute(
            "insert into agent_submissions
             (conversation_id,request_id,turn_id,context_snapshot_id,schema_version,input_json)
             values(?1,?2,?3,?4,1,'{}')",
            params![
                conversation.to_string(),
                request.to_string(),
                turn.to_string(),
                context.to_string()
            ],
        )
        .unwrap();
        db.execute(
            "insert into agent_legacy_ai_run_bindings
             (ai_run_id,conversation_id,request_id,task_kind)
             values(?1,?2,?3,'paper_overview')",
            params![
                run.to_string(),
                conversation.to_string(),
                request.to_string()
            ],
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
                context_snapshot_id: context,
                model,
            },
            EventPayload::PhaseChanged {
                turn_id: turn,
                phase: TurnPhase::RunningModel,
            },
            EventPayload::TurnFinished {
                turn_id: turn,
                outcome: research_agent_core::protocol::TurnOutcome::Failed,
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
            db.execute(
                "insert into agent_items(conversation_id,ordinal,schema_version,payload_json)
                 values(?1,?2,1,?3)",
                params![
                    conversation.to_string(),
                    ordinal as i64,
                    serde_json::to_string(&event).unwrap()
                ],
            )
            .unwrap();
        }

        let original_history: Vec<String> = db
            .prepare(
                "select payload_json from agent_items where conversation_id=?1 order by ordinal",
            )
            .unwrap()
            .query_map([conversation.to_string()], |row| row.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(original_history.len(), 3);
        let owner = crate::data_interface::Database::open(db, &log).unwrap();
        let mut guard = owner.lock().unwrap();
        guard.execute_batch("create temp table fault_parent(id text primary key); create temp table fault_child(id text references fault_parent(id) deferrable initially deferred); create temp trigger fault_terminal after update on main.ai_runs begin insert into fault_child values('missing'); end;").unwrap();
        let error = crate::ai::reconcile_legacy_runs_on_startup(&mut guard).unwrap_err();
        assert!(
            matches!(&error, research_agent_core::conversation::StoreError::Persistence {diagnostic}
            if diagnostic.contains("ai_run_terminal_outcome_unknown") && diagnostic.contains(&run.to_string()))
        );
        drop(guard);
        assert!(owner.lock().is_err());
        drop(owner);
        let db = Connection::open(&path).unwrap();
        let owner = crate::data_interface::Database::open(db, &log).unwrap();
        let mut guard = owner.lock().unwrap();
        assert_eq!(
            crate::ai::reconcile_legacy_runs_on_startup(&mut guard).unwrap(),
            1
        );
        let status: String = guard
            .query_row(
                "select status from ai_runs where id=?1",
                [run.to_string()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(status, "failed");
        let binding_status: String = guard
            .query_row(
                "select terminal_status from agent_legacy_ai_run_bindings where ai_run_id=?1",
                [run.to_string()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(binding_status, "failed");
        assert_eq!(
            crate::ai::reconcile_legacy_runs_on_startup(&mut guard).unwrap(),
            0
        );
        let history: Vec<String> = guard
            .prepare(
                "select payload_json from agent_items where conversation_id=?1 order by ordinal",
            )
            .unwrap()
            .query_map([conversation.to_string()], |row| row.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(history, original_history);
    }

    #[test]
    fn startup_reconciles_failed_business_validation_after_completed_turn() {
        use crate::storage::migrate;
        use research_agent_core::protocol::{AgentEvent, EventPayload, ModelIdentity, TurnOutcome};

        let mut db = Connection::open_in_memory().unwrap();
        migrate(&mut db).unwrap();
        let run = Uuid::new_v4();
        let conversation = Uuid::new_v4();
        let request = Uuid::new_v4();
        let turn = Uuid::new_v4();
        let context = Uuid::new_v4();
        db.execute(
            "insert into ai_runs
             (id,task_kind,status,provider_id,model_id,pipeline_version,prompt_version,
              output_schema_version,user_goal,scope_hash,input_item_count,max_input_tokens,
              max_output_tokens,timeout_ms,created_at,updated_at)
             values(?1,'selection_explanation','validating','fixture-generation','fixture-v1',
                    'test','test',1,'',?2,0,100,100,1000,'now','now')",
            params![run.to_string(), "a".repeat(64)],
        )
        .unwrap();
        db.execute(
            "insert into ai_run_stages
             (id,run_id,stage_kind,attempt,status,error_code,error_message,
              started_at,finished_at,created_at)
             values(?1,?2,'validating',1,'failed','invalid_input',
                    'structured output rejected','now','now','now')",
            params![Uuid::new_v4().to_string(), run.to_string()],
        )
        .unwrap();
        db.execute(
            "insert into agent_conversations(id,title) values(?1,'legacy')",
            [conversation.to_string()],
        )
        .unwrap();
        db.execute(
            "insert into agent_submissions
             (conversation_id,request_id,turn_id,context_snapshot_id,schema_version,input_json)
             values(?1,?2,?3,?4,1,'{}')",
            params![
                conversation.to_string(),
                request.to_string(),
                turn.to_string(),
                context.to_string()
            ],
        )
        .unwrap();
        db.execute(
            "insert into agent_legacy_ai_run_bindings
             (ai_run_id,conversation_id,request_id,task_kind)
             values(?1,?2,?3,'selection_explanation')",
            params![
                run.to_string(),
                conversation.to_string(),
                request.to_string()
            ],
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
                context_snapshot_id: context,
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
            db.execute(
                "insert into agent_items(conversation_id,ordinal,schema_version,payload_json)
                 values(?1,?2,1,?3)",
                params![
                    conversation.to_string(),
                    ordinal as i64,
                    serde_json::to_string(&event).unwrap()
                ],
            )
            .unwrap();
        }

        assert_eq!(reconcile_on_startup(&mut db).unwrap(), 1);
        let (run_status, binding_status): (String, String) = db
            .query_row(
                "select run.status, binding.terminal_status
                 from ai_runs run
                 join agent_legacy_ai_run_bindings binding on binding.ai_run_id = run.id
                 where run.id = ?1",
                [run.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(run_status, "failed");
        assert_eq!(binding_status, "failed");
        assert_eq!(reconcile_on_startup(&mut db).unwrap(), 0);
    }
}
