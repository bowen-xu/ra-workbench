//! Original candidate publication and explicit Native settlement on the Core guard.
use super::*;
use crate::ai::generation_storage_failure::GenerationStorageUnknown;
use crate::{
    ai::structured_resource_validation::Flow,
    data_interface::{DatabaseGuard, guarded_transaction},
    hbs_storage::boundary::{self, BoundaryError},
};
use std::convert::Infallible;
#[cfg(test)]
#[path = "generated/tests.rs"]
mod tests;

#[derive(Debug, Clone)]
pub(crate) struct GeneratedCandidateCommit {
    pub(crate) id: String,
    original_payload_hash: String,
}

impl GeneratedCandidateCommit {
    pub(crate) fn payload_hash(&self) -> &str {
        debug_assert!(!self.id.is_empty());
        &self.original_payload_hash
    }
}

pub(crate) fn save_generated_candidate(
    policy: super::super::generation_policy::GenerationPolicySource,
    connection: &mut Connection,
    run_id: &str,
    bundle: &AiEvidenceBundle,
    content: &str,
) -> Result<GeneratedCandidateCommit, AiError> {
    let prepared = (policy.generated)(connection, run_id, bundle, content)?;
    save_prepared_candidate(connection, run_id, bundle, prepared)
}

pub(crate) fn save_prepared_candidate(
    connection: &mut Connection,
    run_id: &str,
    bundle: &AiEvidenceBundle,
    prepared: super::super::generation_policy::PreparedCandidate,
) -> Result<GeneratedCandidateCommit, AiError> {
    save_with(connection, run_id, bundle, prepared, Settlement::Legacy)
}

#[derive(Clone, Copy)]
enum Settlement {
    Legacy,
    Explicit,
}

fn save_with(
    connection: &mut Connection,
    run_id: &str,
    bundle: &AiEvidenceBundle,
    prepared: super::super::generation_policy::PreparedCandidate,
    settlement: Settlement,
) -> Result<GeneratedCandidateCommit, AiError> {
    let candidate_kind = prepared.kind;
    let payload_json = prepared.canonical_json;
    let payload_hash = hash_text(&payload_json);
    if let Some(existing) = connection
        .query_row(
            "select id, original_payload_hash
             from ai_candidates where run_id = ?1",
            [run_id],
            |row| {
                Ok(GeneratedCandidateCommit {
                    id: row.get(0)?,
                    original_payload_hash: row.get(1)?,
                })
            },
        )
        .optional()?
    {
        if existing.payload_hash() == payload_hash {
            return Ok(existing);
        }
        return Err(AiError::InvalidInput(
            "该运行已保存不同的候选；原结果未被覆盖".to_owned(),
        ));
    }
    let candidate_id = Uuid::new_v4().to_string();
    let transaction = connection.transaction()?;
    let candidate_identity = candidate_id.clone();
    let result = (|| {
    transaction.execute(
        "insert into ai_candidates
         (id, run_id, evidence_bundle_id, candidate_kind, schema_version,
          current_revision, review_state, validation_status, original_payload_hash,
          created_at, updated_at)
         values (?1, ?2, ?3, ?4, 1, 1, 'unreviewed',
                 'valid', ?5, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                 strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
        params![
            candidate_id,
            run_id,
            bundle.id,
            candidate_kind,
            payload_hash
        ],
    )?;
    transaction.execute(
        "insert into ai_candidate_revisions
         (id, candidate_id, revision, payload_json, payload_hash, editor_kind,
          source_run_id, parent_revision, change_reason, created_at)
         values (?1, ?2, 1, ?3, ?4, 'ai', ?5, null, 'provider_generation',
                 strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
        params![
            Uuid::new_v4().to_string(),
            candidate_id,
            payload_json,
            payload_hash,
            run_id,
        ],
    )?;
    let has_legacy_bindings: bool = transaction.query_row(
        "select exists(select 1 from sqlite_master where type='table' and name='agent_legacy_ai_run_bindings')",
        [],
        |row| row.get(0),
    )?;
    if has_legacy_bindings {
        transaction.execute(
            "update agent_legacy_ai_run_bindings set candidate_id = ?2 where ai_run_id = ?1",
            params![run_id, candidate_id],
        )?;
    }

        Ok(GeneratedCandidateCommit { id: candidate_id, original_payload_hash: payload_hash })
    })().map_err(Flow::<AiError,Infallible>::original);
    let explicit = matches!(settlement, Settlement::Explicit);
    let settled = boundary::settle_with(
        transaction,
        result,
        "write",
        explicit,
        true,
        AiError::from,
        |_| AiError::TemporaryUnavailable,
    );
    match boundary::confirm_with(connection, settled, explicit, |_| {
        AiError::TemporaryUnavailable
    }) {
        Ok(value) => Ok(value),
        Err(BoundaryError::Failure(failure)) if explicit && failure.needs_recovery() => Err(
            AiError::GenerationStorageUnknown(Box::new(GenerationStorageUnknown::from_failure(
                run_id,
                "candidate",
                Some(candidate_identity),
                failure,
            ))),
        ),
        Err(error) => Err(error.into_static()),
    }
}

pub(crate) fn save_prepared_candidate_guarded(
    connection: &mut DatabaseGuard<'_>,
    run_id: &str,
    bundle: &AiEvidenceBundle,
    prepared: super::super::generation_policy::PreparedCandidate,
) -> Result<GeneratedCandidateCommit, AiError> {
    if !connection.is_autocommit() {
        connection.require_recovery();
        return Err(AiError::GenerationStorageUnknown(Box::new(
            GenerationStorageUnknown::unsettled(
                run_id,
                "candidate",
                "begin",
                "sql_transaction_already_open",
            ),
        )));
    }
    guarded_transaction::run_mut(connection, |db| {
        let result = save_with(db, run_id, bundle, prepared, Settlement::Explicit);
        let unknown =
            !db.is_autocommit() || matches!(&result, Err(AiError::GenerationStorageUnknown(_)));
        (result, unknown)
    })
}
