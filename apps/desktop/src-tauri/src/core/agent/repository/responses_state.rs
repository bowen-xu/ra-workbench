//! SQLite-backed Responses state. Transactions run on the storage executor;
//! canceled awaits may leave a committed checkpoint, never a restarted request.
use crate::agent::model::{
    responses_continuation::ResponsesContinuation,
    responses_transport::{ResponsesReceipt, ResponsesReceiptSink},
};
use crate::data_interface::{IntoDatabase, SharedDatabase};
use research_agent_core::{
    model::{ModelError, ModelFuture},
    protocol::ModelIdentity,
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde_json::Value;
#[cfg(test)]
use std::sync::{Arc, Mutex};
use uuid::Uuid;

#[derive(Clone)]
pub struct ResponsesStateScope {
    pub conversation_id: Uuid,
    pub turn_id: Uuid,
    pub context_snapshot_id: Uuid,
    pub model: ModelIdentity,
}
pub struct SqliteResponsesState {
    connection: SharedDatabase,
    scope: ResponsesStateScope,
    max_bytes: usize,
    native_read_policy: Option<crate::agent::execution_profile::NativeCompactionPolicy>,
}
impl SqliteResponsesState {
    pub(crate) fn new(
        connection: impl IntoDatabase,
        scope: ResponsesStateScope,
        max_bytes: usize,
    ) -> Result<Self, ModelError> {
        if scope.conversation_id.is_nil()
            || scope.turn_id.is_nil()
            || scope.context_snapshot_id.is_nil()
            || max_bytes == 0
            || max_bytes > 33_554_432
        {
            return Err(ModelError::InvalidRequest);
        }
        ResponsesContinuation::new(scope.turn_id, scope.model.clone(), max_bytes)?;
        Ok(Self {
            connection: connection.into_database(),
            scope,
            max_bytes,
            native_read_policy: None,
        })
    }
    pub(crate) fn with_native_read_policy(
        mut self,
        policy: crate::agent::execution_profile::NativeCompactionPolicy,
    ) -> Self {
        self.native_read_policy = Some(policy);
        self
    }
    #[cfg(test)]
    async fn transaction<T: Send + 'static>(
        &self,
        operation: impl FnOnce(&ResponsesContinuation, &Connection) -> Result<T, ModelError>
        + Send
        + 'static,
    ) -> Result<T, ModelError> {
        let connection = self.connection.clone();
        let scope = self.scope.clone();
        let max_bytes = self.max_bytes;
        tokio::task::spawn_blocking(move || {
            let mut connection=connection.lock().map_err(|_|ModelError::Unavailable)?;
            let tx=connection.transaction_with_behavior(TransactionBehavior::Immediate).map_err(|_|ModelError::Unavailable)?;
            admission::validate_scope(&tx,&scope)?;
            let active: Option<String> = tx.query_row("select window_id from agent_active_context_windows where conversation_id=?1", [scope.conversation_id.to_string()], |row| row.get(0)).optional().map_err(|_| ModelError::Unavailable)?;
            let saved:Option<(u32,String,Option<String>)>=tx.query_row("select p.schema_version,p.checkpoint_json,b.window_id from agent_responses_checkpoints p left join agent_responses_window_bindings b on b.turn_id=p.turn_id where p.turn_id=?1",[scope.turn_id.to_string()],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).optional().map_err(|_|ModelError::Unavailable)?;
            let state=match saved {
                Some((_, _, window)) if window != active => ResponsesContinuation::new(scope.turn_id, scope.model.clone(), max_bytes)?,
                Some((1..=3,encoded,_))=>ResponsesContinuation::restore(scope.turn_id,scope.model.clone(),max_bytes,&encoded)?,
                Some(_)=>return Err(ModelError::InvalidStream),
                None=>history::initial_state(&tx, &scope, max_bytes, active.as_deref())?,
            };
            let result=operation(&state, &tx)?;
            let encoded=state.encode_checkpoint()?;
            let schema = serde_json::from_str::<Value>(&encoded).map_err(|_| ModelError::InvalidStream)?["schema_version"].as_u64().ok_or(ModelError::InvalidStream)?;
            tx.execute("insert into agent_responses_checkpoints(turn_id,schema_version,checkpoint_json) values (?1,?2,?3) on conflict(turn_id) do update set schema_version=excluded.schema_version,checkpoint_json=excluded.checkpoint_json,updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now')",
                params![scope.turn_id.to_string(),schema,encoded]).map_err(|_|ModelError::Unavailable)?;
            if let Some(window) = active {
                tx.execute("insert into agent_responses_window_bindings(turn_id,window_id) values (?1,?2) on conflict(turn_id) do update set window_id=excluded.window_id", params![scope.turn_id.to_string(),window]).map_err(|_|ModelError::Unavailable)?;
            } else {
                tx.execute("delete from agent_responses_window_bindings where turn_id=?1", [scope.turn_id.to_string()]).map_err(|_|ModelError::Unavailable)?;
            }
            tx.commit().map_err(|_|ModelError::Unavailable)?;Ok(result)
        }).await.map_err(|_|ModelError::Unavailable)?
    }
}
impl ResponsesReceiptSink for SqliteResponsesState {
    fn prepare_canonical_input(
        &self,
        account: Uuid,
        model: ModelIdentity,
        input: Vec<Value>,
        captured: crate::agent::model::responses_metadata::CapturedResponsesInput,
    ) -> ModelFuture<'_, crate::agent::model::responses_metadata::PreparedResponsesInput> {
        canonical::prepare(self, account, model, input, captured)
    }
    fn prepare_input(
        &self,
        _account: Uuid,
        _model: ModelIdentity,
        _input: Vec<Value>,
    ) -> ModelFuture<'_, Vec<Value>> {
        Box::pin(async { Err(ModelError::UnsupportedCapability) })
    }
    fn preserve(&self, _receipt: ResponsesReceipt) -> ModelFuture<'_, ()> {
        Box::pin(async { Err(ModelError::UnsupportedCapability) })
    }
}

