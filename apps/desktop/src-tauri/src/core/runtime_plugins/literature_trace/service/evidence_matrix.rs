//! Original matrix operations pause only before BEGIN and recheck the owned prefix.
use super::*;
use crate::{
    data_interface::{SharedDatabase, guarded_transaction},
    runtime_plugins::literature_matrix::{
        self, MatrixError, MatrixPolicy, MatrixRequest, MatrixResult, MatrixSql,
    },
};
use research_workbench_native_sdk::{
    catalog::{CatalogInput, CatalogOutput},
    evidence_matrix::{MatrixValidationDecisionV1, MatrixValidationInputV1},
};

#[derive(Default)]
struct Policy {
    observations: Vec<Observation>,
    cursor: usize,
}
struct Observation {
    input: MatrixValidationInputV1,
    decision: MatrixValidationDecisionV1,
    origin: Option<(String, String)>,
}
impl MatrixPolicy for Policy {
    fn validate(&mut self, input: MatrixValidationInputV1) -> Result<(), MatrixError> {
        let Some(Observation {
            input: prior,
            decision,
            origin,
        }) = self.observations.get(self.cursor)
        else {
            return Err(MatrixError::Need(Box::new(input)));
        };
        self.cursor += 1;
        let fail = |code, message: &str| {
            let mut error = LiteratureTraceService::error(code, message);
            if let Some((instance, request)) = origin {
                error.instance_id = Some(instance.clone());
                error.request_id = Some(request.clone());
            }
            MatrixError::Technical(error)
        };
        if prior != &input {
            return Err(fail(
                "plugin_host_source_changed",
                "矩阵校验来源已变化，请重新载入。",
            ));
        }
        if !input.matches_decision(decision) {
            return Err(fail(
                "plugin_host_response_mismatch",
                "矩阵回复与当前校验阶段不匹配。",
            ));
        }
        match decision.rejection {
            None => Ok(()),
            Some(reason) => Err(fail("invalid_input", reason.message())),
        }
    }
}
impl LiteratureTraceService {
    pub(crate) async fn evidence_matrix(
        self: &Arc<Self>,
        database: SharedDatabase,
        key: String,
        request: MatrixRequest,
    ) -> NativeResult<MatrixResult> {
        let owner = Arc::clone(self);
        let selected = key.clone();
        let operation = uuid::Uuid::new_v4().to_string();
        tokio::spawn(async move { owner.matrix_owned(database, &selected, request).await })
            .await
            .map_err(|_| {
                Self::error(
                    "matrix_operation_unsettled",
                    "矩阵操作结果未确认，禁止自动重试；请重开后核对。",
                )
            })
            .and_then(|result| result)
            .map_err(|mut error| {
                error.package_key = Some(key);
                error.operation_id = Some(operation);
                error
            })
    }
    async fn matrix_owned(
        &self,
        database: SharedDatabase,
        key: &str,
        request: MatrixRequest,
    ) -> NativeResult<MatrixResult> {
        self.available()?;
        let _lifetime = self.doi_operations.read().await;
        let catalog = Arc::clone(&self.catalog);
        let selected = key.to_owned();
        let sql = tokio::task::spawn_blocking(move || {
            let package = package::accept_evidence_matrix(&catalog, &selected)?;
            MatrixSql::load(&catalog, &package)
                .map_err(|cause| Self::error(cause.code, cause.message))
        })
        .await
        .map_err(|_| Self::error("plugin_host_accept_failed", "矩阵资源核对任务未完成。"))??;
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
            let mut slot = self.host.lock().await;
            self.available()?;
            let result = match self.ensure_host(&mut slot, key).await {
                Ok(host) => self.run_matrix(host, database, sql, request).await,
                Err(error) => Err(error),
            };
            self.finish_domain(&mut slot, result).await
        }
    }
    #[cfg(unix)]
    async fn run_matrix(
        &self,
        host: &mut NativeHost,
        database: SharedDatabase,
        mut sql: MatrixSql,
        request: MatrixRequest,
    ) -> NativeResult<MatrixResult> {
        let before = host.request_identity();
        let mut policy = Policy::default();
        // One header plus at most twenty statements, then the original writer.
        for _ in 0..22 {
            self.available()?;
            let source = Arc::clone(&database);
            let draft = request.clone();
            let (next, resources, result) = tokio::task::spawn_blocking(move || {
                policy.cursor = 0;
                let result = (|| {
                    let mut db = source.lock().map_err(owner_error)?;
                    guarded_transaction::run_mut(&mut db, |db| {
                        let result = literature_matrix::execute(&sql, db, &mut policy, draft);
                        let unknown = !db.is_autocommit()
                            || result.as_ref().is_err_and(MatrixError::needs_recovery);
                        (result, unknown)
                    })
                })();
                (policy, sql, result)
            })
            .await
            .map_err(|_| {
                host.error(
                    "matrix_operation_unsettled",
                    "矩阵执行结果未确认，禁止自动重试；请重开后核对。",
                )
            })?;
            policy = next;
            match result {
                Ok(value) => return Ok(value),
                Err(MatrixError::Need(input)) => {
                    let stopped = self.stopped.notified();
                    tokio::pin!(stopped);
                    stopped.as_mut().enable();
                    self.available()?;
                    let output = tokio::select! {
                        biased;
                        _ = &mut stopped => Err(host.error("plugin_host_stopping", "应用正在关闭矩阵校验。")),
                        result = host.catalog_call(CatalogInput::ValidateEvidenceMatrix { input: input.clone() }, Duration::from_secs(60)) => result,
                    }?;
                    let CatalogOutput::EvidenceMatrixValidation { decision } = output else {
                        return Err(
                            host.error("plugin_host_response_mismatch", "矩阵回复种类不匹配。")
                        );
                    };
                    if !input.matches_decision(&decision) {
                        return Err(host.error(
                            "plugin_host_response_mismatch",
                            "矩阵回复与当前校验阶段不匹配。",
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
            "矩阵校验超过原有有限阶段。",
        ))
    }
}
fn owner_error(cause: crate::data_interface::DataError) -> MatrixError {
    use crate::data_interface::DataError;
    match cause {
        DataError::Sql(cause) => MatrixError::Database(cause),
        _ => MatrixError::Technical(LiteratureTraceService::error(
            "storage_error",
            "DATA_RUNTIME_UNAVAILABLE：矩阵数据拥有者未就绪，请重开后核对。",
        )),
    }
}
