//! One accepted owner spans Native preparation and the original Core work.
use super::*;
use crate::{
    runtime_plugins::literature_attachments::{
        self, AttachmentFailure, AttachmentProgress, AttachmentRequest, AttachmentResources,
        AttachmentResult,
    },
    storage::AppStore,
};
use research_workbench_native_sdk::{
    attachments::{
        AttachmentPlanV1, AttachmentPreparationV1, AttachmentPrepareInputV1,
        RenameAttachmentInputV1,
    },
    catalog::{CatalogInput, CatalogOutput},
};

impl LiteratureTraceService {
    /// Core-only Docling location: the selected package grants one fixed read.
    /// This never initializes a host or sends a path through Native RPC.
    pub(crate) fn docling_attachment_path(
        &self,
        db: &rusqlite::Connection,
        key: &str,
        content_directory: &std::path::Path,
        attachment_id: &str,
    ) -> NativeResult<std::path::PathBuf> {
        self.available().map_err(|error| error.package(key))?;
        let package = package::accept_attachments(&self.catalog, key)?;
        let resources = AttachmentResources::load_content_path(&self.catalog, &package)
            .map_err(|error| Self::error(error.code, error.message).package(key))?;
        if resources.package_key() != key {
            return Err(Self::error(
                "plugin_host_response_mismatch",
                "附件资源不属于本次文献包。",
            )
            .package(key));
        }
        literature_attachments::content_path(db, content_directory, &resources, attachment_id)
            .map_err(|error| error.native().package(key))
    }

    pub(crate) async fn attachments(
        self: &Arc<Self>,
        store: AppStore,
        key: String,
        request: AttachmentRequest,
    ) -> NativeResult<AttachmentResult> {
        let owner = Arc::clone(self);
        let selected = key.clone();
        tokio::spawn(async move { owner.attachments_owned(store, &selected, request).await })
            .await
            .map_err(|_| AttachmentFailure::unknown("execution").native())
            .and_then(|result| result)
            .map_err(|mut error| {
                error.package_key = Some(key);
                if error.attachments.is_none() {
                    error.attachments = Some(Box::new(
                        AttachmentProgress::new("preparation")
                            .fail(&error.code, error.message.clone()),
                    ));
                }
                error
            })
    }

