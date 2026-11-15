//! Original binding precedence, partial write and response/read-back order.
use super::*;
use research_workbench_native_sdk::{
    writing_bibliography::mutation::{Decision as MutationDecision, Input as MutationInput},
    writing_project::CitationBinding,
};
use rusqlite::{OptionalExtension, params};
impl Runtime<'_> {
    pub(super) fn bind(
        &self,
        input: &BibliographyEntryInput,
        outcome: &mut SaveOutcome,
    ) -> NativeResult<CitationBinding> {
        self.adopt(outcome)?;
        self.publish(outcome)?;
        let binding = self.read(|db, reader| {
            self.project(db, reader)?;
            if !self.active_library_source(db, &input.literature_item_id)? {
                return reader.rejected(WritingError::CitationSourceNotFound);
            }
            if let Some(existing) = self.binding(db, &input.literature_item_id)? {
                let AdoptionDecision::CanonicalEntry { text } =
                    reader.adoption(AdoptionInput::NormalizeBinding {
                        entry: input.bibtex_entry.clone(),
                        key: existing.citation_key.clone(),
                    })?
                else {
                    return reader.mismatch();
                };
                *outcome = SaveOutcome::Unknown;
                return Ok(db
                    .execute(
                        self.sql.get("project-bibliography-update-binding.v1.sql"),
                        params![self.operation.manuscript_id, input.literature_item_id, text],
                    )
                    .map(|_| CitationBinding {
                        bibtex_entry: text,
                        ..existing
                    })
                    .map_err(|cause| self.error(cause.into())));
            }
            let imported = self.imported_key(db, &input.literature_item_id)?;
            let pinned = self
                .sql
                .prepare(db, "project-bibliography-pinned-key.v1.sql")
                .map_err(original)?
                .query_row(
                    params![self.operation.manuscript_id, input.literature_item_id],
                    |row| row.get(0),
                )
                .optional()
                .map_err(original)?;
            let mut occupied = self
                .strings(db, "project-bibliography-adoption-bound-keys.v1.sql")
                .map_err(original)?;
            occupied.extend(
                self.strings(db, "project-bibliography-keys.v1.sql")
                    .map_err(original)?,
            );
            let MutationDecision::Key { key } = reader.mutation(MutationInput::BindKey {
                requested: input.requested_key.clone(),
                imported,
                pinned,
                occupied,
            })?
            else {
                return reader.mismatch();
            };
            let AdoptionDecision::CanonicalEntry { text } =
                reader.adoption(AdoptionInput::NormalizeBinding {
                    entry: input.bibtex_entry.clone(),
                    key: key.clone(),
                })?
            else {
                return reader.mismatch();
            };
            *outcome = SaveOutcome::Unknown;
            Ok((|| {
                db.execute(
                    self.sql.get("project-bibliography-bind-insert.v1.sql"),
                    params![
                        self.operation.manuscript_id,
                        input.literature_item_id,
                        key,
                        text
                    ],
                )
                .map_err(|cause| self.error(cause.into()))?;
                // The insert is already committed. A read-back error must never
                // restart this capture or repeat the insert.
                self.binding(db, &input.literature_item_id)
                    .map_err(|flow| match flow {
                        Flow::Fail(cause) => self.failed(cause, reader),
                        Flow::Need(_) => self.mismatch(),
                    })?
                    .ok_or_else(|| self.error(WritingError::CitationSourceNotFound))
            })())
        })??;
        self.regenerate(false, outcome)?;
        Ok(binding)
    }
}
#[tauri::command]
pub(crate) async fn bind_runtime_manuscript_citation(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<NativePreviewService>>,
    catalog: tauri::State<'_, Arc<RuntimePluginCatalog>>,
    package_key: String,
    manuscript_id: String,
    entry: BibliographyEntryInput,
) -> NativeResult<CitationBinding> {
    match super::super::operate_files(
        store.inner().clone(),
        Arc::clone(service.inner()),
        Arc::clone(catalog.inner()),
        package_key.clone(),
        manuscript_id,
        FileRequest::Bibliography {
            call: Call::Bind { entry },
        },
    )
    .await?
    {
        FileResult::CitationBinding(binding) => Ok(binding),
        _ => Err(failure(
            "plugin_host_response_mismatch",
            "引用绑定结果种类不匹配。",
            &package_key,
        )),
    }
}
