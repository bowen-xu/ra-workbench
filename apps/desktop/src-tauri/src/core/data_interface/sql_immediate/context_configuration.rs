//! Original configuration edits and their receipt share the SQL owner.
use super::{
    Database, ReadFence, ReceiptIdentity, SqlImmediateDiagnostic, SqlImmediateFailure,
    SqlImmediateReceipt, SqlImmediateStatus, Transaction, admission_error, diagnostic, receipts,
    unmanaged,
};
use crate::agent::context_session::{Error, configuration};
use crate::ai::paper_reading_source::PaperReadingSource;
use research_agent_core::conversation::CatalogError;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use uuid::Uuid;

const FAMILY: &str = "agent.context.configuration.save";
const SEED_FAMILY: &str = "agent.context.configuration.seed";

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ContextConfigurationSeedIntent {
    pub operation_id: Uuid,
    pub conversation_id: Uuid,
    pub expected_epoch: Uuid,
}

impl ContextConfigurationSeedIntent {
    fn identity(&self) -> Result<ReceiptIdentity, SqlImmediateFailure> {
        let invalid = || {
            SqlImmediateFailure::Invalid(Box::new(SqlImmediateDiagnostic::for_operation(
                self.operation_id,
                None,
                "context-configuration-seed-validate",
                "invalid_seed_intent",
            )))
        };
        if self.operation_id.is_nil()
            || self.conversation_id.is_nil()
            || self.expected_epoch.is_nil()
        {
            return Err(invalid());
        }
        let bytes = serde_json::to_vec(&(SEED_FAMILY, 1u32, self)).map_err(|_| invalid())?;
        Ok(ReceiptIdentity {
            operation_id: self.operation_id,
            kind: SEED_FAMILY,
            version: 1,
            fingerprint: Sha256::digest(bytes).into(),
        })
    }
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ContextConfigurationIntent {
    pub operation_id: Uuid,
    pub conversation_id: Uuid,
    pub expected_epoch: Uuid,
    pub expected_revision: i64,
    pub state: Value,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ContextConfigurationEnvironmentRequest {
    pub operation_id: Uuid,
    pub conversation_id: Uuid,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
pub(crate) enum ContextConfigurationFailure {
    Domain(Error),
    Storage(SqlImmediateFailure),
}

impl ContextConfigurationIntent {
    fn identity(&self) -> Result<ReceiptIdentity, SqlImmediateFailure> {
        let invalid = || {
            SqlImmediateFailure::Invalid(Box::new(SqlImmediateDiagnostic::for_operation(
                self.operation_id,
                None,
                "context-configuration-validate",
                "invalid_configuration_intent",
            )))
        };
        if self.operation_id.is_nil()
            || self.conversation_id.is_nil()
            || self.expected_epoch.is_nil()
            || self.expected_revision < 0
            || self.expected_revision >= 9_007_199_254_740_991
            || self.state.to_string().len() > 1_048_576
        {
            return Err(invalid());
        }
        let bytes = serde_json::to_vec(&(FAMILY, 1u32, self)).map_err(|_| invalid())?;
        Ok(ReceiptIdentity {
            operation_id: self.operation_id,
            kind: FAMILY,
            version: 1,
            fingerprint: Sha256::digest(bytes).into(),
        })
    }
}

impl Database {
    pub(crate) fn seed_context_configuration(
        &self,
        intent: &ContextConfigurationSeedIntent,
        writing_source: Option<crate::agent::writing_resources::WritingResourceSource>,
        reading_source: impl Into<Option<PaperReadingSource>>,
        analysis_source: impl Into<Option<crate::ai::analysis_source::PaperAnalysisSource>>,
    ) -> Result<SqlImmediateReceipt, ContextConfigurationFailure> {
        self.seed_context_configuration_owned(
            intent,
            #[cfg(test)]
            super::Fault::None,
            writing_source,
            reading_source,
            analysis_source,
        )
    }

    fn seed_context_configuration_owned(
        &self,
        intent: &ContextConfigurationSeedIntent,
        #[cfg(test)] fault: super::Fault,
        writing_source: Option<crate::agent::writing_resources::WritingResourceSource>,
        reading_source: impl Into<Option<PaperReadingSource>>,
        analysis_source: impl Into<Option<crate::ai::analysis_source::PaperAnalysisSource>>,
    ) -> Result<SqlImmediateReceipt, ContextConfigurationFailure> {
        let reading_source = reading_source.into();
        let analysis_source = analysis_source.into();
        self.seed_context_configuration_with(
            intent,
            #[cfg(test)]
            fault,
            |tx| {
                crate::ai::seed_existing_structured_context(
                    analysis_source,
                    reading_source,
                    tx,
                    intent.conversation_id,
                    writing_source,
                )
            },
        )
    }

    fn seed_context_configuration_with(
        &self,
        intent: &ContextConfigurationSeedIntent,
        #[cfg(test)] fault: super::Fault,
        seed: impl FnOnce(&rusqlite::Transaction<'_>) -> Result<(), crate::ai::error::AiError>,
    ) -> Result<SqlImmediateReceipt, ContextConfigurationFailure> {
        let identity = intent
            .identity()
            .map_err(ContextConfigurationFailure::Storage)?;
        let mut domain = None;
        let result = self.execute_sql(
            intent.operation_id,
            |tx, fence| {
                let operation = intent.operation_id;
                if intent.expected_epoch != fence.epoch {
                    return Err(SqlImmediateFailure::Conflict(Box::new(diagnostic(
                        operation,
                        fence,
                        "context-configuration-seed",
                        "data_epoch_changed",
                    ))));
                }
                receipts::check_state(tx, operation, fence, "context-configuration-seed")?;
                receipts::check_cross_kind(tx, operation, fence, "context-configuration-seed")?;
                configuration::check(tx, intent.conversation_id, true).map_err(|error| {
                    if error == Error::Unavailable {
                        SqlImmediateFailure::Unavailable(Box::new(diagnostic(
                            operation,
                            fence,
                            "context-configuration-seed-validate",
                            "configuration_validation_unavailable",
                        )))
                    } else {
                        domain = Some(error);
                        SqlImmediateFailure::Domain(CatalogError::InvalidInput)
                    }
                })?;
                if let Some(receipt) =
                    receipts::lookup(tx, &identity, fence, "context-configuration-seed")?
                {
                    return Ok(receipt);
                }
                seed(tx).map_err(|error| {
                    if let crate::ai::error::AiError::Database(source) = &error {
                        receipts::storage(operation, fence, "context-configuration-seed", source)
                    } else {
                        SqlImmediateFailure::Unavailable(Box::new(diagnostic(
                            operation,
                            fence,
                            "context-configuration-seed",
                            error.code(),
                        )))
                    }
                })?;
                receipts::insert(tx, &identity, fence)
            },
            #[cfg(test)]
            fault,
        );
        match result {
            Err(SqlImmediateFailure::Domain(CatalogError::InvalidInput)) if domain.is_some() => {
                Err(ContextConfigurationFailure::Domain(
                    domain.expect("domain signal contains error"),
                ))
            }
            Err(error) => Err(ContextConfigurationFailure::Storage(error)),
            Ok(receipt) => Ok(receipt),
        }
    }

    pub(crate) fn context_configuration_seed_status(
        &self,
        intent: &ContextConfigurationSeedIntent,
    ) -> Result<SqlImmediateStatus, SqlImmediateFailure> {
        let status = self.sql_operation_status(&intent.identity()?)?;
        let epoch = match &status {
            SqlImmediateStatus::Committed(receipt) => receipt.epoch,
            SqlImmediateStatus::NoSqlReceipt { epoch, .. }
            | SqlImmediateStatus::Unconfirmed { epoch, .. } => *epoch,
        };
        if epoch != intent.expected_epoch {
            return Err(SqlImmediateFailure::Conflict(Box::new(
                SqlImmediateDiagnostic::for_operation(
                    intent.operation_id,
                    None,
                    "context-configuration-seed-status",
                    "data_epoch_changed",
                ),
            )));
        }
        Ok(status)
    }

    /// A scoped read for freezing a new request, never proof of any write.
    pub(crate) fn context_configuration_environment(
        &self,
        request: &ContextConfigurationEnvironmentRequest,
    ) -> Result<(ReadFence, Option<configuration::Configuration>), ContextConfigurationFailure>
    {
        let operation = request.operation_id;
        if operation.is_nil() || request.conversation_id.is_nil() {
            return Err(ContextConfigurationFailure::Storage(
                SqlImmediateFailure::Invalid(Box::new(SqlImmediateDiagnostic::for_operation(
                    operation,
                    None,
                    "context-configuration-environment",
                    "invalid_configuration_scope",
                ))),
            ));
        }
        let owner = self
            .owner
            .as_ref()
            .ok_or_else(|| ContextConfigurationFailure::Storage(unmanaged(operation)))?;
        let mut admission = owner.materialize_guard().map_err(|error| {
            ContextConfigurationFailure::Storage(admission_error(operation, error, None))
        })?;
        let fence = admission.fence();
        let db = self.raw.lock().map_err(|_| {
            admission.block();
            ContextConfigurationFailure::Storage(SqlImmediateFailure::RecoveryRequired(Box::new(
                diagnostic(
                    operation,
                    fence,
                    "context-configuration-environment",
                    "sql_mutex_poisoned",
                ),
            )))
        })?;
        if !db.is_autocommit() {
            admission.block();
            return Err(ContextConfigurationFailure::Storage(
                SqlImmediateFailure::RecoveryRequired(Box::new(diagnostic(
                    operation,
                    fence,
                    "context-configuration-environment",
                    "sql_transaction_open",
                ))),
            ));
        }
        let tx = match db.unchecked_transaction() {
            Ok(tx) => tx,
            Err(error) => {
                if !db.is_autocommit() {
                    admission.block();
                    return Err(ContextConfigurationFailure::Storage(
                        SqlImmediateFailure::RecoveryRequired(Box::new(
                            diagnostic(
                                operation,
                                fence,
                                "context-configuration-environment",
                                "read_transaction_begin_unconfirmed",
                            )
                            .sqlite(&error),
                        )),
                    ));
                }
                return Err(ContextConfigurationFailure::Storage(receipts::storage(
                    operation,
                    fence,
                    "context-configuration-environment",
                    &error,
                )));
            }
        };
        let result = (|| {
            receipts::check_state(&tx, operation, fence, "context-configuration-environment")
                .map_err(ContextConfigurationFailure::Storage)?;
            configuration::read_in(&tx, request.conversation_id).map_err(|error| {
                if error == Error::Unavailable {
                    ContextConfigurationFailure::Storage(SqlImmediateFailure::Unavailable(
                        Box::new(diagnostic(
                            operation,
                            fence,
                            "context-configuration-environment",
                            "configuration_read_unavailable",
                        )),
                    ))
                } else {
                    ContextConfigurationFailure::Domain(error)
                }
            })
        })();
        let rollback = tx.rollback();
        if rollback.is_err() || !db.is_autocommit() {
            admission.block();
            let detail = diagnostic(
                operation,
                fence,
                "context-configuration-environment",
                "read_transaction_cleanup_failed",
            );
            let detail = match rollback.as_ref().err() {
                Some(error) => detail.sqlite(error),
                None => detail,
            };
            return Err(ContextConfigurationFailure::Storage(
                SqlImmediateFailure::RecoveryRequired(Box::new(detail)),
            ));
        }
        if matches!(
            result,
            Err(ContextConfigurationFailure::Storage(
                SqlImmediateFailure::Corrupt(_)
            ))
        ) {
            admission.block();
        }
        result.map(|configuration| (fence, configuration))
    }

    pub(crate) fn save_context_configuration(
        &self,
        intent: &ContextConfigurationIntent,
        writing_source: Option<crate::agent::writing_resources::WritingResourceSource>,
    ) -> Result<SqlImmediateReceipt, ContextConfigurationFailure> {
        self.save_context_configuration_owned(
            intent,
            #[cfg(test)]
            super::Fault::None,
            writing_source,
        )
    }

    fn save_context_configuration_owned(
        &self,
        intent: &ContextConfigurationIntent,
        #[cfg(test)] fault: super::Fault,
        writing_source: Option<crate::agent::writing_resources::WritingResourceSource>,
    ) -> Result<SqlImmediateReceipt, ContextConfigurationFailure> {
        self.save_context_configuration_owned_with_scope(
            intent,
            #[cfg(test)]
            fault,
            writing_source.map(crate::agent::writing_resources::WritingScope::Static),
        )
    }

    pub(crate) fn save_context_configuration_with_scope(
        &self,
        intent: &ContextConfigurationIntent,
        writing_source: Option<crate::agent::writing_resources::WritingScope<'_>>,
    ) -> Result<SqlImmediateReceipt, ContextConfigurationFailure> {
        self.save_context_configuration_owned_with_scope(
            intent,
            #[cfg(test)]
            super::Fault::None,
            writing_source,
        )
    }

    fn save_context_configuration_owned_with_scope(
        &self,
        intent: &ContextConfigurationIntent,
        #[cfg(test)] fault: super::Fault,
        writing_source: Option<crate::agent::writing_resources::WritingScope<'_>>,
    ) -> Result<SqlImmediateReceipt, ContextConfigurationFailure> {
        let identity = intent
            .identity()
            .map_err(ContextConfigurationFailure::Storage)?;
        let mut domain = None;
        let result = self.execute_sql(
            intent.operation_id,
            |tx, fence| register(tx, fence, intent, &identity, &mut domain, writing_source),
            #[cfg(test)]
            fault,
        );
        match result {
            Err(SqlImmediateFailure::Domain(CatalogError::InvalidInput)) if domain.is_some() => {
                Err(ContextConfigurationFailure::Domain(
                    domain.expect("domain signal contains error"),
                ))
            }
            Err(error) => Err(ContextConfigurationFailure::Storage(error)),
            Ok(receipt) => Ok(receipt),
        }
    }

    pub(crate) fn context_configuration_status(
        &self,
        intent: &ContextConfigurationIntent,
    ) -> Result<SqlImmediateStatus, SqlImmediateFailure> {
        let status = self.sql_operation_status(&intent.identity()?)?;
        let epoch = match &status {
            SqlImmediateStatus::Committed(receipt) => receipt.epoch,
            SqlImmediateStatus::NoSqlReceipt { epoch, .. }
            | SqlImmediateStatus::Unconfirmed { epoch, .. } => *epoch,
        };
        if epoch != intent.expected_epoch {
            return Err(SqlImmediateFailure::Conflict(Box::new(
                SqlImmediateDiagnostic::for_operation(
                    intent.operation_id,
                    None,
                    "context-configuration-status",
                    "data_epoch_changed",
                ),
            )));
        }
        Ok(status)
    }
}

fn register(
    tx: &Transaction<'_>,
    fence: ReadFence,
    intent: &ContextConfigurationIntent,
    identity: &ReceiptIdentity,
    domain: &mut Option<Error>,
    writing_source: Option<crate::agent::writing_resources::WritingScope<'_>>,
) -> Result<SqlImmediateReceipt, SqlImmediateFailure> {
    let operation = intent.operation_id;
    if intent.expected_epoch != fence.epoch {
        return Err(SqlImmediateFailure::Conflict(Box::new(diagnostic(
            operation,
            fence,
            "context-configuration-save",
            "data_epoch_changed",
        ))));
    }
    receipts::check_state(tx, operation, fence, "context-configuration-save")?;
    receipts::check_cross_kind(tx, operation, fence, "context-configuration-save")?;
    let business = |error: Error, domain: &mut Option<Error>| {
        if error == Error::Unavailable {
            SqlImmediateFailure::Unavailable(Box::new(diagnostic(
                operation,
                fence,
                "context-configuration-validate",
                "configuration_validation_unavailable",
            )))
        } else {
            *domain = Some(error);
            SqlImmediateFailure::Domain(CatalogError::InvalidInput)
        }
    };
    configuration::check(tx, intent.conversation_id, true)
        .map_err(|error| business(error, domain))?;
    configuration::validate_with_scope(tx, &intent.state, writing_source)
        .map_err(|error| business(error, domain))?;
    if let Some(receipt) = receipts::lookup(tx, identity, fence, "context-configuration-save")? {
        return Ok(receipt);
    }
    let current = configuration::read_in(tx, intent.conversation_id)
        .map_err(|error| business(error, domain))?;
    if current.as_ref().map_or(0, |saved| saved.revision) != intent.expected_revision {
        return Err(business(Error::ConfirmationMismatch, domain));
    }
    tx.execute("insert into agent_context_configurations(conversation_id,revision,state_json,updated_at) values(?1,?2,?3,strftime('%Y-%m-%dT%H:%M:%fZ','now')) on conflict(conversation_id) do update set revision=excluded.revision,state_json=excluded.state_json,updated_at=excluded.updated_at", params![intent.conversation_id.to_string(), intent.expected_revision + 1, intent.state.to_string()])
        .map_err(|error| receipts::storage(operation, fence, "context-configuration-write", &error))?;
    receipts::insert(tx, identity, fence)
}

#[cfg(test)]
#[path = "context_configuration_tests.rs"]
mod tests;