// Only legacy compatibility fixtures can create schema 1–3 accumulated state.
// Production receives native items through ItemAccepted and reads old rows once.
#[cfg(test)]
impl SqliteResponsesState {
    fn fixture_prepare_with_authority(
        &self,
        account: Uuid,
        model: ModelIdentity,
        input: Vec<Value>,
        kind: crate::agent::model::responses_metadata::ResponsesInputKind,
    ) -> ModelFuture<'_, Vec<Value>> {
        Box::pin(async move {
            Ok(self
                .fixture_prepare_with_metadata(account, model, input, kind)
                .await?
                .input)
        })
    }
    fn fixture_prepare_with_metadata(
        &self,
        account: Uuid,
        model: ModelIdentity,
        input: Vec<Value>,
        kind: crate::agent::model::responses_metadata::ResponsesInputKind,
    ) -> ModelFuture<'_, crate::agent::model::responses_metadata::PreparedResponsesInput> {
        use crate::agent::model::responses_metadata::{
            PreparedResponsesInput, ResponsesRequestMetadata,
        };
        Box::pin(async move {
            let scope = self.scope.clone();
            let policy = self.native_read_policy;
            self.transaction(move |state, db| {
                compaction::bind_installed(state, db, &scope, account, policy)?;
                let identity = super::context_windows::identity::read_or_create_owned(
                    db,
                    scope.conversation_id,
                    Uuid::now_v7(),
                )
                .map_err(|error| match error {
                    super::context_windows::Error::Storage => ModelError::Unavailable,
                    super::context_windows::Error::Canceled => ModelError::Canceled,
                    super::context_windows::Error::Inactive
                    | super::context_windows::Error::Conflict => ModelError::InvalidRequest,
                    super::context_windows::Error::Invalid => ModelError::InvalidStream,
                })?;
                let input = state.prepare_scoped_sync(
                    account,
                    model,
                    input,
                    kind.authority_items(),
                    kind.text_compaction(),
                )?;
                Ok(PreparedResponsesInput {
                    input,
                    metadata: Some(ResponsesRequestMetadata::new(
                        scope.conversation_id,
                        scope.turn_id,
                        identity.number,
                    )?),
                })
            })
            .await
        })
    }
    fn fixture_prepare_input(
        &self,
        credential_scope_id: Uuid,
        model: ModelIdentity,
        input: Vec<Value>,
    ) -> ModelFuture<'_, Vec<Value>> {
        Box::pin(async move {
            let scope = self.scope.clone();
            let policy = self.native_read_policy;
            self.transaction(move |state, db| {
                compaction::bind_installed(state, db, &scope, credential_scope_id, policy)?;
                state.prepare_sync(credential_scope_id, model, input)
            })
            .await
        })
    }
    fn fixture_preserve(&self, receipt: ResponsesReceipt) -> ModelFuture<'_, ()> {
        let scope = self.scope.clone();
        let policy = self.native_read_policy;
        Box::pin(async move {
            self.transaction(move |state, connection| {
                if receipt.request_id.is_nil() {
                    return Err(ModelError::InvalidStream);
                }
                compaction::bind_installed(
                    state,
                    connection,
                    &scope,
                    receipt.credential_scope_id,
                    policy,
                )?;
                // Exact retries of an already committed checkpoint remain safe
                // after the turn ends; preserve_sync verifies every saved value.
                if state.saved_request_id()? != Some(receipt.request_id) {
                    admission::require_running_attempt(connection, &scope, &receipt)?;
                }
                state.preserve_sync(receipt)
            })
            .await
        })
    }
}

#[path = "responses_state/admission.rs"]
mod admission;
#[path = "responses_state/canonical.rs"]
mod canonical;
#[path = "responses_state/compaction.rs"]
mod compaction;
#[cfg(test)]
#[path = "responses_state/history.rs"]
mod history;
#[cfg(test)]
#[path = "responses_state/tests.rs"]
mod tests;
