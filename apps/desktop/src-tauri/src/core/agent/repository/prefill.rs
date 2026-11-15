//! Native body-scope baseline owner. SQLite work stays on the storage executor;
//! window and rollback epoch fences prevent installing stale observations.
use crate::data_interface::{IntoDatabase, SharedDatabase};
use research_agent_core::{
    context::{
        accounting::*, checkpoint::ContextCheckpoint, compaction_strategy::CompactionOutput,
    },
    model::*,
    protocol::ModelIdentity,
};
#[cfg(test)]
use rusqlite::Connection;
use rusqlite::{DropBehavior, OptionalExtension, Transaction, TransactionBehavior, params};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

pub(crate) struct SqlitePrefillEstimator {
    connection: SharedDatabase,
    conversation: Uuid,
    model_json: String,
    inner: Arc<PrefillScopedEstimator>,
    epoch: Arc<Mutex<Option<u64>>>,
}

fn persistence_diagnostic(
    conversation_id: Uuid,
    phase: &'static str,
    safe_cause: &'static str,
    error: Option<&rusqlite::Error>,
) -> ModelError {
    let sqlite_extended_code = match error {
        Some(rusqlite::Error::SqliteFailure(code, _)) => Some(code.extended_code),
        _ => None,
    };
    ModelError::PersistenceDiagnostic(ContextPersistenceDiagnostic {
        conversation_id,
        phase,
        safe_cause,
        sqlite_extended_code,
    })
}
impl SqlitePrefillEstimator {
    pub(crate) fn new(
        connection: impl IntoDatabase,
        conversation: Uuid,
        model: &ModelIdentity,
        inner: Arc<dyn ContextTokenEstimator>,
    ) -> Result<Self, ModelError> {
        let model_json = serde_json::to_string(model).map_err(|_| ModelError::InvalidRequest)?;
        if conversation.is_nil()
            || model.provider_id.trim().is_empty()
            || model.model_id.trim().is_empty()
            || model_json.len() > 4096
        {
            return Err(ModelError::InvalidRequest);
        }
        Ok(Self {
            connection: connection.into_database(),
            conversation,
            model_json,
            inner: Arc::new(PrefillScopedEstimator::new(inner)),
            epoch: Arc::new(Mutex::new(None)),
        })
    }
    fn transaction(
        &self,
        window: Option<Uuid>,
        active: Option<u64>,
        prepare: bool,
    ) -> ModelFuture<'_, ()> {
        let connection = self.connection.clone();
        let conversation_id = self.conversation;
        let conversation = conversation_id.to_string();
        let model = self.model_json.clone();
        let inner = self.inner.clone();
        let epoch = self.epoch.clone();
        Box::pin(async move {
            tokio::task::spawn_blocking(move || {
                let mut db = connection.lock().map_err(|_| ModelError::Persistence)?;
                crate::data_interface::guarded_transaction::run(&mut db, |db| {
                    let mut tx = match Transaction::new_unchecked(db, TransactionBehavior::Immediate) {
                        Ok(tx) => tx,
                        Err(error) => return (Err(persistence_diagnostic(conversation_id, "begin", "sql_begin_failed", Some(&error))), !db.is_autocommit()),
                    };
                    tx.set_drop_behavior(DropBehavior::Ignore);
                    let operation = (|| -> Result<u64, ModelError> {
                        let archived: Option<bool> = tx.query_row("select archived from agent_conversations where id=?1", [&conversation], |row| row.get(0)).optional().map_err(|error| persistence_diagnostic(conversation_id, "read", "sql_read_failed", Some(&error)))?;
                        if archived != Some(false) { return Err(ModelError::InvalidRequest); }
                        let installed: Option<String> = tx.query_row("select window_id from agent_active_context_windows where conversation_id=?1", [&conversation], |row| row.get(0)).optional().map_err(|error| persistence_diagnostic(conversation_id, "read", "sql_read_failed", Some(&error)))?;
                        if installed != window.map(|id| id.to_string()) { return Err(ModelError::InvalidRequest); }
                        let revision: u64 = tx.query_row("select coalesce(max(ordinal)+1,0) from agent_items where conversation_id=?1 and json_extract(payload_json,'$.payload.type')='conversationRolledBack'", [&conversation], |row| row.get(0)).map_err(|error| persistence_diagnostic(conversation_id, "read", "sql_read_failed", Some(&error)))?;
                        let key = installed.unwrap_or_default();
                        if prepare {
                            let saved: Option<String> = tx.query_row("select baseline_json from agent_prefill_baselines where conversation_id=?1 and model_json=?2 and window_key=?3 and rollback_epoch=?4", params![conversation,model,key,revision], |row| row.get(0)).optional().map_err(|error| persistence_diagnostic(conversation_id, "read", "sql_read_failed", Some(&error)))?;
                            let saved: Option<PrefillBaseline> = saved.map(|value| serde_json::from_str(&value).map_err(|_| ModelError::InvalidStream)).transpose()?;
                            if saved.is_some_and(|value| value.window != window) { return Err(ModelError::InvalidStream); }
                            inner.restore_baseline(saved)?;
                            // The normalized window identity is passed through a small
                            // state operation, not a fabricated checkpoint payload.
                            if saved.is_none() && let Some(tokens) = active {
                                inner.restore_baseline(Some(PrefillBaseline { window, tokens, observed: false }))?;
                            }
                        } else if *epoch.lock().map_err(|_| ModelError::Persistence)? != Some(revision) {
                            return Err(ModelError::InvalidRequest);
                        }
                        if let Some(value) = inner.baseline()? {
                            if value.window != window { return Err(ModelError::InvalidRequest); }
                            let encoded = serde_json::to_string(&value).map_err(|_| ModelError::InvalidStream)?;
                            tx.execute("insert into agent_prefill_baselines(conversation_id,model_json,window_key,rollback_epoch,baseline_json) values(?1,?2,?3,?4,?5) on conflict(conversation_id,model_json,window_key,rollback_epoch) do update set baseline_json=excluded.baseline_json where json_extract(agent_prefill_baselines.baseline_json,'$.observed')=0 and agent_prefill_baselines.baseline_json <> excluded.baseline_json", params![conversation,model,key,revision,encoded]).map_err(|error| persistence_diagnostic(conversation_id, "write", "sql_write_failed", Some(&error)))?;
                        }
                        Ok(revision)
                    })();
                    match operation {
                        Err(error) => {
                            let rollback = tx.rollback();
                            let rollback_failed = rollback.is_err() || !db.is_autocommit();
                            let error = if rollback_failed {
                                persistence_diagnostic(conversation_id, "rollback", if rollback.is_err() { "sql_rollback_failed" } else { "sql_transaction_remains_open" }, rollback.as_ref().err())
                            } else { error };
                            (Err(error), rollback_failed)
                        }
                        Ok(revision) => {
                            if let Err(error) = tx.commit() {
                                if !db.is_autocommit() { let _ = db.execute_batch("rollback"); }
                                return (Err(persistence_diagnostic(conversation_id, "commit", "sql_commit_outcome_unknown", Some(&error))), true);
                            }
                            if !db.is_autocommit() { return (Err(persistence_diagnostic(conversation_id, "commit", "sql_transaction_remains_open", None)), true); }
                            if prepare {
                                match epoch.lock() {
                                    Ok(mut epoch) => *epoch = Some(revision),
                                    Err(_) => return (Err(ModelError::Persistence), false),
                                }
                            }
                            (Ok(()), false)
                        }
                    }
                })
            }).await.map_err(|_| ModelError::Persistence)?
        })
    }
}
impl ContextTokenEstimator for SqlitePrefillEstimator {
    fn prepare_auto_compaction<'a>(
        &'a self,
        active: Option<u64>,
        checkpoint: Option<&'a ContextCheckpoint>,
    ) -> ModelFuture<'a, ()> {
        self.transaction(checkpoint.map(|value| value.window_id), active, true)
    }
    fn persist_auto_compaction<'a>(
        &'a self,
        checkpoint: Option<&'a ContextCheckpoint>,
    ) -> ModelFuture<'a, ()> {
        self.transaction(checkpoint.map(|value| value.window_id), None, false)
    }
    fn auto_compaction_scope(&self) -> AutoCompactionScope {
        AutoCompactionScope::BodyAfterPrefix
    }
    fn auto_compaction_tokens(
        &self,
        active: Option<u64>,
        checkpoint: Option<&ContextCheckpoint>,
    ) -> Result<Option<u64>, ModelError> {
        self.inner.auto_compaction_tokens(active, checkpoint)
    }
    fn estimate(&self, messages: &[ModelMessage], tools: &[ToolDefinition]) -> Option<u64> {
        self.inner.estimate(messages, tools)
    }
    fn estimate_window(
        &self,
        messages: &[ModelMessage],
        tools: &[ToolDefinition],
        checkpoint: Option<&ContextCheckpoint>,
    ) -> Result<Option<u64>, ModelError> {
        self.inner.estimate_window(messages, tools, checkpoint)
    }
    fn estimate_compaction(&self, output: &CompactionOutput) -> Option<u64> {
        self.inner.estimate_compaction(output)
    }
    fn text_byte_budget(&self, tokens: u64) -> Option<usize> {
        self.inner.text_byte_budget(tokens)
    }
    fn observe(
        &self,
        messages: &[ModelMessage],
        tools: &[ToolDefinition],
        usage: ModelUsage,
        checkpoint: Option<&ContextCheckpoint>,
    ) -> Result<(), ModelError> {
        self.inner.observe(messages, tools, usage, checkpoint)
    }
}

#[cfg(test)]
#[path = "prefill/tests.rs"]
mod tests;
