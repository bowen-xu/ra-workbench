//! One accepted owner retains the host until the fixed original SQL settles.
use super::*;
use crate::{
    data_interface::SharedDatabase,
    runtime_plugins::literature_concepts::{
        self, ConceptFailure, ConceptProblem, ConceptRequest, ConceptResources, ConceptResult,
    },
};
use research_workbench_native_sdk::catalog::{CatalogInput, CatalogOutput};
impl LiteratureTraceService {
    pub(crate) async fn concepts(
        self: &Arc<Self>,
        database: SharedDatabase,
        key: String,
        request: ConceptRequest,
    ) -> NativeResult<ConceptResult> {
        let owner = Arc::clone(self);
        let selected = key.clone();
        let concept_id = match &request {
            ConceptRequest::Search { .. } => None,
            ConceptRequest::Save { draft } => draft.id.clone(),
        };
        tokio::spawn(async move { owner.concepts_owned(database, &selected, request).await })
            .await
            .map_err(|_| ConceptFailure::unknown().native())
            .and_then(|v| v)
            .map_err(|mut e| {
                e.package_key = Some(key);
                if let Some(failure) = e.concepts.as_mut()
                    && failure.concept_id.is_none()
                {
                    failure.concept_id = concept_id;
                }
                e
            })
    }
    async fn concepts_owned(
        &self,
        database: SharedDatabase,
        key: &str,
        request: ConceptRequest,
    ) -> NativeResult<ConceptResult> {
        self.available()?;
        let catalog = Arc::clone(&self.catalog);
        let selected = key.to_owned();
        let db = Arc::clone(&database);
        let resources = tokio::task::spawn_blocking(move || {
            let package = package::accept_concepts(&catalog, &selected)?;
            let resources = ConceptResources::load(&catalog, &package)
                .map_err(|e| Self::error(e.code, e.message))?;
            // The original owner admission precedes pure validation. Nothing
            // borrowed here survives the actual Native preparation await.
            drop(db.lock().map_err(|e| ConceptFailure::owner(e).native())?);
            Ok::<_, Box<NativeHostError>>(resources)
        })
        .await
        .map_err(|_| ConceptFailure::unknown().native())??;
        #[cfg(not(unix))]
        {
            let _ = (database, resources, request);
            Err(Self::error(
                "plugin_host_platform_unsupported",
                "此平台尚未提供文献宿主。",
            ))
        }
        #[cfg(unix)]
        {
            let mut slot = self.host.lock().await;
            self.available()?;
            let mut before = None;
            let result = async {
                if resources.package_key() != key {
                    return Err(
                        ConceptFailure::new("preparation", ConceptProblem::Binding).native()
                    );
                }
                let host = self.ensure_host(&mut slot, key).await?;
                before = host.request_identity();
                self.run_concepts(host, database, resources, request)
                    .await
                    .map_err(|e| bind(host, e))
            }
            .await;
            self.finish_domain(&mut slot, result)
                .await
                .map_err(|mut error| {
                    if let Some((instance, request)) = before.as_ref()
                        && error.instance_id.as_ref() == Some(instance)
                        && error.request_id.as_ref() == Some(request)
                    {
                        error.request_id = None;
                    }
                    error
                })
        }
    }
    #[cfg(unix)]
    async fn run_concepts(
        &self,
        host: &mut NativeHost,
        database: SharedDatabase,
        resources: ConceptResources,
        request: ConceptRequest,
    ) -> NativeResult<ConceptResult> {
        let stopped = self.stopped.notified();
        tokio::pin!(stopped);
        stopped.as_mut().enable();
        self.available()?;
        let output = tokio::select! {biased;
            _=&mut stopped=>Err(host.error("plugin_host_stopping","应用正在关闭概念准备。")),
            output=host.catalog_call(CatalogInput::PrepareResearchConcept{input:Box::new(request.clone())},Duration::from_secs(60))=>output,
        }?;
        let CatalogOutput::ResearchConceptPreparation { outcome } = output else {
            return Err(ConceptFailure::new("preparation", ConceptProblem::Binding).native());
        };
        literature_concepts::check_preparation(&request, &outcome)
            .map_err(ConceptFailure::native)?;
        self.available()?;
        tokio::task::spawn_blocking(move || {
            let mut db = database
                .lock()
                .map_err(|e| ConceptFailure::owner(e).native())?;
            literature_concepts::apply(&mut db, &resources, *outcome)
                .map_err(ConceptFailure::native)
        })
        .await
        .map_err(|_| ConceptFailure::unknown().native())
        .and_then(|v| v)
    }
}
#[cfg(unix)]
fn bind(host: &NativeHost, mut error: Box<NativeHostError>) -> Box<NativeHostError> {
    if let Some((instance, request)) = host.request_identity() {
        error.instance_id = Some(instance);
        error.request_id = Some(request);
    }
    error
}

#[cfg(test)]
mod tests;
