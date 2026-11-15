//! Accept the selected inventory resources without a host exchange or DB guard.
use super::*;
use crate::{
    agent::context_catalog::ContextCatalogReadError,
    runtime_plugins::context_catalog::{ContextCatalogResources, admission_failure},
};

impl LiteratureTraceService {
    pub(crate) async fn context_catalog_resources(
        &self,
        key: &str,
    ) -> Result<ContextCatalogResources, ContextCatalogReadError> {
        self.available()
            .map_err(|error| admission_failure(&error.code, key))?;
        let catalog = Arc::clone(&self.catalog);
        let selected = key.to_owned();
        let resources = tokio::task::spawn_blocking(move || {
            let package = package::accept_catalog(&catalog, &selected)
                .map_err(|error| admission_failure(&error.code, &selected))?;
            ContextCatalogResources::load(&catalog, &package)
        })
        .await
        .map_err(|_| admission_failure("plugin_host_accept_failed", key))??;
        self.available()
            .map_err(|error| admission_failure(&error.code, key))?;
        Ok(resources)
    }
}
