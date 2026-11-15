use super::*;
#[path = "create/records.rs"]
mod records;
use crate::ai::paper_generation_admission::PaperGenerationSource;
use crate::ai::{
    credential::JsonCredentialStore,
    model::{AiTaskPreview, AiTaskSpecDraft},
};
use crate::data_interface::DatabaseGuard;
use rusqlite::{DropBehavior, Transaction, TransactionBehavior};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
const CLOUD_CONSENT_VERSION: &str = "ai-cloud-consent-v1";

#[cfg(test)]
#[path = "create_tests.rs"]
mod tests;

#[derive(Debug)]
pub(crate) struct RunCreateIdentities {
    run: Uuid,
    scope_items: Vec<Uuid>,
    conversation_candidate: Uuid,
    request: Uuid,
    intent_hash: [u8; 32],
    resolved_model: Option<ResolvedModel>,
}

#[derive(Debug, PartialEq, Eq)]
struct ResolvedModel {
    provider_id: String,
    model_id: String,
    reasoning_effort: Option<String>,
    is_cloud: bool,
}
impl RunCreateIdentities {
    pub(crate) fn new(
        draft: &AiTaskSpecDraft,
        run: Uuid,
        shared_submission: Option<(Uuid, Uuid)>,
    ) -> Result<Self, AiError> {
        if run.is_nil() || draft.scope_items.len() > 32 {
            return Err(AiError::InvalidInput("AI 运行身份或范围无效".to_owned()));
        }
        Ok(Self {
            run,
            scope_items: draft.scope_items.iter().map(|_| Uuid::new_v4()).collect(),
            conversation_candidate: Uuid::new_v4(),
            request: Uuid::new_v4(),
            intent_hash: intent_hash(draft, shared_submission)?,
            resolved_model: None,
        })
    }
    fn validate(
        &self,
        draft: &AiTaskSpecDraft,
        shared_submission: Option<(Uuid, Uuid)>,
    ) -> Result<(), AiError> {
        let mut ids = HashSet::new();
        if self.scope_items.len() != draft.scope_items.len()
            || [self.run, self.conversation_candidate, self.request]
                .into_iter()
                .chain(self.scope_items.iter().copied())
                .any(|id| id.is_nil() || !ids.insert(id))
        {
            return Err(AiError::InvalidInput("AI 运行身份或范围无效".to_owned()));
        }
        if self.intent_hash != intent_hash(draft, shared_submission)? {
            return Err(AiError::InvalidInput(
                "AI 运行请求已变化，请重新准备".to_owned(),
            ));
        }
        Ok(())
    }

    fn bind_model(&mut self, preview: &AiTaskPreview) -> Result<(), AiError> {
        let resolved = ResolvedModel {
            provider_id: preview.provider_id.clone(),
            model_id: preview.model_id.clone(),
            reasoning_effort: preview.reasoning_effort.clone(),
            is_cloud: preview.is_cloud,
        };
        match &self.resolved_model {
            Some(previous) if previous != &resolved => Err(AiError::InvalidInput(
                "AI 模型配置已变化，请重新准备".to_owned(),
            )),
            Some(_) => Ok(()),
            None => {
                self.resolved_model = Some(resolved);
                Ok(())
            }
        }
    }
}

fn intent_hash(
    draft: &AiTaskSpecDraft,
    shared_submission: Option<(Uuid, Uuid)>,
) -> Result<[u8; 32], AiError> {
    #[derive(Serialize)]
    struct Frozen<'a> {
        version: u8,
        draft: &'a AiTaskSpecDraft,
        shared_submission: Option<(Uuid, Uuid)>,
    }
    let bytes = serde_json::to_vec(&Frozen {
        version: 1,
        draft,
        shared_submission,
    })
    .map_err(|_| AiError::InvalidInput("AI 运行请求无法准备".to_owned()))?;
    Ok(Sha256::digest(bytes).into())
}

#[cfg(test)]
pub(crate) fn insert_run(
    connection: &mut Connection,
    credentials: &JsonCredentialStore,
    draft: &AiTaskSpecDraft,
) -> Result<AiRun, AiError> {
    insert_run_with_shared_submission(connection, credentials, draft, None)
}

