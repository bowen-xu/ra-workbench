//! Existing bibliography edit/link/remove entrypoints over package-owned decisions.
use super::super::bibliography_transaction::{self, Phases, sql_failure};
use super::*;
use crate::data_interface::guarded_transaction;
use research_workbench_native_sdk::writing_bibliography::mutation::{
    Decision as MutationDecision, Input as MutationInput,
};
use research_workbench_native_sdk::writing_project::CitationBinding;
use rusqlite::{OptionalExtension, params};
use serde_json::Value;
impl Runtime<'_> {
    pub(super) fn update(
        &self,
        entry_id: &str,
        expected: &str,
        draft: &Value,
        outcome: &mut SaveOutcome,
    ) -> NativeResult<()> {
        let accepted = self.read(|_, reader| {
            reader.mutation(MutationInput::ValidateDraft {
                draft: draft.clone(),
            })
        })?;
        if !matches!(accepted, MutationDecision::Accepted) {
            return Err(self.mismatch());
        }
        self.operation.recover(outcome)?;
        let FileResult::Bibliography(entries) = self.operation.bibliography(
            self.sql,
            &Call::List {
                include_unused_automatic: true,
            },
            outcome,
        )?
        else {
            return Err(self.mismatch());
        };
        let MutationDecision::UpdateSelection { selected, patch } = self.read(|_, reader| {
            reader.mutation(MutationInput::SelectUpdate {
                entry_id: entry_id.into(),
                entries: entries.clone(),
                draft: draft.clone(),
                library_available: self.library_available(),
            })
        })?
        else {
            return Err(self.mismatch());
        };
        self.checked_write(
            outcome,
            |db, reader| {
                let (key, current, linked): (String, String, Option<String>) = self
                    .sql
                    .prepare(db, "project-bibliography-edit-row.v1.sql")
                    .map_err(original)?
                    .query_row(params![self.operation.manuscript_id, entry_id], |row| {
                        Ok((row.get(0)?, row.get(1)?, row.get(2)?))
                    })
                    .map_err(original)?;
                let MutationDecision::Text { text } = reader.mutation(MutationInput::Update {
                    key,
                    current,
                    linked,
                    expected: expected.into(),
                    selected: selected.clone(),
                    patch: patch.clone(),
                })?
                else {
                    return reader.mismatch();
                };
                Ok(text)
            },
            |db, text| {
                db.execute(
                    self.sql.get("project-bibliography-edit.v1.sql"),
                    params![self.operation.manuscript_id, entry_id, text],
                )
                .map_err(|cause| self.error(cause.into()))?;
                Ok(())
            },
        )?;
        self.publish(outcome)
    }
    pub(super) fn binding(&self, db: &Connection, source: &str) -> Read<Option<CitationBinding>> {
        self.sql
            .prepare(db, "project-bibliography-binding.v1.sql")
            .map_err(original)?
            .query_row(params![self.operation.manuscript_id, source], |row| {
                Ok(CitationBinding {
                    manuscript_id: row.get(0)?,
                    literature_item_id: row.get(1)?,
                    citation_key: row.get(2)?,
                    bibtex_entry: row.get(3)?,
                    state: row.get(4)?,
                    pinned_at: row.get(5)?,
                    updated_at: row.get(6)?,
                })
            })
            .optional()
            .map_err(original)
    }
    pub(super) fn link(
        &self,
        entry_id: &str,
        item_id: &str,
        outcome: &mut SaveOutcome,
    ) -> NativeResult<()> {
        let MutationDecision::Link { source } = self.read(|_, reader| {
            reader.mutation(MutationInput::SelectLink {
                entry_id: entry_id.into(),
                item_id: item_id.into(),
            })
        })?
        else {
            return Err(self.mismatch());
        };
        if let Some(source) = source {
            self.checked_write(
                outcome,
                |db, reader| {
                    let binding = self
                        .binding(db, &source)?
                        .ok_or_else(|| original(WritingError::CitationSourceNotFound))?;
                    let item = self.library_item(db, reader, item_id)?;
                    let AdoptionDecision::CanonicalEntry { text } =
                        reader.adoption(AdoptionInput::FromLibrary {
                            template: Some(binding.bibtex_entry),
                            key: binding.citation_key,
                            item,
                        })?
                    else {
                        return reader.mismatch();
                    };
                    Ok(text)
                },
                |db, text| {
                    db.execute(
                        self.sql.get("project-bibliography-update-binding.v1.sql"),
                        params![self.operation.manuscript_id, item_id, text],
                    )
                    .map_err(|cause| self.error(cause.into()))?;
                    Ok(())
                },
            )?;
            self.regenerate(false, outcome)?;
            return Ok(());
        }
        self.link_imported(entry_id, item_id, outcome)?;
        self.publish(outcome)
    }
    pub(super) fn link_imported(
        &self,
        entry_id: &str,
        item_id: &str,
        outcome: &mut SaveOutcome,
    ) -> NativeResult<()> {
        self.checked_write(
            outcome,
            |db, reader| {
                let item = self.library_item(db, reader, item_id)?;
                let (key, json): (String, String) = self
                    .sql
                    .prepare(db, "project-bibliography-linked-entry.v1.sql")
                    .map_err(original)?
                    .query_row(params![self.operation.manuscript_id, entry_id], |row| {
                        Ok((row.get(0)?, row.get(1)?))
                    })
                    .map_err(original)?;
                let metadata: Value = serde_json::from_str(&json).map_err(original)?;
                let AdoptionDecision::CanonicalEntry { text } =
                    reader.adoption(AdoptionInput::FromLibrary {
                        template: metadata["rawBibtex"].as_str().map(ToOwned::to_owned),
                        key,
                        item: item.clone(),
                    })?
                else {
                    return reader.mismatch();
                };
                Ok((item, text))
            },
            |db, (item, text)| {
                let snapshot =
                    serde_json::to_string(&item).map_err(|cause| self.error(cause.into()))?;
                db.execute(
                    self.sql.get("project-bibliography-link-entry.v1.sql"),
                    params![
                        self.operation.manuscript_id,
                        entry_id,
                        item_id,
                        snapshot,
                        text
                    ],
                )
                .map_err(|cause| self.error(cause.into()))?;
                Ok(())
            },
        )
    }
    pub(super) fn remove_unused(&self, outcome: &mut SaveOutcome) -> NativeResult<usize> {
        self.adopt(outcome)?;
        self.publish(outcome)?;
        self.publish(outcome)?;
        let used = self.retained_with_imports(false)?;
        let mut reader = Reader::default();
        let removed = loop {
            reader.begin();
            let attempt = {
                let mut guard = self
                    .operation
                    .store
                    .connection()
                    .map_err(|cause| self.error(cause.into()))?;
                guarded_transaction::run(&mut guard, |db| {
                    let attempt = bibliography_transaction::run(
                        db,
                        Phases {
                            begin: "bibliography_remove_begin",
                            commit: "bibliography_remove_commit",
                            rollback: "bibliography_remove_rollback",
                            unsettled: "writing_bibliography_remove_unsettled",
                        },
                        |tx| {
                            let keys = self
                                .strings(tx, "project-bibliography-keys.v1.sql")
                                .map_err(original)?;
                            let MutationDecision::Remove { keys } =
                                reader.mutation(MutationInput::RemoveKeys {
                                    keys,
                                    used: used.clone(),
                                })?
                            else {
                                return reader.mismatch();
                            };
                            let mut count = 0;
                            for key in keys {
                                count += tx
                                    .execute(
                                        self.sql.get("project-bibliography-remove-key.v1.sql"),
                                        params![self.operation.manuscript_id, key],
                                    )
                                    .map_err(|cause| {
                                        sql_failure(cause, "bibliography_remove_key")
                                    })?;
                            }
                            Ok(count)
                        },
                    );
                    if attempt.requires_recovery {
                        *outcome = SaveOutcome::Unknown;
                    } else if attempt.result.is_ok() {
                        *outcome = SaveOutcome::Committed {
                            item_id: self.operation.manuscript_id.clone(),
                        };
                    }
                    let unknown = attempt.requires_recovery;
                    (Ok::<_, std::convert::Infallible>(attempt), unknown)
                })
                .expect("transaction result carries its finite settlement")
            };
            match attempt.result {
                Ok(count) => break count,
                Err(Flow::Fail(cause)) => {
                    let mut error = self.failed(cause, &reader);
                    error.save_rollback_error = attempt.rollback_error.map(Box::new);
                    return Err(error);
                }
                Err(Flow::Need(need)) => self.resume(&mut reader, need)?,
            }
        };
        self.publish(outcome)?;
        Ok(removed)
    }
}

