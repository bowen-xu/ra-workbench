//! Frozen non-secret model policy. Never persist authentication material or URLs.
use crate::data_interface::{IntoDatabase, SharedDatabase};
use research_agent_core::{
    context::{
        accounting::AutoCompactionScope,
        compaction_strategy::{CompactionBinding, CompactionBudget},
    },
    model::{ModelError, ModelFuture},
    submission::{SubmissionInput, SubmissionRecord},
};
use rusqlite::{OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ContextAuthentication {
    ApiKey,
    CodexBackend,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FrozenContextPolicy {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_summary:
        Option<super::super::model::responses_reasoning::ResponsesReasoningSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compact_prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_developer_model: Option<research_agent_core::protocol::ModelIdentity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_budget_settings:
        Option<super::super::execution_profile::token_budget::ResolvedTokenBudgetSettings>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_truncation_policy: Option<crate::ai::codex_model_catalog::TruncationPolicy>,
    pub schema_version: u32,
    pub binding: CompactionBinding,
    pub budget: Option<CompactionBudget>,
    pub max_output_tokens: u32,
    pub scope: AutoCompactionScope,
    pub unknown_window_trigger_tokens: Option<u64>,
    pub comp_hash: Option<String>,
    pub authentication: ContextAuthentication,
}
impl FrozenContextPolicy {
    /// Legacy reset snapshots use reviewed defaults; absence on any other
    /// strategy remains disabled. Never infer enablement from a model name.
    pub fn token_budget_policy(
        &self,
    ) -> Result<Option<research_agent_core::context::token_budget::TokenBudgetPolicy>, ModelError>
    {
        use research_agent_core::context::{
            compaction_strategy::CompactionKind, token_budget::TokenBudgetPolicy,
        };
        if self.binding.kind != CompactionKind::ResetWindow {
            return if self.token_budget_settings.is_some() {
                Err(ModelError::InvalidRequest)
            } else {
                Ok(None)
            };
        }
        match &self.token_budget_settings {
            Some(settings) => settings.policy().map(Some),
            None => TokenBudgetPolicy::resolve(Default::default()).map(Some),
        }
    }

    pub fn validate(&self) -> Result<(), ModelError> {
        if self.tool_truncation_policy.is_some()
            && self.authentication != ContextAuthentication::CodexBackend
        {
            return Err(ModelError::InvalidRequest);
        }
        if self.reasoning_summary.is_some()
            && self.authentication != ContextAuthentication::CodexBackend
        {
            return Err(ModelError::InvalidRequest);
        }
        self.token_budget_policy()?;
        if self.chat_developer_model.as_ref().is_some_and(|model| {
            model != &self.binding.model || self.authentication != ContextAuthentication::ApiKey
        }) {
            return Err(ModelError::InvalidRequest);
        }
        if let Some(prompt) = &self.compact_prompt {
            if self.binding.kind
                != research_agent_core::context::compaction_strategy::CompactionKind::TextSummary
            {
                return Err(ModelError::InvalidRequest);
            }
            research_agent_core::context::text_compaction::validate_compact_prompt(prompt)
                .map_err(|_| ModelError::InvalidRequest)?;
        }
        self.binding
            .validate()
            .map_err(|_| ModelError::InvalidRequest)?;
        if self.schema_version != 1
            || self.max_output_tokens == 0
            || self.unknown_window_trigger_tokens == Some(0)
            || self.comp_hash.as_ref().is_some_and(|hash| {
                hash.trim().is_empty() || hash.len() > 256 || hash.chars().any(char::is_control)
            })
        {
            return Err(ModelError::InvalidRequest);
        }
        if let Some(budget) = self.budget {
            if budget.output_reserve_tokens != u64::from(self.max_output_tokens) {
                return Err(ModelError::InvalidRequest);
            }
            budget
                .limits_for_scope(self.scope)
                .map_err(|_| ModelError::InvalidRequest)?;
        }
        Ok(())
    }
}

pub(crate) struct SqliteContextPolicy {
    connection: SharedDatabase,
    conversation: Uuid,
    request: Uuid,
    turn: Uuid,
    snapshot: Uuid,
    policy: FrozenContextPolicy,
}
impl SqliteContextPolicy {
    /// Reopening a submission must retain its admitted budget even if provider
    /// metadata or local overrides changed. Verify the complete stored identity.
    pub(crate) fn restored<D: IntoDatabase + Clone>(
        connection: &D,
        record: &SubmissionRecord,
    ) -> Result<Option<FrozenContextPolicy>, ModelError> {
        let connection = connection.clone().into_database();
        let db = connection.lock().map_err(|_| ModelError::Unavailable)?;
        let encoded: Option<String> = db.query_row(
            "select p.policy_json from agent_context_policies p join agent_submissions s on s.turn_id=p.turn_id where s.conversation_id=?1 and s.request_id=?2 and s.turn_id=?3 and s.context_snapshot_id=?4",
            params![record.conversation_id.to_string(), record.request_id.to_string(), record.turn_id.to_string(), record.context_snapshot_id.to_string()],
            |row| row.get(0),
        ).optional().map_err(|_| ModelError::Unavailable)?;
        encoded
            .map(|encoded| {
                if encoded.len() > 131_072 {
                    return Err(ModelError::InvalidStream);
                }
                let policy: FrozenContextPolicy =
                    serde_json::from_str(&encoded).map_err(|_| ModelError::InvalidStream)?;
                policy.validate().map_err(|_| ModelError::InvalidStream)?;
                if policy.binding.model != record.input.model {
                    return Err(ModelError::InvalidRequest);
                }
                Ok(policy)
            })
            .transpose()
    }
    pub fn snapshot(&self) -> FrozenContextPolicy {
        self.policy.clone()
    }
    pub fn tool_output_policy(
        &self,
    ) -> Option<research_agent_core::model::tool_output::ToolOutputPolicy> {
        self.policy
            .tool_truncation_policy
            .and_then(|policy| policy.shared())
    }
    pub fn previous<'a>(
        &'a self,
        turn: Uuid,
        model: &'a research_agent_core::protocol::ModelIdentity,
    ) -> ModelFuture<'a, Option<(SubmissionRecord, FrozenContextPolicy)>> {
        let connection = self.connection.clone();
        let conversation = self.conversation;
        let model = model.clone();
        Box::pin(async move {
            tokio::task::spawn_blocking(move || {
                let mut db = connection.lock().map_err(|_| ModelError::Unavailable)?;
                let tx = db.transaction().map_err(|_| ModelError::Unavailable)?;
                // Reuse the frozen-input lineage contract: nested forks point
                // to the original submission and verify its snapshot identity.
                let record =
                    match super::submissions::read_historical_record(&tx, conversation, turn) {
                        Ok(record) => record,
                        Err(research_agent_core::submission::SubmissionError::NotFound) => {
                            return Ok(None);
                        }
                        Err(research_agent_core::submission::SubmissionError::Unavailable) => {
                            return Err(ModelError::Unavailable);
                        }
                        Err(_) => return Err(ModelError::InvalidStream),
                    };
                if record.input.model != model {
                    return Err(ModelError::InvalidStream);
                }
                let policy: Option<String> = tx
                    .query_row(
                        "select policy_json from agent_context_policies where turn_id=?1",
                        [record.turn_id.to_string()],
                        |row| row.get(0),
                    )
                    .optional()
                    .map_err(|_| ModelError::Unavailable)?;
                let Some(policy) = policy else {
                    return Ok(None);
                };
                let policy: FrozenContextPolicy =
                    serde_json::from_str(&policy).map_err(|_| ModelError::InvalidStream)?;
                policy.validate().map_err(|_| ModelError::InvalidStream)?;
                if policy.binding.model != model {
                    return Err(ModelError::InvalidStream);
                }
                tx.commit().map_err(|_| ModelError::Unavailable)?;
                Ok(Some((record, policy)))
            })
            .await
            .map_err(|_| ModelError::Unavailable)?
        })
    }
    pub(crate) fn new(
        connection: impl IntoDatabase,
        record: &SubmissionRecord,
        policy: FrozenContextPolicy,
    ) -> Result<Self, ModelError> {
        policy.validate()?;
        if policy.binding.model != record.input.model
            || [
                record.conversation_id,
                record.request_id,
                record.turn_id,
                record.context_snapshot_id,
            ]
            .iter()
            .any(Uuid::is_nil)
        {
            return Err(ModelError::InvalidRequest);
        }
        Ok(Self {
            connection: connection.into_database(),
            conversation: record.conversation_id,
            request: record.request_id,
            turn: record.turn_id,
            snapshot: record.context_snapshot_id,
            policy,
        })
    }
    pub fn prepare(&self) -> ModelFuture<'_, ()> {
        let db = self.connection.clone();
        let (conversation, request, turn, snapshot) = (
            self.conversation.to_string(),
            self.request.to_string(),
            self.turn.to_string(),
            self.snapshot.to_string(),
        );
        let policy = self.policy.clone();
        Box::pin(async move {
            tokio::task::spawn_blocking(move || {
                let mut db = db.lock().map_err(|_| ModelError::Unavailable)?;
                let tx = db.transaction_with_behavior(TransactionBehavior::Immediate).map_err(|_| ModelError::Unavailable)?;
                let input: Option<String> = tx.query_row("select s.input_json from agent_submissions s join agent_conversations c on c.id=s.conversation_id where s.conversation_id=?1 and s.request_id=?2 and s.turn_id=?3 and s.context_snapshot_id=?4 and c.archived=0", params![conversation,request,turn,snapshot], |row| row.get(0)).optional().map_err(|_| ModelError::Unavailable)?;
                let input: SubmissionInput = serde_json::from_str(&input.ok_or(ModelError::InvalidRequest)?).map_err(|_| ModelError::InvalidStream)?;
                if input.model != policy.binding.model { return Err(ModelError::InvalidRequest); }
                let encoded = serde_json::to_string(&policy).map_err(|_| ModelError::InvalidRequest)?;
                if encoded.len() > 131_072 { return Err(ModelError::InvalidRequest); }
                tx.execute("insert into agent_context_policies(turn_id,policy_json) values(?1,?2) on conflict(turn_id) do nothing", params![turn,encoded]).map_err(|_| ModelError::Unavailable)?;
                let saved: String = tx.query_row("select policy_json from agent_context_policies where turn_id=?1", [&turn], |row| row.get(0)).map_err(|_| ModelError::Unavailable)?;
                let saved: FrozenContextPolicy = serde_json::from_str(&saved).map_err(|_| ModelError::InvalidStream)?;
                saved.validate().map_err(|_| ModelError::InvalidStream)?;
                if saved != policy { return Err(ModelError::InvalidRequest); }
                tx.commit().map_err(|_| ModelError::Unavailable)
            }).await.map_err(|_| ModelError::Unavailable)?
        })
    }
}
