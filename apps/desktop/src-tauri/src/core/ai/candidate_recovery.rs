//! Original physical candidate recovery and immutable audits; domain policy is supplied.
use research_agent_core::{conversation::ConversationId, protocol::TurnOutcome};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use uuid::Uuid;

use crate::storage::StorageError;
use research_workbench_native_sdk::analysis_workflow::recovery::{
    CandidateRecoveryInputV1, CandidateRecoveryPolicyV1,
};

const RECOVERY_VERSION: i64 = 1;

pub(crate) struct RecoveryStatements<'a> {
    pub candidates: &'a str,
    pub targets: &'a str,
    pub run_stage: &'a str,
    pub run: &'a str,
    pub attempt: &'a str,
    pub turn: &'a str,
    pub concept: &'a str,
    pub workflow: &'a str,
    pub audit: &'a str,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StrandedCandidate {
    candidate_id: String,
    run_id: String,
    task_kind: String,
    candidate_kind: String,
    run_status: String,
    run_error_code: Option<String>,
    run_error_message: Option<String>,
    provider_id: String,
    model_id: String,
    context_snapshot_id: Option<String>,
    scope_hash: String,
    input_tokens: Option<i64>,
    output_tokens: Option<i64>,
    original_payload_hash: String,
    revision_payload_hash: String,
    payload_json: String,
}

struct WorkflowTarget {
    workflow_id: String,
    turn_id: String,
    attempt_id: String,
    attempt_status: String,
    attempt_error_code: Option<String>,
    attempt_error_message: Option<String>,
}

