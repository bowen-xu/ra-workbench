//! Explicit test dependency injection. This branch never opens a
//! credential handle or consults a real provider/catalog as a fallback.
use super::*;
use crate::agent::model::responses_transport::{
    PreviewResponsesWire, ResponsesReceiptSink, ResponsesTransport, ResponsesTransportConfig,
};

pub(crate) type ModelFactory = dyn Fn(
        AgentHost,
        Option<AgentModelSelection>,
    ) -> Result<PreparedAgentModel, AgentModelPreparationError>
    + Send
    + Sync;
pub(super) struct PreparedPreview {
    pub wire: Arc<dyn PreviewResponsesWire>,
    pub account: uuid::Uuid,
}

impl AgentModelResolver {
    pub(crate) fn for_preview(
        store: &crate::storage::AppStore,
        factory: Arc<ModelFactory>,
    ) -> Result<Self, AgentModelPreparationError> {
        let root = store
            .content_directory()
            .parent()
            .ok_or(AgentModelPreparationError::Settings)?;
        // A handle is retained for the production resolver layout. It is never
        // read: prepare() takes the explicit preview branch before any I/O.
        let mut resolver = Self::from_app_store(
            store,
            JsonCredentialStore::from_path(
                root.join("configuration/disabled-preview-credentials.json"),
            ),
        );
        resolver.preview = Some(factory);
        Ok(resolver)
    }
}
impl PreparedAgentModel {
    pub(crate) fn for_preview(
        model: ModelIdentity,
        reasoning_effort: Option<String>,
        service_tier: Option<String>,
        capabilities: ModelCapabilities,
        wire: Arc<dyn PreviewResponsesWire>,
        account: uuid::Uuid,
    ) -> Result<Self, AgentModelPreparationError> {
        if model.provider_id != "openai" || model.model_id.trim().is_empty() || account.is_nil() {
            return Err(AgentModelPreparationError::Settings);
        }
        // The fixture uses the pinned, local synthetic model metadata; it never
        // consults the account catalog or the network.
        let catalog_truncation_policy = super::super::codex_model_catalog::bundled_snapshot()
            .map_err(|_| AgentModelPreparationError::Settings)?
            .model(&model.model_id)
            .and_then(|entry| entry.truncation_policy);
        Ok(Self {
            model,
            reasoning_effort,
            service_tier,
            base_url: String::new(),
            timeout: Duration::from_secs(120),
            authentication: PreparedAuthentication::Preview(PreparedPreview { wire, account }),
            context_window: None,
            catalog_budget: None,
            catalog_truncation_policy,
            catalog_capabilities: Some(capabilities),
        })
    }
}
pub(super) fn transport(
    prepared: PreparedPreview,
    model: ModelIdentity,
    timeout: Duration,
    service_tier: Option<String>,
    capabilities: ModelCapabilities,
    receipts: Arc<dyn ResponsesReceiptSink>,
) -> Result<ResponsesTransport, AgentModelPreparationError> {
    ResponsesTransport::preview(
        ResponsesTransportConfig {
            credential_scope_id: prepared.account,
            provider_id: model.provider_id,
            timeout,
            service_tier,
            capabilities,
        },
        prepared.wire,
        receipts,
    )
    .map_err(Into::into)
}

/// Metadata and prepared credentials share one fixture within the production
/// profile upper bound. It advertises only the reviewed UI execution modes.
pub(crate) fn capabilities_fixture(
    model: &ModelIdentity,
) -> Result<ModelCapabilities, AgentModelPreparationError> {
    let mut capabilities = crate::agent::execution_profile::execution_profile(
        model,
        crate::agent::execution_profile::AgentExecutionMode::ChatGptDefault,
    )
    .map_err(|_| AgentModelPreparationError::Settings)?
    .capabilities;
    capabilities
        .reasoning_efforts
        .retain(|effort| matches!(effort.as_str(), "low" | "medium" | "high" | "xhigh" | "max"));
    Ok(capabilities)
}
