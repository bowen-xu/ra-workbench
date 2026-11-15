//! A host must still own a live resource when admitting a history rewrite.
use super::*;
use crate::data_interface::{
    AuthorizeRollback, SqlAuthorizationError, SqlImmediateDiagnostic, SqlImmediateFailure,
    SqlImmediateStatus,
};
use rusqlite::Transaction;

#[derive(Debug)]
pub(crate) enum AuthorizationError {
    Domain(CatalogError),
    Data(SqlImmediateFailure),
}
impl From<SqlImmediateFailure> for AuthorizationError {
    fn from(error: SqlImmediateFailure) -> Self {
        match error {
            SqlImmediateFailure::Domain(error) => Self::Domain(error),
            other => Self::Data(other),
        }
    }
}

impl SqliteConversationCatalog {
    /// Read a registration proof without reauthorizing the host or materializing
    /// pending journal data. The same fingerprint proves the original binding.
    pub(crate) async fn read_rollback_authorization_status(
        &self,
        command: AuthorizeRollback,
    ) -> Result<SqlImmediateStatus, AuthorizationError> {
        let operation = command.operation_id;
        let connection = self.connection.clone();
        self.worker_retention
            .spawn(move || connection.rollback_authorization_status(&command))
            .await
            .map_err(|_| {
                AuthorizationError::Data(SqlImmediateFailure::Unavailable(Box::new(
                    SqlImmediateDiagnostic {
                        operation_id: operation,
                        epoch: None,
                        sql_fence: None,
                        phase: "authorization-status-worker",
                        sqlite_code: None,
                        sqlite_extended_code: None,
                        safe_cause: "authorization_status_worker_failed",
                    },
                )))
            })?
            .map_err(AuthorizationError::from)
    }

    pub(crate) async fn authorize_rollback(
        &self,
        id: ConversationId,
        request: Uuid,
        binding: ConversationBinding,
    ) -> Result<(), AuthorizationError> {
        if id.as_uuid().is_nil() || request.is_nil() {
            return Err(AuthorizationError::Domain(CatalogError::InvalidInput));
        }
        validate_binding(&binding).map_err(AuthorizationError::Domain)?;
        let command = AuthorizeRollback {
            operation_id: request,
            conversation_id: id,
            binding,
        };
        let connection = self.connection.clone();
        // The blocking worker retains the real data owner through its transaction
        // and receipt. A lost worker result cannot prove that COMMIT did not run.
        self.worker_retention
            .spawn(move || connection.authorize_rollback(&command))
            .await
            .map_err(|_| {
                AuthorizationError::Data(SqlImmediateFailure::Unknown(Box::new(
                    SqlImmediateDiagnostic {
                        operation_id: request,
                        epoch: None,
                        sql_fence: None,
                        phase: "authorization-worker",
                        sqlite_code: None,
                        sqlite_extended_code: None,
                        safe_cause: "authorization_worker_result_unknown",
                    },
                )))
            })?
            .map(|_| ())
            .map_err(AuthorizationError::from)
    }

    /// Explicit pure-SQL fixture path. It produces no managed storage receipt.
    #[cfg(test)]
    pub(crate) async fn authorize_rollback_legacy_for_tests(
        &self,
        id: ConversationId,
        request: Uuid,
        binding: ConversationBinding,
    ) -> Result<(), CatalogError> {
        if self.connection.logged() {
            return Err(CatalogError::InvalidInput);
        }
        let command = AuthorizeRollback {
            operation_id: request,
            conversation_id: id,
            binding,
        };
        self.run(move |db| {
            let tx = db
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(db_error)?;
            authorize_in(&tx, &command).map_err(|error| match error {
                SqlAuthorizationError::Domain(error) => error,
                SqlAuthorizationError::Sql(error) => db_error(error),
            })?;
            tx.commit().map_err(db_error)
        })
        .await
    }
}

/// Runs inside the facade-owned transaction. Replays revalidate the live host
/// before consulting either the authority identity or the facade receipt.
pub(crate) fn authorize_in(
    tx: &Transaction<'_>,
    command: &AuthorizeRollback,
) -> Result<(), SqlAuthorizationError> {
    let AuthorizeRollback {
        conversation_id: id,
        operation_id: request,
        binding,
    } = command;
    if id.as_uuid().is_nil() || request.is_nil() {
        return Err(CatalogError::InvalidInput.into());
    }
    validate_binding(binding)?;
    let bound: bool = tx.query_row(
                "select exists(select 1 from agent_conversations c join agent_conversation_bindings b on b.conversation_id=c.id
                 where c.id=?1 and c.archived=0 and b.resource_kind=?2 and b.resource_id=?3)",
                params![id.as_uuid().to_string(), binding_kind(binding.kind), binding.resource_id],
                |row| row.get(0),
            )?;
    let available: bool = tx.query_row(
        binding_target_query(binding.kind),
        [&binding.resource_id],
        |row| row.get(0),
    )?;
    if !bound || !available {
        return Err(CatalogError::NotFound.into());
    }
    let prior: Option<(String, String, String)> = tx.query_row(
                "select conversation_id,resource_kind,resource_id from agent_rollback_authority where request_id=?1",
                [request.to_string()], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)),
            ).optional()?;
    let identity = (
        id.as_uuid().to_string(),
        binding_kind(binding.kind).to_string(),
        binding.resource_id.clone(),
    );
    if prior.as_ref().is_some_and(|prior| prior != &identity) {
        return Err(CatalogError::Conflict.into());
    }
    if prior.is_none() {
        tx.execute("insert into agent_rollback_authority(request_id,conversation_id,resource_kind,resource_id) values (?1,?2,?3,?4)",
            params![request.to_string(), identity.0, identity.1, identity.2])?;
    }
    Ok(())
}

/// Revalidate a renderer request inside the actual rollback transaction. Trusted
/// backend operations without a host admission record keep their existing API.
pub(crate) fn recheck_in(db: &Connection, id: Uuid, request: Uuid) -> Result<(), CatalogError> {
    let exists: bool = db.query_row("select exists(select 1 from sqlite_master where type='table' and name='agent_rollback_authority')", [], |row|row.get(0)).map_err(db_error)?;
    if !exists {
        return Ok(());
    }
    let saved: Option<(String, String, String)> = db.query_row(
        "select conversation_id,resource_kind,resource_id from agent_rollback_authority where request_id=?1",
        [request.to_string()], |row|Ok((row.get(0)?,row.get(1)?,row.get(2)?)),
    ).optional().map_err(db_error)?;
    let Some((scope, kind, resource)) = saved else {
        return Ok(());
    };
    if scope != id.to_string() {
        return Err(CatalogError::Conflict);
    }
    let kind = match kind.as_str() {
        "manuscript" => BindingKind::Manuscript,
        "paper" => BindingKind::Paper,
        "note" => BindingKind::Note,
        "readingNote" => BindingKind::ReadingNote,
        _ => return Err(CatalogError::Corrupt),
    };
    let bound: bool = db.query_row("select exists(select 1 from agent_conversation_bindings where conversation_id=?1 and resource_kind=?2 and resource_id=?3)",
        params![scope,binding_kind(kind),resource], |row|row.get(0)).map_err(db_error)?;
    let live: bool = db
        .query_row(binding_target_query(kind), [&resource], |row| row.get(0))
        .map_err(db_error)?;
    if !bound || !live {
        return Err(CatalogError::NotFound);
    }
    Ok(())
}