/// Compatibility bridge until creation has a typed, durable SQL operation.
/// The guard keeps the data owner fenced and blocks it on uncertain cleanup.
pub(crate) fn insert_run_guarded(
    source: PaperGenerationSource,
    policy: super::super::generation_policy::GenerationPolicySource,
    connection: &mut DatabaseGuard<'_>,
    credentials: &JsonCredentialStore,
    draft: &AiTaskSpecDraft,
    shared_submission: Option<(Uuid, Uuid)>,
) -> Result<AiRun, AiError> {
    let mut identities = RunCreateIdentities::new(draft, Uuid::new_v4(), shared_submission)?;
    let run_id = identities.run;
    if !connection.is_autocommit() {
        connection.require_recovery();
        return Err(creation_unknown(
            run_id,
            "begin",
            "sql_transaction_already_open",
            None,
            None,
        ));
    }
    crate::data_interface::guarded_transaction::run(connection, |db| {
        insert_run_guarded_transaction(
            source,
            policy,
            db,
            credentials,
            draft,
            shared_submission,
            &mut identities,
        )
    })
}

fn insert_run_guarded_transaction(
    source: PaperGenerationSource,
    policy: super::super::generation_policy::GenerationPolicySource,
    connection: &Connection,
    credentials: &JsonCredentialStore,
    draft: &AiTaskSpecDraft,
    shared_submission: Option<(Uuid, Uuid)>,
    identities: &mut RunCreateIdentities,
) -> (Result<AiRun, AiError>, bool) {
    let run_id = identities.run;
    settle_run_transaction(connection, run_id, |transaction| {
        insert_run_in(
            source,
            policy,
            transaction,
            credentials,
            draft,
            shared_submission,
            identities,
        )
    })
}

