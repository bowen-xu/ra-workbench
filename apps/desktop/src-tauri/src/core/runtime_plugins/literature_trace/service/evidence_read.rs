//! Fixed read groups retain Core's connection; package SQL adds no write permission.
use super::*;
use crate::{
    data_interface::{DataError, SharedDatabase},
    runtime_plugins::literature_evidence::{
        self, EvidenceReadError, EvidenceReadRequest, EvidenceReadResult, EvidenceReadSql,
    },
};
impl LiteratureTraceService {
    pub(crate) async fn evidence_read(
        self: &Arc<Self>,
        database: SharedDatabase,
        key: String,
        request: EvidenceReadRequest,
    ) -> NativeResult<EvidenceReadResult> {
        let owner = Arc::clone(self);
        let selected = key.clone();
        let operation = uuid::Uuid::new_v4().to_string();
        tokio::spawn(async move {
            owner.available()?;
            let _lifetime = owner.doi_operations.read().await;
            owner.available()?;
            let catalog = Arc::clone(&owner.catalog);
            tokio::task::spawn_blocking(move || {
                let package = package::accept_evidence_read(&catalog, &selected)?;
                let sql = EvidenceReadSql::load(&catalog, &package)
                    .map_err(|cause| Self::error(cause.code, cause.message))?;
                let db = database.lock().map_err(owner_error)?;
                literature_evidence::execute(&sql, &db, request).map_err(EvidenceReadError::native)
            })
            .await
            .map_err(|_| Self::error("db_query_worker_failed", "证据查询工作线程未完成。"))?
        })
        .await
        .map_err(|_| Self::error("db_query_worker_failed", "证据查询操作未完成。"))
        .and_then(|result| result)
        .map_err(|mut error| {
            error.package_key = Some(key);
            error.operation_id = Some(operation);
            error
        })
    }
}
fn owner_error(cause: DataError) -> Box<NativeHostError> {
    let code = match cause {
        DataError::Sql(cause) => return EvidenceReadError::Database(cause).native(),
        DataError::Unavailable => "DATA_RUNTIME_UNAVAILABLE",
        DataError::Pair(_) => "DATA_RUNTIME_PAIR_INVALID",
        DataError::Log(_) => "DATA_RUNTIME_LOG",
        DataError::Admission(_) => "DATA_RUNTIME_ADMISSION",
    };
    LiteratureTraceService::error("storage_error", format!("证据数据拥有者未就绪：{code}"))
}
