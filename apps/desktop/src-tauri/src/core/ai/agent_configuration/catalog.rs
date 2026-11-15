//! Shared catalog ports and exact-model metadata admission.
use super::*;
impl AgentModelResolver {
    pub(crate) async fn codex_catalog_versions(
        &self,
        refresh: bool,
    ) -> Result<crate::ai::codex_model_catalog::CodexCatalogVersions, crate::ai::error::AiError>
    {
        self.codex_catalog
            .as_ref()
            .ok_or(crate::ai::error::AiError::TemporaryUnavailable)?
            .versions(refresh)
            .await
    }
    pub(crate) async fn select_codex_catalog_version(
        &self,
        version: String,
    ) -> Result<crate::ai::codex_model_catalog::CodexCatalogVersions, crate::ai::error::AiError>
    {
        self.codex_catalog
            .as_ref()
            .ok_or(crate::ai::error::AiError::TemporaryUnavailable)?
            .select_version(version)
            .await
    }
    pub(crate) fn with_context_metadata(mut self, app: &tauri::AppHandle) -> Self {
        self.context_metadata = crate::ai::model_metadata::cache_path(app);
        self.codex_catalog = self
            .chatgpt
            .clone()
            .and_then(|source| {
                crate::ai::codex_model_catalog::CodexModelCatalog::from_app(app, source)
            })
            .map(Arc::new);
        self
    }

    pub(crate) async fn chatgpt_models(
        &self,
        refresh: bool,
    ) -> Result<crate::ai::model::LlmProviderModels, crate::ai::error::AiError> {
        self.codex_catalog
            .as_ref()
            .ok_or(crate::ai::error::AiError::TemporaryUnavailable)?
            .load(refresh)
            .await
            .map(|catalog| catalog.provider_models())
    }
    pub(crate) async fn invalidate_chatgpt_models(&self) {
        if let Some(catalog) = &self.codex_catalog {
            catalog.invalidate().await;
        }
    }

    pub(super) async fn resolve_catalog(
        &self,
        prepared: &mut PreparedAgentModel,
    ) -> Result<(), AgentModelPreparationError> {
        if prepared.is_chatgpt() {
            let snapshot = match &self.codex_catalog {
                Some(catalog) => catalog.load(false).await.map_err(|error| match error {
                    crate::ai::error::AiError::Provider("authentication") => {
                        AgentModelPreparationError::Credentials
                    }
                    _ => AgentModelPreparationError::Settings,
                })?,
                #[cfg(test)]
                None => crate::ai::codex_model_catalog::bundled_snapshot()
                    .map_err(|_| AgentModelPreparationError::Settings)?,
                #[cfg(not(test))]
                None => return Err(AgentModelPreparationError::ChatGptTransportUnavailable),
            };
            if let Some(model) = snapshot.model(&prepared.model.model_id) {
                prepared.context_window =
                    model
                        .window()
                        .map(|tokens| crate::ai::model_metadata::ResolvedContextWindow {
                            tokens: tokens as i64,
                            source: "provider_catalog".into(),
                        });
                prepared.catalog_budget = Some((
                    model.effective_context_window_percent,
                    model.auto_compact_token_limit,
                ));
                prepared.catalog_truncation_policy = model.truncation_policy;
                prepared.catalog_capabilities = Some(model.capabilities());
                if let Some(scope) = snapshot.account_scope()
                    && let PreparedAuthentication::ChatGpt(source) = &prepared.authentication
                {
                    prepared.authentication =
                        PreparedAuthentication::ChatGpt(Arc::new(CatalogAccountCredentials {
                            source: source.clone(),
                            scope,
                        }));
                }
                if prepared.service_tier.as_deref() == Some("fast")
                    && !model.descriptor(false).supports_fast_mode
                {
                    return Err(AgentModelPreparationError::Settings);
                }
            } else {
                return Err(AgentModelPreparationError::Settings);
            }
        }
        Ok(())
    }
}

/// Account changes between metadata preparation and sampling require fresh
/// preparation. Refreshes for the same account retain the normal lease lifecycle.
struct CatalogAccountCredentials {
    source: Arc<dyn ChatGptCredentialSource>,
    scope: uuid::Uuid,
}
impl ChatGptCredentialSource for CatalogAccountCredentials {
    fn acquire(&self) -> crate::agent::authentication::CredentialFuture<'_> {
        Box::pin(async move {
            let lease = self.source.acquire().await?;
            lease.check()?;
            if lease.scope_id() != self.scope {
                return Err(crate::agent::authentication::AuthenticationError::Revoked);
            }
            Ok(lease)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    struct Source {
        changed: AtomicBool,
        epochs: Mutex<Vec<tokio::sync::watch::Sender<uuid::Uuid>>>,
    }
    impl ChatGptCredentialSource for Source {
        fn acquire(&self) -> crate::agent::authentication::CredentialFuture<'_> {
            Box::pin(async move {
                let account = if self.changed.load(Ordering::SeqCst) {
                    "catalog-account-b"
                } else {
                    "catalog-account-a"
                };
                let (lease, epoch) =
                    crate::agent::authentication::tests::synthetic_lease_for_account(account);
                self.epochs.lock().unwrap().push(epoch);
                Ok(lease)
            })
        }
    }
    #[tokio::test]
    async fn prepared_catalog_credentials_refresh_only_within_the_same_account() {
        let source = Arc::new(Source {
            changed: AtomicBool::new(false),
            epochs: Mutex::new(vec![]),
        });
        let scope = source.acquire().await.unwrap().scope_id();
        let bound = CatalogAccountCredentials {
            source: source.clone(),
            scope,
        };
        assert_eq!(bound.acquire().await.unwrap().scope_id(), scope);
        source.changed.store(true, Ordering::SeqCst);
        assert!(matches!(
            bound.acquire().await,
            Err(crate::agent::authentication::AuthenticationError::Revoked)
        ));
    }
}
