//! A Core-owned save survives its caller stopping the wait. The existing host
//! mutex keeps shutdown behind the original SQL transaction and result capture.
use super::*;
use crate::data_interface::SharedDatabase;
use crate::runtime_plugins::literature_catalog::{
    data::CatalogResources,
    save::{self, SaveOutcome, SaveResources},
};
use research_workbench_native_sdk::{
    catalog::{
        CatalogInput, CatalogItem, CatalogOutput, CatalogProjectionInput, CatalogProjectionOutcome,
        CatalogRequest,
    },
    catalog_save::{CatalogDraft, CatalogSaveInput, CatalogSaveOutcome, PreparedDoi},
};

mod errors;

#[cfg(unix)]
struct SaveContext {
    database: SharedDatabase,
    draft: CatalogDraft,
    writes: SaveResources,
    reads: CatalogResources,
}

impl LiteratureTraceService {
    pub(crate) async fn save_catalog(
        self: &Arc<Self>,
        database: SharedDatabase,
        key: String,
        draft: CatalogDraft,
    ) -> NativeResult<CatalogItem> {
        let owner = Arc::clone(self);
        let selected = key.clone();
        // Dropping this JoinHandle does not abort the Core task. Once it owns
        // the host mutex, shutdown waits for its accepted SQL work to settle.
        let result = tokio::spawn(async move { owner.save_owned(database, selected, draft).await })
            .await
            .map_err(|_| errors::unknown("Core题录保存任务未能结算。").package(&key))?;
        result.map_err(|mut error| {
            error.package_key = Some(key);
            if error.save_outcome.is_none() {
                error.save_outcome = Some(SaveOutcome::NotStarted);
            }
            error
        })
    }

    async fn save_owned(
        &self,
        database: SharedDatabase,
        key: String,
        draft: CatalogDraft,
    ) -> NativeResult<CatalogItem> {
        self.available().map_err(|error| error.package(&key))?;
        let catalog = Arc::clone(&self.catalog);
        let selected = key.clone();
        let (writes, reads) = tokio::task::spawn_blocking(move || {
            let package = package::accept_catalog_save(&catalog, &selected)?;
            let writes = SaveResources::load(&catalog, &package)
                .map_err(|error| Self::error(error.code, error.message))?;
            let reads = CatalogResources::load(&catalog, &package)
                .map_err(|error| Self::error(error.code, error.message))?;
            Ok::<_, Box<NativeHostError>>((writes, reads))
        })
        .await
        .map_err(|_| Self::error("plugin_host_accept_failed", "题录保存资源核对任务未完成。"))??;
        #[cfg(not(unix))]
        {
            let _ = (database, draft, writes, reads);
            Err(Self::error(
                "plugin_host_platform_unsupported",
                "此平台尚未提供文献宿主。",
            )
            .package(&key))
        }
        #[cfg(unix)]
        {
            let mut slot = self.host.try_lock().map_err(|_| {
                Self::error("plugin_host_busy", "文献原生实例已有在途请求。").package(&key)
            })?;
            let stopped = self.stopped.notified();
            tokio::pin!(stopped);
            stopped.as_mut().enable();
            self.available().map_err(|error| error.package(&key))?;
            let mut result = self
                .save_locked(
                    &mut slot,
                    &key,
                    SaveContext {
                        database,
                        draft,
                        writes,
                        reads,
                    },
                    &mut stopped,
                )
                .await;
            if let Err(error) = &mut result {
                error.package_key = Some(key);
                if error.save_outcome.is_none() {
                    error.save_outcome = Some(SaveOutcome::NotStarted);
                }
            }
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
    }

    #[cfg(unix)]
    async fn save_locked(
        &self,
        slot: &mut Option<NativeHost>,
        key: &str,
        context: SaveContext,
        stopped: &mut std::pin::Pin<&mut tokio::sync::futures::Notified<'_>>,
    ) -> NativeResult<CatalogItem> {
        let SaveContext {
            database,
            draft,
            writes,
            reads,
        } = context;
        if writes.package_key() != key || reads.package_key() != key {
            return Err(Self::error(
                "plugin_host_response_mismatch",
                "保存资源不属于本次文献包。",
            ));
        }
        let host = self.ensure_host(slot, key).await?;
        let probe_db = Arc::clone(&database);
        tokio::task::spawn_blocking(move || {
            let mut guard = probe_db.lock().map_err(errors::owner_failure)?;
            save::probe_begin(&mut guard).map_err(errors::storage)
        })
        .await
        .map_err(|_| errors::bind(host, errors::unknown("题录事务预检任务未完成。")))?
        .map_err(|error| errors::bind(host, error))?;
        let display =
            crate::doi::clean_doi(&draft.doi).unwrap_or_else(|| draft.doi.trim().to_owned());
        let expected_id = draft.id.clone();
        let input = CatalogSaveInput {
            doi: PreparedDoi {
                normalized: crate::doi::normalized_doi(&display),
                display,
            },
            draft,
        };
        let prepared = tokio::select! {
            biased;
            _ = &mut *stopped => Err(Self::error("plugin_host_stopping", "应用正在关闭题录保存。")),
            result = host.catalog_call(CatalogInput::PrepareSave { input: Box::new(input) }, Duration::from_secs(60)) => result,
        }?;
        let CatalogOutput::SavePlan { outcome } = prepared else {
            return Err(host.error("plugin_host_response_mismatch", "题录保存准备阶段不匹配。"));
        };
        let plan = match outcome {
            CatalogSaveOutcome::Planned { plan } => plan,
            CatalogSaveOutcome::Rejected { reason } => return Err(errors::rejected(host, reason)),
        };
        if plan.item.id != expected_id {
            return Err(host.error(
                "plugin_host_response_mismatch",
                "保存计划替换了本次目标身份。",
            ));
        }
        self.available()?;
        // No stop select covers accepted SQL. This Core task retains the host
        // mutex until the original transaction and post-commit reads settle.
        let committed = tokio::task::spawn_blocking(move || {
            let mut guard = database.lock().map_err(errors::owner_failure)?;
            save::persist(&mut guard, &writes, &reads, &plan).map_err(errors::storage)
        })
        .await
        .map_err(|_| {
            errors::bind(
                host,
                errors::unknown("题录写入任务未能结算；不得自动重放。"),
            )
        })?
        .map_err(|error| errors::bind(host, error))?;
        let outcome = SaveOutcome::Committed {
            item_id: committed.item_id.clone(),
        };
        let projection = tokio::select! {
            biased;
            _ = &mut *stopped => Err(Self::error("plugin_host_stopping", "题录已提交，关闭期间不再等待完整结果。")),
            result = host.catalog_call(CatalogInput::Project { input: CatalogProjectionInput {
                request: CatalogRequest::Get { id: committed.item_id.clone() }, items: committed.items,
            }}, Duration::from_secs(60)) => result,
        }.map_err(|mut error| { error.save_outcome = Some(outcome.clone()); error })?;
        let result = match projection {
            CatalogOutput::Projection {
                outcome: CatalogProjectionOutcome::Success { mut items },
            } if items.len() == 1 && items[0].id == committed.item_id => Ok(items.remove(0)),
            CatalogOutput::Projection {
                outcome: CatalogProjectionOutcome::NotFound,
            } => Err(host.error("not_found", "找不到文献或附件")),
            _ => Err(host.error(
                "plugin_host_response_mismatch",
                "题录保存结果与本次身份不匹配。",
            )),
        };
        result.map_err(|mut error| {
            error.save_outcome = Some(outcome);
            error
        })
    }
}

#[cfg(test)]
mod tests;
