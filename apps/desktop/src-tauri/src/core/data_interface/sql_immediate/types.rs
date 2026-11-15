//! Typed rollback registration, separate from the journal rollback result.
use research_agent_core::conversation::{CatalogError, ConversationBinding, ConversationId};
use research_data_runtime::ReadFence;
use serde::{Serialize, ser::SerializeStruct};
use sha2::{Digest, Sha256};
use uuid::Uuid;

pub(super) const KIND: &str = "agent.rollback_authority";
pub(super) const VERSION: u32 = 1;

/// Internal receipt identity. Public callers use one of the typed operations.
pub(super) struct ReceiptIdentity {
    pub operation_id: Uuid,
    pub kind: &'static str,
    pub version: u32,
    pub fingerprint: [u8; 32],
}

#[derive(Clone, Debug)]
pub(crate) struct AuthorizeRollback {
    pub operation_id: Uuid,
    pub conversation_id: ConversationId,
    pub binding: ConversationBinding,
}
impl AuthorizeRollback {
    pub(super) fn fingerprint(&self) -> Result<[u8; 32], SqlImmediateFailure> {
        if self.operation_id.is_nil()
            || self.conversation_id.as_uuid().is_nil()
            || self.binding.resource_id.trim().is_empty()
            || self.binding.resource_id.chars().count() > 256
            || self.binding.resource_id.contains('\0')
        {
            return Err(SqlImmediateFailure::Invalid(Box::new(
                SqlImmediateDiagnostic::new(
                    self,
                    None,
                    "validate",
                    "invalid_authorization_identity",
                ),
            )));
        }
        // This versioned struct has a fixed field order and contains no maps.
        // It is independent of the deployed journal command/hash encoding.
        #[derive(Serialize)]
        struct Frozen<'a> {
            kind: &'static str,
            version: u32,
            operation_id: Uuid,
            conversation_id: Uuid,
            binding: &'a ConversationBinding,
        }
        let bytes = serde_json::to_vec(&Frozen {
            kind: KIND,
            version: VERSION,
            operation_id: self.operation_id,
            conversation_id: *self.conversation_id.as_uuid(),
            binding: &self.binding,
        })
        .map_err(|_| {
            SqlImmediateFailure::Invalid(Box::new(SqlImmediateDiagnostic::new(
                self,
                None,
                "validate",
                "authorization_encoding_failed",
            )))
        })?;
        Ok(Sha256::digest(bytes).into())
    }
}

