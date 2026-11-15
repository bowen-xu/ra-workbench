//! Original ordered binding updates, followed by ordinary regeneration.
use super::*;
use research_workbench_native_sdk::writing_project::{BibliographyStatus, CitationBinding};
use rusqlite::{OptionalExtension, params};

impl Runtime<'_> {
    pub(super) fn sync(
        &self,
        entries: &[BibliographyEntryInput],
        outcome: &mut SaveOutcome,
    ) -> NativeResult<BibliographyStatus> {
        self.read(|db, reader| self.project(db, reader).map(|_| ()))?;
        for input in entries {
            self.read(|db, reader| {
                let binding: Option<CitationBinding> = self
                    .sql
                    .prepare(db, "project-bibliography-binding.v1.sql")
                    .map_err(original)?
                    .query_row(
                        params![self.operation.manuscript_id, input.literature_item_id],
                        |row| {
                            Ok(CitationBinding {
                                manuscript_id: row.get(0)?,
                                literature_item_id: row.get(1)?,
                                citation_key: row.get(2)?,
                                bibtex_entry: row.get(3)?,
                                state: row.get(4)?,
                                pinned_at: row.get(5)?,
                                updated_at: row.get(6)?,
                            })
                        },
                    )
                    .optional()
                    .map_err(original)?;
                let Some(binding) = binding else {
                    return Ok(Ok(()));
                };
                if !self.active_library_source(db, &input.literature_item_id)? {
                    return Ok(Ok(()));
                }
                let AdoptionDecision::CanonicalEntry { text } =
                    reader.adoption(AdoptionInput::NormalizeBinding {
                        entry: input.bibtex_entry.clone(),
                        key: binding.citation_key,
                    })?
                else {
                    return reader.mismatch();
                };
                *outcome = SaveOutcome::Unknown;
                Ok(db
                    .execute(
                        self.sql.get("project-bibliography-update-binding.v1.sql"),
                        params![self.operation.manuscript_id, input.literature_item_id, text],
                    )
                    .map(|_| ())
                    .map_err(|cause| self.error(cause.into())))
            })??;
        }
        self.regenerate(false, outcome)
    }
}

#[tauri::command]
pub(crate) async fn sync_runtime_manuscript_bibliography(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<NativePreviewService>>,
    catalog: tauri::State<'_, Arc<RuntimePluginCatalog>>,
    package_key: String,
    manuscript_id: String,
    entries: Vec<BibliographyEntryInput>,
) -> NativeResult<BibliographyStatus> {
    match super::super::operate_files(
        store.inner().clone(),
        Arc::clone(service.inner()),
        Arc::clone(catalog.inner()),
        package_key.clone(),
        manuscript_id,
        FileRequest::Bibliography {
            call: Call::Sync { entries },
        },
    )
    .await?
    {
        FileResult::BibliographyStatus(status) => Ok(status),
        _ => Err(failure(
            "plugin_host_response_mismatch",
            "写作书目同步结果种类不匹配。",
            &package_key,
        )),
    }
}
