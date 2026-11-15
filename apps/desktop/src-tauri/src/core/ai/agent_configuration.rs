//! Bridge existing application settings/credentials into the shared Agent.
//! ChatGPT metadata uses the authenticated catalog; preparation never starts inference.
#[path = "agent_configuration/catalog.rs"]
mod catalog;
#[cfg(all(test, debug_assertions, feature = "development-data"))]
#[path = "agent_configuration/preview.rs"]
pub(crate) mod preview;
use super::{
    credential::{JsonCredentialStore, ProviderSecret},
    llm_configuration,
    model::LlmModelSelectionDraft,
};
use crate::agent::authentication::ChatGptCredentialSource;
use crate::agent::model::{
    OutputTokenParameter,
    chat_transport::{ChatTransport, ChatTransportConfig},
};
use reqwest::header::HeaderValue;
use research_agent_core::{
    model::{ModelCapabilities, ModelError},
    protocol::ModelIdentity,
};
use rusqlite::{Connection, OptionalExtension};
#[cfg(test)]
use std::sync::Mutex;
use std::{sync::Arc, time::Duration};

#[derive(Clone, Copy)]
pub enum AgentHost {
    General,
    Writing,
    Reading,
    Review,
    Notes,
}
impl AgentHost {
    fn route(self) -> &'static str {
        match self {
            Self::General => "research_chat",
            Self::Writing => "writing_agent",
            Self::Reading => "paper_reading_chat",
            Self::Review => "paper_review_agent",
            Self::Notes => "note_agent",
        }
    }
}

