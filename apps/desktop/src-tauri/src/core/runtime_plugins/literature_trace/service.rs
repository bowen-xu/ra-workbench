//! Core-owned literature host and accepted-operation lifetimes. Fixed storage work uses Core adapters.
mod analysis_history;
mod analysis_workflow;
pub(crate) use analysis_workflow::AttemptFailure;
#[cfg(test)]
pub(crate) use analysis_workflow::StartAnalysisAttempt;
mod attachments;
mod bibliography;
mod browser_capture;
mod catalog;
pub(crate) use bibliography::DecidedBibliography;
mod concept_algorithms;
mod concept_capture;
mod concept_identity;
mod concept_retrieval;
mod concepts;
mod context_catalog;
mod context_metadata;
mod derived_views;
mod doi;
mod duplicate_merge;
mod duplicate_review;
mod duplicate_undo;
mod evidence_matrix;
mod evidence_read;
mod evidence_write;
mod import_jobs;
mod knowledge_failure;
mod knowledge_policy;
mod lifecycle;
mod manual_definitions;
mod manual_relations;
mod metadata;
mod nal_annotation;
mod nal_bootstrap;
mod nal_mutation;
mod nal_query;
mod paper_confirmation;
mod paper_prepare;
mod paper_preview;
mod pdf_import;
mod reading_evidence;
mod reading_history;
mod reading_units;
mod reference_records;
mod research_export;
mod save;
mod semantic_index;
mod structured_hbs;
mod structured_source;
pub(crate) use structured_source::SourceReadError;
mod trash_cleanup;
use super::data::TraceResources;
#[cfg(unix)]
use crate::runtime_plugins::native::{CleanupGuard, transport::NativeHost};
use crate::runtime_plugins::{
    RuntimePluginCatalog,
    native::{NativeHostError, NativeProfile, NativeResult, package},
};
use research_workbench_native_sdk::literature_trace_protocol::{SnapshotInput, TraceDecision};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::sync::{Mutex, Notify};

/// Core-private success metadata for diagnostics in the later caller transaction.
#[derive(Debug)]
pub(crate) struct DecidedTrace {
    pub decision: TraceDecision,
    pub instance_id: String,
    pub request_id: String,
}

pub(crate) struct LiteratureTraceService {
    catalog: Arc<RuntimePluginCatalog>,
    #[cfg(unix)]
    host: Mutex<Option<NativeHost>>,
    stopping: AtomicBool,
    stopped: Notify,
    cleanup_failed: AtomicBool,
    #[cfg(test)]
    admission_observed: Option<Arc<Notify>>,
    // DOI/PDF stages and Core-owned legacy imports retain this lifetime gate
    // while releasing the host, so shutdown drains accepted work before storage.
    doi_operations: tokio::sync::RwLock<()>,
}

impl LiteratureTraceService {
    pub(crate) fn new(catalog: Arc<RuntimePluginCatalog>) -> Self {
        Self {
            catalog,
            #[cfg(unix)]
            host: Mutex::new(None),
            stopping: AtomicBool::new(false),
            stopped: Notify::new(),
            cleanup_failed: AtomicBool::new(false),
            #[cfg(test)]
            admission_observed: None,
            doi_operations: tokio::sync::RwLock::new(()),
        }
    }

    fn error(code: &str, message: impl Into<String>) -> Box<NativeHostError> {
        NativeHostError::for_profile(NativeProfile::NoteTraceDecision, code, message)
    }

    fn available(&self) -> NativeResult<()> {
        if self.cleanup_failed.load(Ordering::Acquire) {
            return Err(Self::error(
                "plugin_host_cleanup_blocked",
                "旧原生实例或输入未确认收尾；请重启应用后再试。",
            ));
        }
        if self.stopping.load(Ordering::Acquire) {
            return Err(Self::error(
                "plugin_host_stopping",
                "应用正在关闭原生插件。",
            ));
        }
        Ok(())
    }

    /// Refresh and validate exactly the caller's selected key, without a DB guard.
    /// No package ID, newest version, manifest or default implicitly selects one.
    pub(crate) async fn accept(&self, package_key: &str) -> NativeResult<TraceResources> {
        self.available()
            .map_err(|error| error.package(package_key))?;
        let catalog = Arc::clone(&self.catalog);
        let key = package_key.to_owned();
        tokio::task::spawn_blocking(move || {
            let selected = package::accept(&catalog, &key, NativeProfile::NoteTraceDecision)?;
            TraceResources::load(&catalog, &selected)
                .map_err(|error| Self::error(error.code, &error.message).package(&key))
        })
        .await
        .map_err(|_| {
            Self::error("plugin_host_accept_failed", "原生插件包核对任务未完成。")
                .package(package_key)
        })?
    }