    async fn attachments_owned(
        &self,
        store: AppStore,
        key: &str,
        request: AttachmentRequest,
    ) -> NativeResult<AttachmentResult> {
        self.available()?;
        // This same lifetime gate already drains Core work that releases the
        // host. Keep it through the blocking worker, including caller abort.
        let _lifetime = self.doi_operations.read().await;
        self.available()?;
        let catalog = Arc::clone(&self.catalog);
        let selected = key.to_owned();
        let resources = tokio::task::spawn_blocking(move || {
            let package = package::accept_attachments(&catalog, &selected)?;
            let resources = AttachmentResources::load(&catalog, &package)
                .map_err(|e| Self::error(e.code, e.message))?;
            if resources.package_key() != selected {
                return Err(Self::error(
                    "plugin_host_response_mismatch",
                    "附件资源不属于本次文献包。",
                ));
            }
            Ok(resources)
        })
        .await
        .map_err(|_| Self::error("plugin_host_accept_failed", "附件资源核对任务未完成。"))??;
        #[cfg(not(unix))]
        {
            let _ = (store, resources, request);
            Err(Self::error(
                "plugin_host_platform_unsupported",
                "此平台尚未提供文献宿主。",
            ))
        }
        #[cfg(unix)]
        {
            if matches!(
                &request,
                AttachmentRequest::Read { .. }
                    | AttachmentRequest::GetState { .. }
                    | AttachmentRequest::IsIndexed { .. }
            ) {
                self.available()?;
                return tokio::task::spawn_blocking(move || {
                    literature_attachments::execute(&store, &resources, request, None)
                        .map_err(AttachmentFailure::native)
                })
                .await
                .map_err(|_| AttachmentFailure::unknown("execution").native())?;
            }
            let mut slot = self.host.lock().await;
            self.available()?;
            let mut before = None;
            let result = async {
                let host = self.ensure_host(&mut slot, key).await?;
                before = host.request_identity();
                self.run_attachments(host, store, resources, request)
                    .await
                    .map_err(|mut error| {
                        if let Some((instance, request)) = host.request_identity() {
                            error.instance_id = Some(instance.clone());
                            if before.as_ref() != Some(&(instance, request.clone())) {
                                error.request_id = Some(request);
                            }
                        }
                        error
                    })
            }
            .await;
            self.finish_domain(&mut slot, result)
                .await
                .map_err(|mut error| {
                    // Cleanup can bind the host's last exchange again. A command
                    // that made no new exchange must not inherit an older ID.
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
    async fn run_attachments(
        &self,
        host: &mut NativeHost,
        store: AppStore,
        resources: AttachmentResources,
        request: AttachmentRequest,
    ) -> NativeResult<AttachmentResult> {
        let input = match &request {
            AttachmentRequest::Rename {
                item_id,
                attachment_id,
                display_name,
            } => {
                // Preserve owner failure before pure name rejection. No guard
                // or admission survives the Native wait.
                let probe = store.clone();
                tokio::task::spawn_blocking(move || {
                    drop(
                        probe
                            .connection()
                            .map_err(|e| AttachmentProgress::new("owner").owner(e).native())?,
                    );
                    Ok::<_, Box<NativeHostError>>(())
                })
                .await
                .map_err(|_| AttachmentFailure::unknown("owner").native())??;
                Some(AttachmentPrepareInputV1::Rename {
                    request: RenameAttachmentInputV1 {
                        item_id: item_id.clone(),
                        attachment_id: attachment_id.clone(),
                        display_name: display_name.clone(),
                    },
                })
            }
            AttachmentRequest::SaveState { input } => {
                Some(AttachmentPrepareInputV1::SaveReadingState {
                    request: input.clone(),
                })
            }
            _ => None,
        };
        let plan = if let Some(input) = input {
            let stopped = self.stopped.notified();
            tokio::pin!(stopped);
            stopped.as_mut().enable();
            self.available()?;
            let result = tokio::select! {
                biased;
                _ = &mut stopped => Err(host.error("plugin_host_stopping", "应用正在关闭附件准备。")),
                output = host.catalog_call(CatalogInput::PrepareLiteratureAttachment { input: Box::new(input.clone()) }, Duration::from_secs(60)) => output,
            }?;
            let CatalogOutput::LiteratureAttachmentPreparation { outcome } = result else {
                return Err(host.error("plugin_host_response_mismatch", "附件准备回复种类不匹配。"));
            };
            match *outcome {
                AttachmentPreparationV1::Ready { plan } if plan_matches(&input, &plan) => {
                    Some(*plan)
                }
                AttachmentPreparationV1::Rejected {
                    input: original,
                    reason,
                } if *original == input => {
                    if matches!(input, AttachmentPrepareInputV1::Rename { .. }) {
                        let probe = store.clone();
                        tokio::task::spawn_blocking(move || {
                            let _db = probe
                                .connection()
                                .map_err(|e| AttachmentProgress::new("owner").owner(e).native())?;
                            Err::<(), _>(
                                AttachmentProgress::new("validation")
                                    .fail(reason.code(), reason.message())
                                    .native(),
                            )
                        })
                        .await
                        .map_err(|_| AttachmentFailure::unknown("validation").native())??;
                    }
                    return Err(AttachmentProgress::new("validation")
                        .fail(reason.code(), reason.message())
                        .native());
                }
                _ => {
                    return Err(host.error(
                        "plugin_host_response_mismatch",
                        "附件准备与本次原输入不匹配。",
                    ));
                }
            }
        } else {
            None
        };
        self.available()?;
        // Caller cancellation never drops an accepted file/SQL write. The host
        // mutex remains held until the worker has settled, including cleanup.
        tokio::task::spawn_blocking(move || {
            literature_attachments::execute(&store, &resources, request, plan)
                .map_err(AttachmentFailure::native)
        })
        .await
        .map_err(|_| AttachmentFailure::unknown("execution").native())
        .and_then(|result| result)
    }
}

#[cfg(unix)]
fn plan_matches(input: &AttachmentPrepareInputV1, plan: &AttachmentPlanV1) -> bool {
    match (input, plan) {
        (
            AttachmentPrepareInputV1::Rename { request: input },
            AttachmentPlanV1::Rename { request, .. },
        ) => input == request,
        (
            AttachmentPrepareInputV1::SaveReadingState { request: input },
            AttachmentPlanV1::SaveReadingState {
                request,
                bounded_zoom,
            },
        ) => {
            input == request
                && bounded_zoom.to_f64().is_finite()
                && (0.5..=3.0).contains(&bounded_zoom.to_f64())
        }
        _ => false,
    }
}

#[cfg(all(test, unix))]
mod tests;
