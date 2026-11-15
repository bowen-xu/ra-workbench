//! Original Deferred read snapshot, released before each owned Native wait.
use super::super::bibliography_transaction::{self, Phases};
use super::*;
use crate::data_interface::guarded_transaction;
use research_workbench_native_sdk::{
    catalog::bibliography::CitationSearchKeys,
    writing_bibliography::{
        CitationCandidate,
        mutation::{Decision as MutationDecision, Input as MutationInput},
    },
};
impl Runtime<'_> {
    pub(super) fn search(&self, query: &str, scope: &str) -> NativeResult<Vec<CitationCandidate>> {
        let mut reader = Reader::default();
        loop {
            reader.begin();
            let attempt = {
                let mut guard = self
                    .operation
                    .store
                    .connection()
                    .map_err(|cause| self.error(cause.into()))?;
                guarded_transaction::run(&mut guard, |db| {
                    let attempt = bibliography_transaction::read(
                        db,
                        Phases {
                            begin: "citation_search_begin",
                            commit: "citation_search_unused_commit",
                            rollback: "citation_search_rollback",
                            unsettled: "writing_citation_search_unsettled",
                        },
                        |tx| {
                            let (manuscript, _) = self.project(tx, &mut reader)?;
                            let MutationDecision::SearchScope { project_only } =
                                reader.mutation(MutationInput::PrepareSearch {
                                    scope: scope.into(),
                                    library_available: self.library_available(),
                                })?
                            else {
                                return reader.mismatch();
                            };
                            let keys = self
                                .sql
                                .prepare(tx, "project-citation-search-keys.v1.sql")
                                .map_err(original)?
                                .query_map([&self.operation.manuscript_id], |row| {
                                    Ok(CitationSearchKeys {
                                        literature_item_id: row.get(0)?,
                                        bound_key: row.get(1)?,
                                        first_imported_key: row.get(2)?,
                                        imported_keys_search_text: row.get(3)?,
                                    })
                                })
                                .map_err(original)?
                                .collect::<rusqlite::Result<Vec<_>>>()
                                .map_err(original)?;
                            let keys = serde_json::to_string(&keys).map_err(|cause| {
                                original(rusqlite::Error::ToSqlConversionFailure(Box::new(cause)))
                            })?;
                            let hits = self.library_search(
                                tx,
                                &mut reader,
                                &keys,
                                project_only,
                                &manuscript.project_id,
                                query,
                            )?;
                            match reader.mutation(MutationInput::ProjectSearch { hits })? {
                                MutationDecision::Candidates { items } => Ok(items),
                                _ => reader.mismatch(),
                            }
                        },
                    );
                    let unknown = attempt.requires_recovery;
                    (Ok::<_, std::convert::Infallible>(attempt), unknown)
                })
                .expect("read attempt carries its complete rollback result")
            };
            match attempt.result {
                Ok(items) => return Ok(items),
                Err(Flow::Fail(cause)) => {
                    let mut error = self.failed(cause, &reader);
                    error.save_rollback_error = attempt.rollback_error.map(Box::new);
                    return Err(error);
                }
                Err(Flow::Need(need)) => self.resume(&mut reader, need)?,
            }
        }
    }
}
#[tauri::command]
pub(crate) async fn search_runtime_manuscript_citations(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<NativePreviewService>>,
    catalog: tauri::State<'_, Arc<RuntimePluginCatalog>>,
    package_key: String,
    manuscript_id: String,
    query: String,
    scope: String,
) -> NativeResult<Vec<CitationCandidate>> {
    match super::super::operate_files(
        store.inner().clone(),
        Arc::clone(service.inner()),
        Arc::clone(catalog.inner()),
        package_key.clone(),
        manuscript_id,
        FileRequest::Bibliography {
            call: Call::Search { query, scope },
        },
    )
    .await?
    {
        FileResult::CitationCandidates(items) => Ok(items),
        _ => Err(failure(
            "plugin_host_response_mismatch",
            "引用候选结果种类不匹配。",
            &package_key,
        )),
    }
}