    pub(crate) async fn decide(
        &self,
        package_key: String,
        input: SnapshotInput,
    ) -> NativeResult<DecidedTrace> {
        self.available()
            .map_err(|error| error.package(&package_key))?;
        #[cfg(not(unix))]
        {
            let _ = input;
            Err(Self::error(
                "plugin_host_platform_unsupported",
                "此平台尚未提供原生插件进程拥有者。",
            )
            .package(&package_key))
        }
        #[cfg(unix)]
        {
            let mut slot = self.host.try_lock().map_err(|_| {
                Self::error("plugin_host_busy", "文献原生实例已有在途请求。").package(&package_key)
            })?;
            let shutdown = self.stopped.notified();
            tokio::pin!(shutdown);
            shutdown.as_mut().enable();
            self.available()
                .map_err(|error| error.package(&package_key))?;
            let result = tokio::select! {
                biased;
                _ = &mut shutdown => Err(Self::error("plugin_host_stopping", "应用正在关闭原生插件。")),
                result = self.decide_inner(&mut slot, &package_key, input) => result,
            };
            if result.is_err()
                && slot
                    .as_ref()
                    .is_some_and(|host| host.is_failed() || self.stopping.load(Ordering::Acquire))
                && let Some(mut host) = slot.take()
            {
                let cleanup = CleanupGuard::new(&self.cleanup_failed);
                let mut error = match result {
                    Err(error) => error,
                    Ok(_) => unreachable!("result was checked above"),
                };
                host.finish_failed(&mut error).await;
                cleanup.finish(Some(error.as_ref()));
                return Err(error.package(&package_key));
            }
            result.map_err(|error| error.package(&package_key))
        }
    }

    #[cfg(unix)]
    async fn decide_inner(
        &self,
        slot: &mut Option<NativeHost>,
        key: &str,
        input: SnapshotInput,
    ) -> NativeResult<DecidedTrace> {
        let host = self.ensure_host(slot, key).await?;
        let decision = host
            .decide_note_trace(input, Duration::from_secs(60))
            .await?;
        let (instance_id, request_id) = host.request_identity().ok_or_else(|| {
            host.error(
                "plugin_host_response_mismatch",
                "文献决定缺少已完成请求的身份。",
            )
        })?;
        Ok(DecidedTrace {
            decision,
            instance_id,
            request_id,
        })
    }

    #[cfg(unix)]
    async fn ensure_host<'slot>(
        &self,
        slot: &'slot mut Option<NativeHost>,
        key: &str,
    ) -> NativeResult<&'slot mut NativeHost> {
        if let Some(host) = slot.as_mut() {
            if host.package_key() != key {
                return Err(Self::error(
                    "plugin_host_restart_required",
                    "更换活动原生插件包需要重新启动应用。",
                ));
            }
            if host.is_failed() {
                let mut old = slot
                    .take()
                    .ok_or_else(|| Self::error("plugin_host_failed", "原生插件实例不可用。"))?;
                let cleanup = CleanupGuard::new(&self.cleanup_failed);
                let mut error = Self::error(
                    "plugin_host_cancelled",
                    "上次文献决定已取消，旧连接不可复用。",
                );
                old.finish_failed(&mut error).await;
                cleanup.finish(Some(error.as_ref()));
                if error.cleanup_error.is_some() || error.termination_error.is_some() {
                    return Err(error);
                }
            }
        }
        if slot.is_none() {
            let startup = async {
                let catalog = Arc::clone(&self.catalog);
                let selected_key = key.to_owned();
                let staged = tokio::task::spawn_blocking(move || {
                    package::accept(&catalog, &selected_key, NativeProfile::NoteTraceDecision)?;
                    package::stage_for(&catalog, &selected_key, NativeProfile::NoteTraceDecision)
                })
                .await
                .map_err(|_| {
                    Self::error("plugin_host_stage_failed", "原生插件资源准备任务未完成。")
                })??;
                *slot = Some(NativeHost::spawn_for(
                    staged,
                    key,
                    NativeProfile::NoteTraceDecision,
                )?);
                slot.as_mut()
                    .ok_or_else(|| Self::error("plugin_host_failed", "原生插件实例不可用。"))?
                    .initialize()
                    .await
            };
            tokio::time::timeout(Duration::from_secs(10), startup)
                .await
                .map_err(|_| {
                    Self::error(
                        "plugin_host_startup_timeout",
                        "原生插件准备／启动／握手超过10秒；此请求不会自动重放。",
                    )
                })??;
        }
        let host = slot
            .as_mut()
            .ok_or_else(|| Self::error("plugin_host_failed", "原生插件实例不可用。"))?;
        Ok(host)
    }

    #[cfg(unix)]
    async fn finish_domain<T>(
        &self,
        slot: &mut Option<NativeHost>,
        result: NativeResult<T>,
    ) -> NativeResult<T> {
        let invalid = result
            .as_ref()
            .is_err_and(|error| error.code == "plugin_host_response_mismatch");
        if result.is_err()
            && slot.as_ref().is_some_and(|host| {
                host.is_failed() || invalid || self.stopping.load(Ordering::Acquire)
            })
            && let Some(mut host) = slot.take()
        {
            let cleanup = CleanupGuard::new(&self.cleanup_failed);
            let mut error = match result {
                Err(error) => error,
                Ok(_) => unreachable!("checked above"),
            };
            host.finish_failed(&mut error).await;
            cleanup.finish(Some(error.as_ref()));
            return Err(error);
        }
        result
    }

    pub(crate) async fn shutdown(&self) -> NativeResult<()> {
        self.stopping.store(true, Ordering::Release);
        self.stopped.notify_waiters();
        let _doi_operations = self.doi_operations.write().await;
        #[cfg(unix)]
        if let Some(mut host) = self.host.lock().await.take() {
            let cleanup = CleanupGuard::new(&self.cleanup_failed);
            let result = host.shutdown().await;
            cleanup.finish(result.as_ref().err().map(|error| error.as_ref()));
            result?;
        }
        if self.cleanup_failed.load(Ordering::Acquire) {
            let mut error = Self::error(
                "plugin_host_cleanup_blocked",
                "原生实例或输入未确认收尾；不能报告关闭成功。",
            );
            error.add_cleanup("先前原生实例或输入清理失败。");
            return Err(error);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;

mod semantic_index_runtime;
