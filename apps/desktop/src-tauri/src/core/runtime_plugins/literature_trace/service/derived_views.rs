//! Recheck the original ordered observations after each confirmed suspension.
use super::*;
use crate::{
    data_interface::{SharedDatabase, guarded_transaction},
    runtime_plugins::literature_derived_view::{
        self, DerivedError, DerivedPolicy, DerivedRequest, DerivedResult, DerivedSql,
    },
};
use research_workbench_native_sdk::{
    catalog::{CatalogInput, CatalogOutput},
    derived_view::manual::{DerivedViewDecisionV1, DerivedViewInputV1},
};

#[derive(Default)]
struct Policy {
    observations: Vec<Observation>,
    cursor: usize,
}
struct Observation {
    input: DerivedViewInputV1,
    decision: DerivedViewDecisionV1,
    origin: Option<(String, String)>,
}
impl Policy {
    fn decide(&mut self, input: DerivedViewInputV1) -> Result<DerivedViewDecisionV1, DerivedError> {
        let Some(Observation {
            input: prior,
            decision,
            origin,
        }) = self.observations.get(self.cursor)
        else {
            return Err(DerivedError::Need(Box::new(input)));
        };
        self.cursor += 1;
        let fail = |code, message: &str| {
            let mut error = LiteratureTraceService::error(code, message);
            if let Some((instance, request)) = origin {
                error.instance_id = Some(instance.clone());
                error.request_id = Some(request.clone());
            }
            DerivedError::Technical(error)
        };
        if prior != &input {
            return Err(fail(
                "plugin_host_source_changed",
                "派生视图校验来源已变化，请重新载入。",
            ));
        }
        if !input.matches_decision(decision) {
            return Err(fail(
                "plugin_host_response_mismatch",
                "派生视图回复与当前阶段不匹配。",
            ));
        }
        Ok(decision.clone())
    }
}
impl DerivedPolicy for Policy {
    fn validate(
        &mut self,
        input: research_workbench_native_sdk::derived_view::DerivedValidationInputV1,
    ) -> Result<(), DerivedError> {
        match self.decide(DerivedViewInputV1::Validate {
            input: Box::new(input),
        })? {
            DerivedViewDecisionV1::Validated { rejection: None } => Ok(()),
            DerivedViewDecisionV1::Validated {
                rejection: Some(reason),
            } => Err(DerivedError::InvalidInput(reason.message().into())),
            _ => unreachable!("decision family checked against input"),
        }
    }
    fn provenance(&mut self, current: String, review: String) -> Result<String, DerivedError> {
        match self.decide(DerivedViewInputV1::Provenance { current, review })? {
            DerivedViewDecisionV1::Provenance { value } => Ok(value),
            _ => unreachable!("decision family checked against input"),
        }
    }
}
impl LiteratureTraceService {
    pub(crate) async fn derived_views(
        self: &Arc<Self>,
        database: SharedDatabase,
        key: String,
        request: DerivedRequest,
    ) -> NativeResult<DerivedResult> {
        let owner = Arc::clone(self);
        let selected = key.clone();
        let operation = uuid::Uuid::new_v4().to_string();
        tokio::spawn(async move { owner.derived_owned(database, &selected, request).await })
            .await
            .map_err(|_| {
                Self::error(
                    "derived_operation_unsettled",
                    "派生视图操作结果未确认，禁止自动重试；请重开后核对。",
                )
            })
            .and_then(|result| result)
            .map_err(|mut error| {
                error.package_key = Some(key);
                error.operation_id = Some(operation);
                error
            })
    }
    async fn derived_owned(
        &self,
        database: SharedDatabase,
        key: &str,
        request: DerivedRequest,
    ) -> NativeResult<DerivedResult> {
        self.available()?;
        let _lifetime = self.doi_operations.read().await;
        let catalog = Arc::clone(&self.catalog);
        let selected = key.to_owned();
        let sql = tokio::task::spawn_blocking(move || {
            let package = package::accept_derived_views(&catalog, &selected)?;
            DerivedSql::load(&catalog, &package)
                .map_err(|cause| Self::error(cause.code, cause.message))
        })
        .await
        .map_err(|_| Self::error("plugin_host_accept_failed", "派生视图资源核对任务未完成。"))??;
        #[cfg(not(unix))]
        {
            let _ = (database, request, sql);
            Err(Self::error(
                "plugin_host_platform_unsupported",
                "此平台尚未提供文献宿主。",
            ))
        }
        #[cfg(unix)]
        {
            if matches!(request, DerivedRequest::List { .. }) {
                return tokio::task::spawn_blocking(move || {
                    let mut db = database.lock().map_err(owner_error)?;
                    guarded_transaction::run_mut(&mut db, |db| {
                        let result = literature_derived_view::execute(
                            &sql,
                            db,
                            &mut Policy::default(),
                            request,
                        );
                        (result, !db.is_autocommit())
                    })
                })
                .await
                .map_err(|_| Self::error("derived_view_read_failed", "派生视图读取任务未完成。"))?
                .map_err(DerivedError::native);
            }
            let mut slot = self.host.lock().await;
            self.available()?;
            let result = match self.ensure_host(&mut slot, key).await {
                Ok(host) => self.run_derived(host, database, sql, request).await,
                Err(error) => Err(error),
            };
            self.finish_domain(&mut slot, result).await
        }
    }
    #[cfg(unix)]
    async fn run_derived(
        &self,
        host: &mut NativeHost,
        database: SharedDatabase,
        mut sql: DerivedSql,
        request: DerivedRequest,
    ) -> NativeResult<DerivedResult> {
        let before = host.request_identity();
        let mut policy = Policy::default();
        // Header, up to 100 input/coverage pairs, 100 items and provenance, then write.
        for _ in 0..303 {
            self.available()?;
            let source = Arc::clone(&database);
            let draft = request.clone();
            let (next, resources, result) = tokio::task::spawn_blocking(move || {
                policy.cursor = 0;
                let result = (|| {
                    let mut db = source.lock().map_err(owner_error)?;
                    guarded_transaction::run_mut(&mut db, |db| {
                        let result = literature_derived_view::execute(&sql, db, &mut policy, draft);
                        let unknown = !db.is_autocommit()
                            || result.as_ref().is_err_and(DerivedError::needs_recovery);
                        (result, unknown)
                    })
                })();
                (policy, sql, result)
            })
            .await
            .map_err(|_| {
                host.error(
                    "derived_operation_unsettled",
                    "派生视图执行结果未确认，禁止自动重试；请重开后核对。",
                )
            })?;
            policy = next;
            match result {
                Ok(value) => return Ok(value),
                Err(DerivedError::Need(input)) => {
                    let stopped = self.stopped.notified();
                    tokio::pin!(stopped);
                    stopped.as_mut().enable();
                    self.available()?;
                    let output = tokio::select! {
                        biased;
                        _ = &mut stopped => Err(host.error("plugin_host_stopping", "应用正在关闭派生视图校验。")),
                        result = host.catalog_call(CatalogInput::DecideDerivedView { input: input.clone() }, Duration::from_secs(60)) => result,
                    }?;
                    let CatalogOutput::DerivedViewDecision { decision } = output else {
                        return Err(
                            host.error("plugin_host_response_mismatch", "派生视图回复种类不匹配。")
                        );
                    };
                    if !input.matches_decision(&decision) {
                        return Err(host.error(
                            "plugin_host_response_mismatch",
                            "派生视图回复与当前校验阶段不匹配。",
                        ));
                    }
                    policy.observations.push(Observation {
                        input: *input,
                        decision,
                        origin: host.request_identity(),
                    });
                }
                Err(cause) => {
                    let mut error = cause.native();
                    if error.instance_id.is_none()
                        && host.request_identity() != before
                        && let Some((instance, request)) = host.request_identity()
                    {
                        error.instance_id = Some(instance);
                        error.request_id = Some(request);
                    }
                    return Err(error);
                }
            }
            sql = resources;
        }
        Err(host.error(
            "plugin_host_response_mismatch",
            "派生视图校验超过原有有限阶段。",
        ))
    }
}
fn owner_error(cause: crate::data_interface::DataError) -> DerivedError {
    use crate::data_interface::DataError;
    match cause {
        DataError::Sql(cause) => DerivedError::Database(cause),
        _ => DerivedError::Technical(LiteratureTraceService::error(
            "storage_error",
            "DATA_RUNTIME_UNAVAILABLE：派生视图数据拥有者未就绪，请重开后核对。",
        )),
    }
}