/// Proof of the registration transaction, never proof of rollback execution or
/// current permission to rewrite a conversation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SqlImmediateReceipt {
    pub operation_id: Uuid,
    pub epoch: Uuid,
    pub sql_fence: ReadFence,
    pub fingerprint: [u8; 32],
    pub result: (),
    pub replayed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SqlImmediateStatus {
    Committed(SqlImmediateReceipt),
    /// No receipt for this SQL registration. This says nothing about journal
    /// operations, older unmanaged registrations or whether anything executed.
    NoSqlReceipt {
        epoch: Uuid,
        sql_fence: ReadFence,
        closed: bool,
    },
    /// A blocked owner or an open transaction cannot prove non-commit from a
    /// missing receipt. Status never repairs, rolls back, or clears that state.
    Unconfirmed {
        epoch: Uuid,
        sql_fence: ReadFence,
        blocked: bool,
        closed: bool,
        reason: &'static str,
    },
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum SqlMutationError {
    #[error(transparent)]
    Domain(#[from] CatalogError),
    #[error(transparent)]
    Sql(#[from] rusqlite::Error),
}
pub(crate) use SqlMutationError as SqlAuthorizationError;

/// Contains only our classifications and SQLite's numeric/enum error codes.
/// SQLite messages, SQL text and bound values must never enter this DTO.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SqlImmediateDiagnostic {
    pub operation_id: Uuid,
    pub epoch: Option<Uuid>,
    /// Decimal text avoids rounding an internal u64 in JavaScript diagnostics.
    pub sql_fence: Option<String>,
    pub phase: &'static str,
    pub sqlite_code: Option<String>,
    pub sqlite_extended_code: Option<i32>,
    pub safe_cause: &'static str,
}
impl SqlImmediateDiagnostic {
    pub(super) fn new(
        command: &AuthorizeRollback,
        fence: Option<ReadFence>,
        phase: &'static str,
        safe_cause: &'static str,
    ) -> Self {
        Self::for_operation(command.operation_id, fence, phase, safe_cause)
    }
    pub(crate) fn for_operation(
        operation_id: Uuid,
        fence: Option<ReadFence>,
        phase: &'static str,
        safe_cause: &'static str,
    ) -> Self {
        Self {
            operation_id,
            epoch: fence.map(|fence| fence.epoch),
            sql_fence: fence.map(|fence| fence.position.to_string()),
            phase,
            sqlite_code: None,
            sqlite_extended_code: None,
            safe_cause,
        }
    }
    pub(super) fn sqlite(mut self, error: &rusqlite::Error) -> Self {
        if let rusqlite::Error::SqliteFailure(code, _) = error {
            self.sqlite_code = Some(format!("{:?}", code.code));
            self.sqlite_extended_code = Some(code.extended_code);
        }
        self
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum SqlImmediateFailure {
    #[error(transparent)]
    Domain(CatalogError),
    #[error("DATA_SQL_INVALID: {0:?}")]
    Invalid(Box<SqlImmediateDiagnostic>),
    #[error("DATA_SQL_UNMANAGED: {0:?}")]
    Unmanaged(Box<SqlImmediateDiagnostic>),
    #[error("DATA_SQL_OPERATION_CONFLICT: {0:?}")]
    Conflict(Box<SqlImmediateDiagnostic>),
    #[error("DATA_SQL_COMMIT_UNKNOWN: {0:?}")]
    Unknown(Box<SqlImmediateDiagnostic>),
    #[error("DATA_SQL_RECOVERY_REQUIRED: {0:?}")]
    RecoveryRequired(Box<SqlImmediateDiagnostic>),
    #[error("DATA_SQL_UNAVAILABLE: {0:?}")]
    Unavailable(Box<SqlImmediateDiagnostic>),
    #[error("DATA_SQL_CORRUPT: {0:?}")]
    Corrupt(Box<SqlImmediateDiagnostic>),
}
impl SqlImmediateFailure {
    pub(crate) fn code(&self) -> &'static str {
        match self {
            Self::Domain(_) => "DATA_SQL_DOMAIN",
            Self::Invalid(_) => "DATA_SQL_INVALID",
            Self::Unmanaged(_) => "DATA_SQL_UNMANAGED",
            Self::Conflict(_) => "DATA_SQL_OPERATION_CONFLICT",
            Self::Unknown(_) => "DATA_SQL_COMMIT_UNKNOWN",
            Self::RecoveryRequired(_) => "DATA_SQL_RECOVERY_REQUIRED",
            Self::Unavailable(_) => "DATA_SQL_UNAVAILABLE",
            Self::Corrupt(_) => "DATA_SQL_CORRUPT",
        }
    }
    pub(crate) fn diagnostic(&self) -> Option<&SqlImmediateDiagnostic> {
        match self {
            Self::Domain(_) => None,
            Self::Invalid(value)
            | Self::Unmanaged(value)
            | Self::Conflict(value)
            | Self::Unknown(value)
            | Self::RecoveryRequired(value)
            | Self::Unavailable(value)
            | Self::Corrupt(value) => Some(value),
        }
    }
}
impl Serialize for SqlImmediateFailure {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut state = serializer.serialize_struct("SqlImmediateFailure", 2)?;
        state.serialize_field("code", self.code())?;
        match self {
            Self::Domain(error) => state.serialize_field("reason", &error.to_string())?,
            _ => state.serialize_field("reason", &self.diagnostic())?,
        }
        state.end()
    }
}