pub(crate) fn shared_candidate_completed(
    connection: &Connection,
    candidate_id: &str,
) -> Result<bool, StorageError> {
    let table_exists = |name: &str| -> rusqlite::Result<bool> {
        connection.query_row(
            "select exists(select 1 from sqlite_master where type='table' and name=?1)",
            [name],
            |row| row.get(0),
        )
    };

    let mut mapping = None;
    if table_exists("agent_paper_analysis_submissions")? {
        mapping = connection
            .query_row(
                "select conversation_id, turn_id
                 from agent_paper_analysis_submissions where candidate_id = ?1 limit 1",
                [candidate_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?;
    }
    if mapping.is_none()
        && table_exists("agent_legacy_ai_run_bindings")?
        && table_exists("agent_submissions")?
    {
        mapping = connection
            .query_row(
                "select binding.conversation_id, submission.turn_id
                 from agent_legacy_ai_run_bindings binding
                 join agent_submissions submission
                   on submission.conversation_id = binding.conversation_id
                  and submission.request_id = binding.request_id
                 where binding.candidate_id = ?1 limit 1",
                [candidate_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?;
    }
    let Some((conversation_id, turn_id)) = mapping else {
        return Ok(true);
    };
    let conversation_id = ConversationId::from_uuid(
        Uuid::parse_str(&conversation_id).map_err(|_| rusqlite::Error::InvalidQuery)?,
    );
    let turn_id = Uuid::parse_str(&turn_id).map_err(|_| rusqlite::Error::InvalidQuery)?;
    Ok(crate::agent::repository::projection::read_turn_outcome(
        connection,
        conversation_id,
        turn_id,
    )? == Some(TurnOutcome::Completed))
}

pub(crate) fn stranded_candidates(
    connection: &Connection,
    statements: &RecoveryStatements<'_>,
) -> Result<Vec<StrandedCandidate>, StorageError> {
    let mut statement = connection.prepare(statements.candidates)?;
    Ok(statement
        .query_map([], |row| {
            Ok(StrandedCandidate {
                candidate_id: row.get(0)?,
                run_id: row.get(1)?,
                task_kind: row.get(2)?,
                candidate_kind: row.get(3)?,
                run_status: row.get(4)?,
                run_error_code: row.get(5)?,
                run_error_message: row.get(6)?,
                provider_id: row.get(7)?,
                model_id: row.get(8)?,
                context_snapshot_id: row.get(9)?,
                scope_hash: row.get(10)?,
                input_tokens: row.get(11)?,
                output_tokens: row.get(12)?,
                original_payload_hash: row.get(13)?,
                revision_payload_hash: row.get(14)?,
                payload_json: row.get(15)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?)
}

impl StrandedCandidate {
    pub(crate) fn input(&self) -> CandidateRecoveryInputV1 {
        CandidateRecoveryInputV1 {
            task_kind: self.task_kind.clone(),
            candidate_kind: self.candidate_kind.clone(),
            original_payload_hash: self.original_payload_hash.clone(),
            revision_payload_hash: self.revision_payload_hash.clone(),
            payload_json: self.payload_json.clone(),
        }
    }
    pub(crate) fn candidate_id(&self) -> &str {
        &self.candidate_id
    }
}

pub(crate) fn recover_candidate(
    connection: &mut Connection,
    candidate: &StrandedCandidate,
    statements: &RecoveryStatements<'_>,
    policy: &CandidateRecoveryPolicyV1,
) -> Result<(), StorageError> {
    let transaction = connection.transaction()?;
    let workflow_targets = workflow_targets(&transaction, candidate, statements, policy)?;
    let workflow_target = (workflow_targets.len() == 1).then(|| &workflow_targets[0]);
    recover_run(&transaction, candidate, statements)?;
    if let Some(target) = workflow_target {
        recover_workflow(&transaction, candidate, target, statements, policy)?;
    }
    record_recovery(
        &transaction,
        candidate,
        workflow_target,
        workflow_targets.len(),
        statements,
        policy,
    )?;
    transaction.commit()?;
    Ok(())
}

fn workflow_targets(
    connection: &Connection,
    candidate: &StrandedCandidate,
    statements: &RecoveryStatements<'_>,
    policy: &CandidateRecoveryPolicyV1,
) -> Result<Vec<WorkflowTarget>, StorageError> {
    let Some(snapshot_id) = candidate.context_snapshot_id.as_deref() else {
        return Ok(Vec::new());
    };
    let mut statement = connection.prepare(statements.targets)?;
    Ok(statement
        .query_map(
            params![
                snapshot_id,
                candidate.provider_id,
                candidate.model_id,
                policy.expected_workflow_status,
                candidate.task_kind,
                candidate.run_status,
                policy.running_workflow_status,
            ],
            |row| {
                Ok(WorkflowTarget {
                    workflow_id: row.get(0)?,
                    turn_id: row.get(1)?,
                    attempt_id: row.get(2)?,
                    attempt_status: row.get(3)?,
                    attempt_error_code: row.get(4)?,
                    attempt_error_message: row.get(5)?,
                })
            },
        )?
        .collect::<Result<Vec<_>, _>>()?)
}

fn recover_run(
    transaction: &Transaction<'_>,
    candidate: &StrandedCandidate,
    statements: &RecoveryStatements<'_>,
) -> Result<(), StorageError> {
    transaction.execute(
        statements.run_stage,
        params![candidate.run_id, candidate.revision_payload_hash],
    )?;
    transaction.execute(statements.run, [&candidate.run_id])?;
    Ok(())
}

fn recover_workflow(
    transaction: &Transaction<'_>,
    candidate: &StrandedCandidate,
    target: &WorkflowTarget,
    statements: &RecoveryStatements<'_>,
    policy: &CandidateRecoveryPolicyV1,
) -> Result<(), StorageError> {
    transaction.execute(
        statements.attempt,
        params![
            target.attempt_id,
            candidate.run_id,
            candidate.candidate_id,
            candidate.scope_hash,
            candidate.revision_payload_hash,
            candidate.input_tokens,
            candidate.output_tokens,
        ],
    )?;
    transaction.execute(
        statements.turn,
        params![target.turn_id, candidate.candidate_id],
    )?;
    let has_concept_turn: bool =
        transaction.query_row(statements.concept, [&target.workflow_id], |row| row.get(0))?;
    let workflow_status = if has_concept_turn {
        &policy.completion_with_concept
    } else {
        &policy.completion_without_concept
    };
    transaction.execute(
        statements.workflow,
        params![target.workflow_id, workflow_status],
    )?;
    Ok(())
}

fn record_recovery(
    transaction: &Transaction<'_>,
    candidate: &StrandedCandidate,
    target: Option<&WorkflowTarget>,
    target_count: usize,
    statements: &RecoveryStatements<'_>,
    policy: &CandidateRecoveryPolicyV1,
) -> Result<(), StorageError> {
    let recovery_kind = target.map_or("run_only", |_| candidate.task_kind.as_str());
    let recovery_reason = match target_count {
        0 => &policy.recovery_reasons[0],
        1 => &policy.recovery_reasons[1],
        _ => &policy.recovery_reasons[2],
    };
    transaction.execute(
        statements.audit,
        params![
            Uuid::new_v4().to_string(),
            candidate.candidate_id,
            candidate.run_id,
            target.map(|value| value.workflow_id.as_str()),
            recovery_kind,
            recovery_reason,
            RECOVERY_VERSION,
            candidate.run_status,
            candidate.run_error_code,
            candidate.run_error_message,
            target.map(|value| value.attempt_status.as_str()),
            target.and_then(|value| value.attempt_error_code.as_deref()),
            target.and_then(|value| value.attempt_error_message.as_deref()),
        ],
    )?;
    Ok(())
}

/// Original shared-submission projection, following domain attempt interruption.
pub(crate) fn interrupt_submissions(db: &Connection) -> Result<(), rusqlite::Error> {
    db.execute_batch(
        "         update agent_paper_analysis_submissions
         set terminal_status = 'interrupted',
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         where terminal_status is null
           and exists(
               select 1 from paper_analysis_turn_attempts attempt
               where attempt.id = agent_paper_analysis_submissions.attempt_id
                 and attempt.status = 'interrupted'
           );",
    )
}