#[derive(Clone)]
pub struct AgentModelSelection {
    pub provider_id: String,
    pub model_id: String,
    pub reasoning_effort: Option<String>,
    pub service_tier: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum AgentModelPreparationError {
    #[error("agent model settings are unavailable")]
    Settings,
    #[error("agent model provider is disabled")]
    Disabled,
    #[error("agent model credentials are unavailable")]
    Credentials,
    #[error("ChatGPT credential source is unavailable")]
    ChatGptTransportUnavailable,
    #[error("agent model protocol is unsupported")]
    UnsupportedProtocol,
    #[error("agent model preparation worker is unavailable")]
    Worker,
    #[error(transparent)]
    Transport(#[from] ModelError),
}

/// Owns the same connection/credential store as existing application settings.
#[derive(Clone)]
pub struct AgentModelResolver {
    #[cfg(all(test, debug_assertions, feature = "development-data"))]
    preview: Option<Arc<preview::ModelFactory>>,
    compaction_hooks: Option<Arc<dyn crate::agent::compaction_hook_factory::CompactionHookFactory>>,
    #[cfg(target_os = "macos")]
    native_hooks: Option<Arc<crate::agent::compaction_hook_config::native::NativeHookFactory>>,
    connection: crate::data_interface::SharedDatabase,
    credentials: JsonCredentialStore,
    chatgpt: Option<Arc<dyn ChatGptCredentialSource>>,
    context_metadata: Option<std::path::PathBuf>,
    codex_catalog: Option<Arc<super::codex_model_catalog::CodexModelCatalog>>,
}

impl AgentModelResolver {
    pub(crate) fn from_app_store(
        store: &crate::storage::AppStore,
        credentials: JsonCredentialStore,
    ) -> Self {
        Self {
            #[cfg(all(test, debug_assertions, feature = "development-data"))]
            preview: None,
            compaction_hooks: None,
            #[cfg(target_os = "macos")]
            native_hooks: None,
            connection: store.shared_agent_connection(),
            credentials,
            chatgpt: None,
            context_metadata: None,
            codex_catalog: None,
        }
    }
    pub(crate) fn with_chatgpt(mut self, source: Arc<dyn ChatGptCredentialSource>) -> Self {
        self.chatgpt = Some(source);
        self
    }
    /// Native dependency injection only. No default discovery or command consent.
    pub fn with_compaction_hooks(
        mut self,
        factory: Arc<dyn crate::agent::compaction_hook_factory::CompactionHookFactory>,
    ) -> Self {
        self.compaction_hooks = Some(factory);
        self
    }
    pub async fn prepare_compaction_hooks(
        &self,
        host: AgentHost,
        record: &research_agent_core::submission::SubmissionRecord,
    ) -> Result<
        crate::agent::compaction_hook_factory::PreparedCompactionHooks,
        AgentModelPreparationError,
    > {
        self.prepare_compaction_hooks_with_workspace(host, record, None)
            .await
    }
    #[cfg(target_os = "macos")]
    pub(crate) fn with_native_compaction_hooks(
        mut self,
        settings: Arc<crate::agent::compaction_hook_commands::HookConsentService>,
    ) -> Self {
        self.native_hooks = Some(Arc::new(
            crate::agent::compaction_hook_config::native::NativeHookFactory::new(settings),
        ));
        self
    }
    #[cfg(target_os = "macos")]
    pub(crate) async fn prepare_stop_hooks(
        &self,
        host: AgentHost,
        record: &research_agent_core::submission::SubmissionRecord,
        workspace: crate::agent::compaction_hook_factory::HookWorkspaceHandle,
        target: crate::agent::stop_hook_process::StopTarget,
    ) -> Result<
        Option<crate::agent::compaction_hook_config::native::NativeStopHandlers>,
        AgentModelPreparationError,
    > {
        match &self.native_hooks {
            Some(factory) => factory
                .prepare_stop_target(host, record, workspace, target)
                .await
                .map_err(Into::into),
            None => Ok(None),
        }
    }
    pub async fn prepare_compaction_hooks_with_workspace(
        &self,
        host: AgentHost,
        record: &research_agent_core::submission::SubmissionRecord,
        workspace: crate::agent::compaction_hook_factory::HookWorkspaceHandle,
    ) -> Result<
        crate::agent::compaction_hook_factory::PreparedCompactionHooks,
        AgentModelPreparationError,
    > {
        self.prepare_hooks(host, record, workspace, None).await
    }
    /// Freeze the budget from the already resolved execution profile. Legacy
    /// preparation ports cannot enable native commands without this snapshot.
    pub async fn prepare_compaction_hooks_for_profile(
        &self,
        host: AgentHost,
        record: &research_agent_core::submission::SubmissionRecord,
        profile: &crate::agent::execution_profile::AgentExecutionProfile,
        workspace: crate::agent::compaction_hook_factory::HookWorkspaceHandle,
    ) -> Result<
        crate::agent::compaction_hook_factory::PreparedCompactionHooks,
        AgentModelPreparationError,
    > {
        let budget = profile
            .hook_context_budget(&record.input.model)
            .map_err(|_| ModelError::UnsupportedCapability)?;
        self.prepare_hooks(host, record, workspace, Some(budget))
            .await
    }
    async fn prepare_hooks(
        &self,
        host: AgentHost,
        record: &research_agent_core::submission::SubmissionRecord,
        workspace: crate::agent::compaction_hook_factory::HookWorkspaceHandle,
        budget: Option<crate::agent::compaction_hook_spill::HookContextBudget>,
    ) -> Result<
        crate::agent::compaction_hook_factory::PreparedCompactionHooks,
        AgentModelPreparationError,
    > {
        match &self.compaction_hooks {
            Some(factory) => factory.prepare(host, record).await.map_err(Into::into),
            None => {
                #[cfg(target_os = "macos")]
                if let Some(factory) = &self.native_hooks {
                    return factory
                        .prepare(host, record, workspace, budget)
                        .await
                        .map_err(Into::into);
                }
                let _ = (workspace, budget);
                Ok(None)
            }
        }
    }
    /// Blocking database/file reads never run on the async/UI thread. This is
    /// read-only: canceling preparation cannot start a model or change settings.
    pub async fn prepare(
        &self,
        host: AgentHost,
        selection: Option<AgentModelSelection>,
    ) -> Result<PreparedAgentModel, AgentModelPreparationError> {
        self.prepare_retained(host, selection, Default::default())
            .await
    }
    /// A canceled caller cannot release its SQL-worker lease before the actual
    /// settings/credential read returns. Later async phases remain cancelable.
    pub(crate) async fn prepare_retained(
        &self,
        host: AgentHost,
        selection: Option<AgentModelSelection>,
        retention: crate::agent::repository::WorkerRetention,
    ) -> Result<PreparedAgentModel, AgentModelPreparationError> {
        #[cfg(all(test, debug_assertions, feature = "development-data"))]
        if let Some(factory) = &self.preview {
            return factory(host, selection);
        }
        let connection = self.connection.clone();
        let credentials = self.credentials.clone();
        let chatgpt = self.chatgpt.clone();
        let metadata = self.context_metadata.clone();
        let mut prepared = retention
            .spawn(move || {
                let connection = connection
                    .lock()
                    .map_err(|_| AgentModelPreparationError::Settings)?;
                let mut prepared = prepare(&connection, &credentials, host, selection, chatgpt)?;
                prepared.context_window = metadata
                    .as_ref()
                    .filter(|_| !prepared.is_chatgpt())
                    .and_then(|path| {
                        super::model_metadata::cached_exact_context_window(path, &prepared.model)
                    });
                Ok::<_, AgentModelPreparationError>(prepared)
            })
            .await
            .map_err(|_| AgentModelPreparationError::Worker)??;
        self.resolve_catalog(&mut prepared).await?;
        Ok(prepared)
    }
}

/// Native-only frozen configuration. No Debug/Serialize and no credential getter.
pub struct PreparedAgentModel {
    model: ModelIdentity,
    reasoning_effort: Option<String>,
    service_tier: Option<String>,
    base_url: String,
    timeout: Duration,
    authentication: PreparedAuthentication,
    context_window: Option<super::model_metadata::ResolvedContextWindow>,
    catalog_budget: Option<(u8, Option<u64>)>,
    catalog_truncation_policy: Option<super::codex_model_catalog::TruncationPolicy>,
    catalog_capabilities: Option<ModelCapabilities>,
}
enum PreparedAuthentication {
    ApiKey(ProviderSecret),
    ChatGpt(Arc<dyn ChatGptCredentialSource>),
    #[cfg(all(test, debug_assertions, feature = "development-data"))]
    Preview(preview::PreparedPreview),
}
impl PreparedAgentModel {
    pub(crate) fn context_window_tokens(&self) -> Option<u64> {
        self.context_window
            .as_ref()
            .and_then(|window| u64::try_from(window.tokens).ok())
    }
    pub(crate) fn catalog_budget(&self) -> Option<(u8, Option<u64>)> {
        self.catalog_budget
    }
    pub(crate) fn catalog_truncation_policy(
        &self,
    ) -> Option<super::codex_model_catalog::TruncationPolicy> {
        self.catalog_truncation_policy
    }
    pub fn is_chatgpt(&self) -> bool {
        #[cfg(all(test, debug_assertions, feature = "development-data"))]
        if matches!(self.authentication, PreparedAuthentication::Preview(_)) {
            return true;
        }
        matches!(self.authentication, PreparedAuthentication::ChatGpt(_))
    }
    pub fn into_chatgpt_transport(
        self,
        mut capabilities: ModelCapabilities,
        receipts: Arc<dyn crate::agent::model::responses_transport::ResponsesReceiptSink>,
    ) -> Result<
        crate::agent::model::responses_transport::ResponsesTransport,
        AgentModelPreparationError,
    > {
        // The host profile is an upper bound. Exact catalog membership is resolved
        // natively before constructing a transport, including for newly listed IDs.
        let declared = self
            .catalog_capabilities
            .as_ref()
            .ok_or(ModelError::UnsupportedCapability)?;
        capabilities.vision &= declared.vision;
        capabilities
            .reasoning_efforts
            .retain(|effort| declared.reasoning_efforts.contains(effort));
        if self
            .reasoning_effort
            .as_ref()
            .is_some_and(|effort| !capabilities.reasoning_efforts.contains(effort))
        {
            return Err(ModelError::UnsupportedCapability.into());
        }
        let source = match self.authentication {
            PreparedAuthentication::ChatGpt(source) => source,
            #[cfg(all(test, debug_assertions, feature = "development-data"))]
            PreparedAuthentication::Preview(preview) => {
                return preview::transport(
                    preview,
                    self.model,
                    self.timeout,
                    self.service_tier,
                    capabilities,
                    receipts,
                );
            }
            _ => return Err(AgentModelPreparationError::UnsupportedProtocol),
        };
        Ok(
            crate::agent::model::responses_transport::ResponsesTransport::chatgpt(
                crate::agent::model::responses_transport::ResponsesTransportConfig {
                    credential_scope_id: uuid::Uuid::nil(),
                    provider_id: self.model.provider_id,
                    timeout: self.timeout,
                    service_tier: self.service_tier,
                    capabilities,
                },
                source,
                receipts,
            )?,
        )
    }
    pub fn identity(&self) -> &ModelIdentity {
        &self.model
    }

    pub fn service_tier(&self) -> Option<&str> {
        self.service_tier.as_deref()
    }

    pub fn reasoning_effort(&self) -> Option<&str> {
        self.reasoning_effort.as_deref()
    }

    /// Caller supplies the capability profile for this resolved model, after
    /// checking identity. Preparation does not infer tools/vision from a name.
    pub fn into_chat_transport(
        self,
        capabilities: ModelCapabilities,
        token_parameter: OutputTokenParameter,
    ) -> Result<ChatTransport, AgentModelPreparationError> {
        self.into_chat_transport_with_policy(capabilities, token_parameter, Default::default())
    }

    pub fn into_chat_transport_with_policy(
        self,
        capabilities: ModelCapabilities,
        token_parameter: OutputTokenParameter,
        request_policy: crate::agent::model::ChatRequestPolicy,
    ) -> Result<ChatTransport, AgentModelPreparationError> {
        if self
            .reasoning_effort
            .as_ref()
            .is_some_and(|effort| !capabilities.reasoning_efforts.contains(effort))
        {
            return Err(ModelError::UnsupportedCapability.into());
        }
        let PreparedAuthentication::ApiKey(secret) = self.authentication else {
            return Err(AgentModelPreparationError::UnsupportedProtocol);
        };
        let authorization = HeaderValue::from_str(&format!("Bearer {}", secret.expose()))
            .map_err(|_| AgentModelPreparationError::Credentials)?;
        Ok(ChatTransport::new(
            ChatTransportConfig {
                request_policy,
                provider_id: self.model.provider_id,
                base_url: self.base_url,
                timeout: self.timeout,
                capabilities,
                output_token_parameter: token_parameter,
            },
            authorization,
        )?)
    }
}

fn prepare(
    connection: &Connection,
    credentials: &JsonCredentialStore,
    host: AgentHost,
    selection: Option<AgentModelSelection>,
    chatgpt: Option<Arc<dyn ChatGptCredentialSource>>,
) -> Result<PreparedAgentModel, AgentModelPreparationError> {
    let service_tier = selection
        .as_ref()
        .and_then(|value| value.service_tier.clone());
    let selection = selection.map(|value| LlmModelSelectionDraft {
        provider_id: value.provider_id,
        model_id: value.model_id,
        reasoning_effort: value.reasoning_effort,
    });
    // Legacy selection normalizes non-OpenAI effort away. Preserve an explicit
    // Agent override; the resolved adapter profile owns support validation.
    let override_effort = selection
        .as_ref()
        .and_then(|value| value.reasoning_effort.clone());
    let selected =
        llm_configuration::resolve_model_selection(connection, host.route(), selection.as_ref())
            .map_err(|_| AgentModelPreparationError::Settings)?;
    let reasoning_effort = override_effort.or(selected.reasoning_effort);
    if service_tier
        .as_deref()
        .is_some_and(|tier| selected.provider_id != "openai" || !matches!(tier, "fast" | "default"))
    {
        return Err(AgentModelPreparationError::Settings);
    }
    if reasoning_effort.as_ref().is_some_and(|value| {
        value.is_empty() || value.len() > 1024 || value.trim() != value || value.contains('\0')
    }) {
        return Err(AgentModelPreparationError::Settings);
    }
    // Do not touch API-key storage to masquerade a ChatGPT login as a key, or
    // dispatch an old Codex turn. Auth lease/Responses is a separate adapter.
    if selected.provider_id == "openai" {
        let source = chatgpt.ok_or(AgentModelPreparationError::ChatGptTransportUnavailable)?;
        let (enabled,timeout_ms,base_url):(bool,i64,String)=connection.query_row(
            "select enabled,timeout_ms,base_url from llm_provider_configs where provider_id='openai'",[],
            |row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).optional().map_err(|_|AgentModelPreparationError::Settings)?
            .ok_or(AgentModelPreparationError::Disabled)?;
        if !enabled {
            return Err(AgentModelPreparationError::Disabled);
        }
        if !(1000..=300000).contains(&timeout_ms) || !base_url.trim().is_empty() {
            return Err(AgentModelPreparationError::Settings);
        }
        return Ok(PreparedAgentModel {
            model: ModelIdentity {
                provider_id: selected.provider_id,
                model_id: selected.model_id,
                revision: None,
            },
            reasoning_effort,
            service_tier,
            base_url: String::new(),
            timeout: Duration::from_millis(timeout_ms as u64),
            authentication: PreparedAuthentication::ChatGpt(source),
            context_window: None,
            catalog_budget: None,
            catalog_truncation_policy: None,
            catalog_capabilities: None,
        });
    }
    let configuration = llm_configuration::get_configuration(connection, credentials)
        .map_err(|_| AgentModelPreparationError::Settings)?;
    let provider = configuration
        .providers
        .into_iter()
        .find(|p| p.provider_id == selected.provider_id)
        .ok_or(AgentModelPreparationError::Settings)?;
    if !provider.enabled {
        return Err(AgentModelPreparationError::Disabled);
    }
    if provider.protocol != "openai-compatible"
        || provider.auth_mode != "api-key"
        || !provider.model_kinds.iter().any(|kind| kind == "text")
    {
        return Err(AgentModelPreparationError::UnsupportedProtocol);
    }
    llm_configuration::validate_base_url(&provider.provider_id, &provider.base_url)
        .map_err(|_| AgentModelPreparationError::Settings)?;
    let timeout_ms = u64::try_from(provider.timeout_ms)
        .ok()
        .filter(|ms| (1000..=300000).contains(ms))
        .ok_or(AgentModelPreparationError::Settings)?;
    let secret = credentials
        .resolve(&provider.provider_id)
        .map_err(|_| AgentModelPreparationError::Credentials)?;
    Ok(PreparedAgentModel {
        model: ModelIdentity {
            provider_id: provider.provider_id,
            model_id: selected.model_id,
            revision: None,
        },
        reasoning_effort,
        service_tier,
        base_url: provider.base_url,
        timeout: Duration::from_millis(timeout_ms),
        authentication: PreparedAuthentication::ApiKey(secret),
        context_window: None,
        catalog_budget: None,
        catalog_truncation_policy: None,
        catalog_capabilities: None,
    })
}

#[cfg(test)]
#[path = "agent_configuration/tests.rs"]
mod tests;