#[tauri::command]
pub(crate) async fn update_runtime_manuscript_bibliography_entry(
    store: tauri::State<'_, AppStore>,
    app: tauri::AppHandle,
    package_key: String,
    manuscript_id: String,
    entry_id: String,
    expected_bibtex: String,
    draft: Value,
) -> NativeResult<()> {
    use tauri::Manager;
    let service = Arc::clone(app.state::<Arc<NativePreviewService>>().inner());
    let catalog = Arc::clone(app.state::<Arc<RuntimePluginCatalog>>().inner());
    match super::super::operate_files(
        store.inner().clone(),
        service,
        catalog,
        package_key.clone(),
        manuscript_id,
        FileRequest::Bibliography {
            call: Call::Update {
                entry_id,
                expected: expected_bibtex,
                draft,
            },
        },
    )
    .await?
    {
        FileResult::BibliographyChanged => Ok(()),
        _ => Err(failure(
            "plugin_host_response_mismatch",
            "书目编辑结果种类不匹配。",
            &package_key,
        )),
    }
}
#[tauri::command]
pub(crate) async fn link_runtime_manuscript_bibliography_entry(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<NativePreviewService>>,
    catalog: tauri::State<'_, Arc<RuntimePluginCatalog>>,
    package_key: String,
    manuscript_id: String,
    entry_id: String,
    literature_item_id: String,
) -> NativeResult<()> {
    match super::super::operate_files(
        store.inner().clone(),
        Arc::clone(service.inner()),
        Arc::clone(catalog.inner()),
        package_key.clone(),
        manuscript_id,
        FileRequest::Bibliography {
            call: Call::Link {
                entry_id,
                item_id: literature_item_id,
            },
        },
    )
    .await?
    {
        FileResult::BibliographyChanged => Ok(()),
        _ => Err(failure(
            "plugin_host_response_mismatch",
            "书目关联结果种类不匹配。",
            &package_key,
        )),
    }
}
#[tauri::command]
pub(crate) async fn remove_unused_runtime_imported_bibliography_entries(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<NativePreviewService>>,
    catalog: tauri::State<'_, Arc<RuntimePluginCatalog>>,
    package_key: String,
    manuscript_id: String,
) -> NativeResult<usize> {
    match super::super::operate_files(
        store.inner().clone(),
        Arc::clone(service.inner()),
        Arc::clone(catalog.inner()),
        package_key.clone(),
        manuscript_id,
        FileRequest::Bibliography {
            call: Call::RemoveUnused,
        },
    )
    .await?
    {
        FileResult::RemovedBibliographyEntries(count) => Ok(count),
        _ => Err(failure(
            "plugin_host_response_mismatch",
            "书目清理结果种类不匹配。",
            &package_key,
        )),
    }
}