fn settle_run_transaction(
    connection: &Connection,
    run_id: Uuid,
    operation: impl FnOnce(&Transaction<'_>) -> Result<AiRun, AiError>,
) -> (Result<AiRun, AiError>, bool) {
    let mut transaction =
        match Transaction::new_unchecked(connection, TransactionBehavior::Deferred) {
            Ok(transaction) => transaction,
            Err(error) => {
                if !connection.is_autocommit() {
                    return (
                        Err(creation_unknown(
                            run_id,
                            "begin",
                            "sql_begin_failed",
                            Some(&error),
                            None,
                        )),
                        true,
                    );
                }
                return (Err(error.into()), false);
            }
        };
    transaction.set_drop_behavior(DropBehavior::Ignore);
    let result = operation(&transaction);
    match result {
        Err(cause) => {
            let rollback = transaction.rollback();
            if rollback.is_err() || !connection.is_autocommit() {
                return (
                    Err(creation_unknown(
                        run_id,
                        "rollback",
                        if rollback.is_err() {
                            "sql_rollback_failed"
                        } else {
                            "sql_transaction_remains_open"
                        },
                        rollback.as_ref().err(),
                        Some(cause.code()),
                    )),
                    true,
                );
            }
            (Err(cause), false)
        }
        Ok(run) => {
            if let Err(error) = transaction.commit() {
                let cleanup_failed = !connection.is_autocommit()
                    && (connection.execute_batch("rollback").is_err()
                        || !connection.is_autocommit());
                return (
                    Err(creation_unknown(
                        run_id,
                        "commit",
                        if cleanup_failed {
                            "sql_commit_unknown_cleanup_failed"
                        } else {
                            "sql_commit_outcome_unknown"
                        },
                        Some(&error),
                        None,
                    )),
                    true,
                );
            }
            if !connection.is_autocommit() {
                return (
                    Err(creation_unknown(
                        run_id,
                        "commit",
                        "sql_commit_left_transaction_open",
                        None,
                        None,
                    )),
                    true,
                );
            }
            (Ok(run), false)
        }
    }
}

fn creation_unknown(
    run_id: Uuid,
    phase: &'static str,
    safe_cause: &'static str,
    error: Option<&rusqlite::Error>,
    prior_code: Option<&'static str>,
) -> AiError {
    let (sqlite_code, sqlite_extended_code) = match error {
        Some(rusqlite::Error::SqliteFailure(code, _)) => {
            (Some(format!("{:?}", code.code)), Some(code.extended_code))
        }
        _ => (None, None),
    };
    AiError::RunCreationUnknown {
        run_id,
        phase,
        safe_cause,
        prior_code,
        sqlite_code,
        sqlite_extended_code,
    }
}

#[cfg(test)]
fn insert_run_with_shared_submission(
    connection: &mut Connection,
    credentials: &JsonCredentialStore,
    draft: &AiTaskSpecDraft,
    shared_submission: Option<(Uuid, Uuid)>,
) -> Result<AiRun, AiError> {
    let mut identities = RunCreateIdentities::new(draft, Uuid::new_v4(), shared_submission)?;
    let transaction = connection.transaction()?;
    identities.validate(draft, shared_submission)?;
    let mut preview = crate::ai::candidate_test_fixture::preview(&transaction, credentials, draft)?;
    bind_confirmed_preview(draft, &mut identities, &mut preview)?;
    let run = records::insert_run_records(
        |_, _, _| Err(AiError::CapabilityUnavailable("portrait test fixture")),
        &transaction,
        draft,
        shared_submission,
        &identities,
        &preview,
        None,
    )?;
    transaction.commit()?;
    Ok(run)
}

/// The caller owns the transaction and chooses the run identity before any
/// writes. The data-interface owner can later attach its proof to this entire
/// mutation; this function neither commits nor acknowledges the operation.
pub(crate) fn insert_run_in(
    source: PaperGenerationSource,
    policy: super::super::generation_policy::GenerationPolicySource,
    transaction: &Transaction<'_>,
    credentials: &JsonCredentialStore,
    draft: &AiTaskSpecDraft,
    shared_submission: Option<(Uuid, Uuid)>,
    identities: &mut RunCreateIdentities,
) -> Result<AiRun, AiError> {
    identities.validate(draft, shared_submission)?;
    let mut preview = preview_task(policy, transaction, credentials, draft)?;
    bind_confirmed_preview(draft, identities, &mut preview)?;
    let retry_context = if draft.task_kind == "paper_overview" {
        validate_retry_candidate(policy, transaction, draft, &preview.scope_items[0])?
            .map(|(context, _)| context)
    } else {
        None
    };
    records::insert_run_records(
        source,
        transaction,
        draft,
        shared_submission,
        identities,
        &preview,
        retry_context.as_ref(),
    )
}

/// Old bounded AI tasks keep their typed candidate projections while model
/// admission and history move to the shared Agent service. Paper-analysis
/// concept turns are excluded because their workflow already supplies its own
/// conversation/request identity.
pub(crate) fn uses_shared_agent_compatibility(task_kind: &str) -> bool {
    matches!(
        task_kind,
        "selection_explanation"
            | "reference_extraction"
            | "matrix_synthesis"
            | "paper_profile_classification"
            | "paper_overview"
            | "paper_suggestions"
    )
}

fn bind_confirmed_preview(
    draft: &AiTaskSpecDraft,
    identities: &mut RunCreateIdentities,
    preview: &mut AiTaskPreview,
) -> Result<(), AiError> {
    if draft.confirmed_scope_hash.as_deref() != Some(preview.scope_hash.as_str()) {
        return Err(AiError::InvalidInput(
            "任务范围已变化，请重新确认外发预览".to_owned(),
        ));
    }
    if preview.is_cloud && draft.cloud_consent_version.as_deref() != Some(CLOUD_CONSENT_VERSION) {
        return Err(AiError::InvalidInput(
            "启动云端任务前必须确认本次外发范围".to_owned(),
        ));
    }
    identities.bind_model(preview)?;
    for (item, id) in preview.scope_items.iter_mut().zip(&identities.scope_items) {
        item.id = id.to_string();
    }
    Ok(())
}

/// Core calls this with its own synchronous capture. No connection crosses a Native wait.
pub(crate) fn insert_prepared_run_guarded(
    connection: &mut DatabaseGuard<'_>,
    draft: &AiTaskSpecDraft,
    shared_submission: Option<(Uuid, Uuid)>,
    identities: &mut RunCreateIdentities,
    prepare: impl FnOnce(&Connection) -> Result<AiTaskPreview, AiError>,
    conversation: impl FnOnce(&Transaction<'_>, &str, Uuid) -> Result<Uuid, AiError>,
) -> Result<AiRun, AiError> {
    let run_id = identities.run;
    if !connection.is_autocommit() {
        connection.require_recovery();
        return Err(creation_unknown(
            run_id,
            "begin",
            "sql_transaction_already_open",
            None,
            None,
        ));
    }
    crate::data_interface::guarded_transaction::run(connection, |db| {
        settle_run_transaction(db, run_id, |tx| {
            identities.validate(draft, shared_submission)?;
            let mut preview = prepare(tx)?;
            bind_confirmed_preview(draft, identities, &mut preview)?;
            records::insert_run_records(
                conversation,
                tx,
                draft,
                shared_submission,
                identities,
                &preview,
                None,
            )
        })
    })
}
