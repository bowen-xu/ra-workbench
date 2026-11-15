//! One accepted owner retains the host until the fixed original SQL settles.
use super::*;
use crate::{
    data_interface::SharedDatabase,
    runtime_plugins::literature_reading_units::{
        self, ReadingUnitFailure, ReadingUnitProblem, ReadingUnitRequest, ReadingUnitResources,
        ReadingUnitResult,
    },
};
use research_workbench_native_sdk::catalog::{CatalogInput, CatalogOutput};
impl LiteratureTraceService {
    pub(crate) async fn reading_units(
        self: &Arc<Self>,
        database: SharedDatabase,
        key: String,
        request: ReadingUnitRequest,
    ) -> NativeResult<ReadingUnitResult> {
        let owner = Arc::clone(self);
        let selected = key.clone();
        let reading_unit_id = match &request {
            ReadingUnitRequest::List { .. } => None,
            ReadingUnitRequest::Save { draft } => draft.id.clone(),
        };
        tokio::spawn(async move {
            owner
                .reading_units_owned(database, &selected, request)
                .await
        })
        .await
        .map_err(|_| ReadingUnitFailure::unknown().native())
        .and_then(|v| v)
        .map_err(|mut e| {
            e.package_key = Some(key);
            if let Some(failure) = e.reading_units.as_mut()
                && failure.reading_unit_id.is_none()
            {
                failure.reading_unit_id = reading_unit_id;
            }
            e
        })
    }
    async fn reading_units_owned(
        &self,
        database: SharedDatabase,
        key: &str,
        request: ReadingUnitRequest,
    ) -> NativeResult<ReadingUnitResult> {
        self.available()?;
        let catalog = Arc::clone(&self.catalog);
        let selected = key.to_owned();
        let db = Arc::clone(&database);
        let resources = tokio::task::spawn_blocking(move || {
            let package = package::accept_reading_units(&catalog, &selected)?;
            let resources = ReadingUnitResources::load(&catalog, &package)
                .map_err(|e| Self::error(e.code, e.message))?;
            // The original owner admission precedes pure validation. Nothing
            // borrowed here survives the actual Native preparation await.
            drop(
                db.lock()
                    .map_err(|e| ReadingUnitFailure::owner(e).native())?,
            );
            Ok::<_, Box<NativeHostError>>(resources)
        })
        .await
        .map_err(|_| ReadingUnitFailure::unknown().native())??;
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
                    return Err(ReadingUnitFailure::new(
                        "preparation",
                        ReadingUnitProblem::Binding,
                    )
                    .native());
                }
                let host = self.ensure_host(&mut slot, key).await?;
                before = host.request_identity();
                self.run_reading_units(host, database, resources, request)
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
    async fn run_reading_units(
        &self,
        host: &mut NativeHost,
        database: SharedDatabase,
        resources: ReadingUnitResources,
        request: ReadingUnitRequest,
    ) -> NativeResult<ReadingUnitResult> {
        let stopped = self.stopped.notified();
        tokio::pin!(stopped);
        stopped.as_mut().enable();
        self.available()?;
        let output = tokio::select! {biased;
            _=&mut stopped=>Err(host.error("plugin_host_stopping","应用正在关闭阅读单元准备。")),
            output=host.catalog_call(CatalogInput::PrepareReadingUnit{input:Box::new(request.clone())},Duration::from_secs(60))=>output,
        }?;
        let CatalogOutput::ReadingUnitPreparation { outcome } = output else {
            return Err(
                ReadingUnitFailure::new("preparation", ReadingUnitProblem::Binding).native(),
            );
        };
        literature_reading_units::check_preparation(&request, &outcome)
            .map_err(ReadingUnitFailure::native)?;
        self.available()?;
        tokio::task::spawn_blocking(move || {
            let mut db = database
                .lock()
                .map_err(|e| ReadingUnitFailure::owner(e).native())?;
            literature_reading_units::apply(&mut db, &resources, *outcome)
                .map_err(ReadingUnitFailure::native)
        })
        .await
        .map_err(|_| ReadingUnitFailure::unknown().native())
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
