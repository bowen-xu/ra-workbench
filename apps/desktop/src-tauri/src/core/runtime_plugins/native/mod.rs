//! Finite native profiles. No plugin business, SQL or generic RPC.
mod diagnostics;
pub(super) mod package;
mod profile;
#[cfg(unix)]
mod retirement;
mod safe_database_error;
#[cfg(unix)]
pub(super) mod staging;
#[cfg(unix)]
pub(super) mod transport;
mod wire;
pub(crate) use diagnostics::{
    NativeHostError, NativeResult, NativeTerminationDiagnostic, StorageMutationDiagnostic,
    StorageMutationOutcome,
};

pub(crate) use package::accept_trash_cleanup;
pub(crate) use profile::NativeProfile;
#[cfg(unix)]
pub(crate) use retirement::CleanupGuard;
pub(crate) use safe_database_error::SafeDatabaseError;
pub(crate) type NativePreviewError = NativeHostError;

use research_workbench_native_sdk::protocol::{
    ImportPreview, PreviewOutcome, RequestBody, ResponseBody,
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tokio::sync::{Mutex, Notify};

use super::RuntimePluginCatalog;

enum WritingCall {
    LegacyPreview(String),
    ProjectPreview(String),
    ProjectDecision(research_workbench_native_sdk::writing_project::Input),
}
impl WritingCall {
    fn profile(&self) -> NativeProfile {
        match self {
            Self::LegacyPreview(_) => NativeProfile::WritingPreview,
            _ => NativeProfile::WritingProjects,
        }
    }
}
enum WritingReply {
    Preview(ImportPreview),
    Project(Box<DecidedProject>),
}

pub(crate) struct DecidedProject {
    pub output: research_workbench_native_sdk::writing_project::Output,
    pub instance_id: String,
    pub request_id: String,
}

/// The existing admission lock retained across Core worker and file handoffs.
pub(crate) type ProjectOperation = Arc<tokio::sync::OwnedRwLockReadGuard<()>>;

/// One app-owned instance, one pending call, no retry or business cache.
pub(crate) struct NativePreviewService {
    catalog: Arc<RuntimePluginCatalog>,
    #[cfg(unix)]
    host: Mutex<Option<transport::NativeHost>>,
    stopping: AtomicBool,
    stopped: Notify,
    cleanup_failed: AtomicBool,
    // Accepted project persistence outlives its IPC waiter. Shutdown drains the
    // same service before the application closes its shared database owner.
    project_operations: Arc<tokio::sync::RwLock<()>>,
}

impl NativePreviewService {
    pub(crate) fn new(catalog: Arc<RuntimePluginCatalog>) -> Self {
        Self {
            catalog,
            #[cfg(unix)]
            host: Mutex::new(None),
            stopping: AtomicBool::new(false),
            stopped: Notify::new(),
            cleanup_failed: AtomicBool::new(false),
            project_operations: Arc::new(tokio::sync::RwLock::new(())),
        }
    }

    pub(crate) async fn preview(
        &self,
        package_key: String,
        source_path: String,
    ) -> NativeResult<ImportPreview> {
        if !self.catalog.native_writing_preview {
            return Err(NativePreviewError::new(
                "plugin_host_disabled",
                "当前组合未启用原生导入预览试点。",
            )
            .package(&package_key));
        }
        match self
            .call(package_key, WritingCall::LegacyPreview(source_path))
            .await?
        {
            WritingReply::Preview(preview) => Ok(preview),
            _ => Err(NativePreviewError::new(
                "plugin_host_response_mismatch",
                "原生导入预览结果种类不匹配。",
            )),
        }
    }

    pub(crate) async fn project_decision(
        &self,
        key: String,
        input: research_workbench_native_sdk::writing_project::Input,
    ) -> NativeResult<DecidedProject> {
        match self.call(key, WritingCall::ProjectDecision(input)).await? {
            WritingReply::Project(output) => Ok(*output),
            _ => Err(NativePreviewError::new(
                "plugin_host_response_mismatch",
                "写作工程结果种类不匹配。",
            )),
        }
    }

    pub(crate) async fn project_operation(&self, key: &str) -> NativeResult<ProjectOperation> {
        let operation = self.project_operations.clone().read_owned().await;
        if self.stopping.load(Ordering::Acquire) {
            return Err(NativeHostError::for_profile(
                NativeProfile::WritingProjects,
                "plugin_host_stopping",
                "应用正在关闭原生插件。",
            )
            .package(key));
        }
        if self.cleanup_failed.load(Ordering::Acquire) {
            return Err(NativeHostError::for_profile(
                NativeProfile::WritingProjects,
                "plugin_host_cleanup_blocked",
                "旧原生实例未确认回收；请重启应用后再试。",
            )
            .package(key));
        }
        Ok(Arc::new(operation))
    }

    pub(crate) async fn project_preview(
        &self,
        key: String,
        source_path: String,
    ) -> NativeResult<ImportPreview> {
        match self
            .call(key, WritingCall::ProjectPreview(source_path))
            .await?
        {
            WritingReply::Preview(preview) => Ok(preview),
            _ => Err(NativePreviewError::new(
                "plugin_host_response_mismatch",
                "写作导入预览结果种类不匹配。",
            )),
        }
    }

    async fn call(&self, package_key: String, call: WritingCall) -> NativeResult<WritingReply> {
        if self.cleanup_failed.load(Ordering::Acquire) {
            return Err(NativePreviewError::new(
                "plugin_host_cleanup_blocked",
                "旧原生实例未确认回收；请重启应用后再试。",
            )
            .package(&package_key));
        }
        if self.stopping.load(Ordering::Acquire) {
            return Err(NativePreviewError::new(
                "plugin_host_stopping",
                "应用正在关闭原生插件。",
            ));
        }
        #[cfg(not(unix))]
        {
            let _ = (package_key, call);
            Err(NativePreviewError::new(
                "plugin_host_platform_unsupported",
                "此平台尚未提供原生插件进程拥有者。",
            ))
        }
        #[cfg(unix)]
        {
            let mut slot = if matches!(&call, WritingCall::LegacyPreview(_)) {
                self.host.try_lock().map_err(|_| {
                    NativePreviewError::new("plugin_host_busy", "原生导入预览已有在途请求。")
                        .package(&package_key)
                })?
            } else {
                self.host.lock().await
            };
            // Register before checking stopping so a concurrent shutdown cannot
            // lose its wakeup between readiness and the select below.
            let shutdown = self.stopped.notified();
            tokio::pin!(shutdown);
            shutdown.as_mut().enable();
            if self.cleanup_failed.load(Ordering::Acquire) {
                return Err(NativePreviewError::new(
                    "plugin_host_cleanup_blocked",
                    "旧原生实例未确认回收；请重启应用后再试。",
                )
                .package(&package_key));
            }
            if self.stopping.load(Ordering::Acquire) {
                return Err(NativePreviewError::new(
                    "plugin_host_stopping",
                    "应用正在关闭原生插件。",
                ));
            }
            let result = tokio::select! {
                biased;
                _ = &mut shutdown => Err(NativePreviewError::new("plugin_host_stopping", "应用正在关闭原生插件。")),
                result = self.call_inner(&mut slot, &package_key, call) => result,
            };
            if result.is_err()
                && slot
                    .as_ref()
                    .is_some_and(|host| host.is_failed() || self.stopping.load(Ordering::Acquire))
                && let Some(mut host) = slot.take()
            {
                let cleanup = CleanupGuard::new(&self.cleanup_failed);
                let mut error = result.err().unwrap_or_else(|| {
                    NativePreviewError::new("plugin_host_failed", "原生导入预览未完成。")
                });
                host.finish_failed(&mut error).await;
                cleanup.finish(Some(error.as_ref()));
                return Err(error.package(&package_key));
            }
            result.map_err(|error| error.package(&package_key))
        }
    }

    #[cfg(unix)]
    async fn call_inner(
        &self,
        slot: &mut Option<transport::NativeHost>,
        key: &str,
        call: WritingCall,
    ) -> NativeResult<WritingReply> {
        let profile = call.profile();
        if let Some(host) = slot.as_mut() {
            if host.package_key() != key {
                return Err(NativePreviewError::new(
                    "plugin_host_restart_required",
                    "更换活动原生插件包需要重新启动应用。",
                ));
            }
            if host.profile() != profile {
                return Err(NativeHostError::for_profile(
                    profile,
                    "plugin_host_profile_unsupported",
                    "活动写作连接不属于所选协议。",
                ));
            }
            if host.is_failed() {
                let mut old = slot.take().ok_or_else(|| {
                    NativePreviewError::new("plugin_host_failed", "原生插件实例不可用。")
                })?;
                let cleanup = CleanupGuard::new(&self.cleanup_failed);
                let mut diagnostic = NativePreviewError::new(
                    "plugin_host_cancelled",
                    if profile == NativeProfile::WritingPreview {
                        "上次导入预览已取消，旧连接不可复用。数据库未写入。"
                    } else {
                        "上次写作调用未完成，旧连接不可复用；请核对原操作。"
                    },
                );
                old.finish_failed(&mut diagnostic).await;
                cleanup.finish(Some(diagnostic.as_ref()));
                if diagnostic.cleanup_error.is_some() || diagnostic.termination_error.is_some() {
                    return Err(diagnostic);
                }
            }
        }
        if slot.is_none() {
            let startup = async {
                let catalog = Arc::clone(&self.catalog);
                let owned_key = key.to_owned();
                let staged = tokio::task::spawn_blocking(move || {
                    if profile == NativeProfile::WritingPreview {
                        package::stage(&catalog, &owned_key)
                    } else {
                        package::stage_for(&catalog, &owned_key, profile)
                    }
                })
                .await
                .map_err(|_| {
                    NativePreviewError::new(
                        "plugin_host_stage_failed",
                        "原生插件资源准备任务未完成。",
                    )
                })??;
                *slot = Some(if profile == NativeProfile::WritingPreview {
                    transport::NativeHost::spawn(staged, key)?
                } else {
                    transport::NativeHost::spawn_for(staged, key, profile)?
                });
                slot.as_mut()
                    .ok_or_else(|| {
                        NativePreviewError::new("plugin_host_failed", "原生插件实例不可用。")
                    })?
                    .initialize()
                    .await
            };
            tokio::time::timeout(std::time::Duration::from_secs(10), startup)
                .await
                .map_err(|_| {
                    NativePreviewError::new(
                        "plugin_host_startup_timeout",
                        "原生插件准备／启动／握手超过10秒；此请求不会自动重放。",
                    )
                })??;
        }
        let host = slot
            .as_mut()
            .ok_or_else(|| NativePreviewError::new("plugin_host_failed", "原生插件实例不可用。"))?;
        match call {
            WritingCall::ProjectDecision(input) => {
                let output = host
                    .project_call(input, std::time::Duration::from_secs(60))
                    .await?;
                let (instance_id, request_id) = host.request_identity().ok_or_else(|| {
                    host.error(
                        "plugin_host_response_mismatch",
                        "写作结果缺少实际请求身份。",
                    )
                })?;
                Ok(WritingReply::Project(Box::new(DecidedProject {
                    output,
                    instance_id,
                    request_id,
                })))
            }
            WritingCall::ProjectPreview(source_path) => host
                .project_preview_call(source_path, std::time::Duration::from_secs(60))
                .await
                .map(WritingReply::Preview),
            WritingCall::LegacyPreview(source_path) => {
                match host
                    .request(
                        RequestBody::PreviewImport { source_path },
                        std::time::Duration::from_secs(60),
                    )
                    .await?
                {
                    ResponseBody::PreviewImportResult {
                        outcome: PreviewOutcome::Success { preview },
                    } => Ok(WritingReply::Preview(preview)),
                    ResponseBody::PreviewImportResult {
                        outcome: PreviewOutcome::Failure { error },
                    } => {
                        // Domain rejection is a valid reply. Keep the healthy process
                        // for a later explicit request; it is not a transport failure.
                        Err(host.domain_error(error))
                    }
                    _ => Err(NativePreviewError::new(
                        "plugin_host_protocol_invalid",
                        "原生插件回复种类与导入预览不匹配。",
                    )),
                }
            }
        }
    }

    pub(crate) async fn shutdown(&self) -> NativeResult<()> {
        self.stopping.store(true, Ordering::Release);
        self.stopped.notify_waiters();
        let _project_operations = self.project_operations.write().await;
        #[cfg(unix)]
        if let Some(mut host) = self.host.lock().await.take() {
            let cleanup = CleanupGuard::new(&self.cleanup_failed);
            let result = host.shutdown().await;
            cleanup.finish(result.as_ref().err().map(|error| error.as_ref()));
            result?;
        }
        if self.cleanup_failed.load(Ordering::Acquire) {
            let mut error = NativePreviewError::new(
                "plugin_host_cleanup_blocked",
                "原生实例或输入未确认收尾；不能报告关闭成功。",
            );
            error.add_cleanup("先前原生实例或输入清理失败。");
            return Err(error);
        }
        Ok(())
    }
}

#[tauri::command]
pub(crate) async fn preview_runtime_manuscript_import(
    service: tauri::State<'_, Arc<NativePreviewService>>,
    package_key: String,
    source_path: String,
) -> NativeResult<ImportPreview> {
    service.preview(package_key, source_path).await
}

#[tauri::command]
pub(crate) async fn preview_runtime_writing_project_import(
    service: tauri::State<'_, Arc<NativePreviewService>>,
    package_key: String,
    source_path: String,
) -> NativeResult<ImportPreview> {
    service.project_preview(package_key, source_path).await
}

#[cfg(all(test, unix))]
mod service_tests;
