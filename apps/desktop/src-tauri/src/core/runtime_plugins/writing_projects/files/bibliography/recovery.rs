//! Original backup, user-file preservation and first-projection ordering.
use super::*;
use research_workbench_native_sdk::writing_project::BibliographyStatus;
use rusqlite::params;
impl Runtime<'_> {
    fn registered(&self, db: &Connection, path: &str) -> Result<bool, WritingError> {
        self.sql
            .prepare(db, "project-bibliography-adoption-registered.v1.sql")?
            .query_row(params![self.operation.manuscript_id, path], |row| {
                row.get(0)
            })
            .map_err(Into::into)
    }
    pub(super) fn restore(&self, outcome: &mut SaveOutcome) -> NativeResult<BibliographyStatus> {
        let files = self.read(|db, _| {
            self.sql
                .prepare(db, "project-bibliography-projection-hashes.v1.sql")
                .map_err(original)?
                .query_map([&self.operation.manuscript_id], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })
                .map_err(original)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(original)
        })?;
        let access = self
            .operation
            .store
            .manuscript_files(&self.operation.manuscript_id);
        for (path, previous) in files {
            let (text, actual) = access.read(&path).map_err(|cause| self.file_error(cause))?;
            if actual == previous {
                continue;
            }
            let backup = format!("{}.backup-{}.txt", path, uuid::Uuid::new_v4());
            *outcome = SaveOutcome::Unknown;
            let hash = access
                .save(&backup, &text, None)
                .map_err(|cause| self.file_error(cause))?;
            self.write(outcome, |db| {
                self.operation
                    .resources
                    .index(
                        db,
                        &self.operation.manuscript_id,
                        &backup,
                        "other",
                        text.len(),
                        &hash,
                    )
                    .map_err(data_failure)?;
                db.execute(
                    self.sql
                        .get("project-bibliography-projection-record.v1.sql"),
                    params![self.operation.manuscript_id, path, actual],
                )
                .map_err(|cause| self.error(cause.into()))?;
                Ok(())
            })?;
        }
        self.publish(outcome)?;
        let registered = self.read(|db, reader| {
            let status = self.status_in(db, reader)?;
            self.registered(db, &status.relative_path).map_err(original)
        })?;
        let mut reference_is_file = None;
        let mut reference = None;
        let mut probes = Vec::new();
        loop {
            let PublicationDecision::LegacyRecovery { plan } =
                self.publication_decision(PublicationInput::LegacyRecovery {
                    registered,
                    reference_is_file,
                    reference_read: reference.is_some(),
                    probes: probes.clone(),
                })?
            else {
                return Err(self.mismatch());
            };
            match plan {
                LegacyRecoveryPlan::NeedIsFile { path } => {
                    reference_is_file = Some(
                        access
                            .is_file(&path)
                            .map_err(|cause| self.file_error(cause))?,
                    )
                }
                LegacyRecoveryPlan::NeedReferenceRead { path } => {
                    let (text, hash) =
                        access.read(&path).map_err(|cause| self.file_error(cause))?;
                    reference = Some((path, text, hash));
                }
                LegacyRecoveryPlan::NeedExists { path } => {
                    let exists = access
                        .exists(&path)
                        .map_err(|cause| self.file_error(cause))?;
                    probes.push((path, exists));
                }
                LegacyRecoveryPlan::Ready { force, user_path } => {
                    if let Some(user_path) = user_path {
                        let Some((path, text, _)) = reference else {
                            return Err(self.mismatch());
                        };
                        *outcome = SaveOutcome::Unknown;
                        let hash = access
                            .save(&user_path, &text, None)
                            .map_err(|cause| self.file_error(cause))?;
                        access
                            .delete(&path)
                            .map_err(|cause| self.file_error(cause))?;
                        self.write(outcome, |db| {
                            db.execute(
                                self.sql.get("project-file-delete.v1.sql"),
                                params![self.operation.manuscript_id, path],
                            )
                            .map_err(|cause| self.error(cause.into()))?;
                            self.operation
                                .resources
                                .index(
                                    db,
                                    &self.operation.manuscript_id,
                                    &user_path,
                                    "other",
                                    text.len(),
                                    &hash,
                                )
                                .map_err(data_failure)
                        })?;
                    }
                    return self.regenerate(force, outcome);
                }
            }
        }
    }
    pub(super) fn regenerate(
        &self,
        force: bool,
        outcome: &mut SaveOutcome,
    ) -> NativeResult<BibliographyStatus> {
        // Preserve restore's pre-adoption publication, then the original recovery
        // and Always publication before resolving linked entries.
        self.operation.recover(outcome)?;
        self.adopt(outcome)?;
        self.publish(outcome)?;
        let entries = self.entries()?;
        if self.library_available() {
            for entry in entries {
                let Some(item) = entry.library_item else {
                    continue;
                };
                if entry.id.starts_with("binding:") {
                    let source = item["id"]
                        .as_str()
                        .ok_or_else(|| self.error(WritingError::CitationSourceNotFound))?;
                    self.checked_write(
                        outcome,
                        |_, reader| {
                            let AdoptionDecision::CanonicalEntry { text } =
                                reader.adoption(AdoptionInput::FromLibrary {
                                    template: Some(entry.bibtex_entry.clone()),
                                    key: entry.citation_key.clone(),
                                    item: item.clone(),
                                })?
                            else {
                                return reader.mismatch();
                            };
                            Ok(text)
                        },
                        |db, text| {
                            db.execute(
                                self.sql.get("project-bibliography-update-binding.v1.sql"),
                                params![self.operation.manuscript_id, source, text],
                            )
                            .map_err(|cause| self.error(cause.into()))?;
                            Ok(())
                        },
                    )?;
                } else {
                    let source = item["id"]
                        .as_str()
                        .ok_or_else(|| self.error(WritingError::CitationSourceNotFound))?;
                    self.link_imported(&entry.id, source, outcome)?;
                    self.publish(outcome)?;
                }
            }
        }
        let context = self.publish_context(outcome)?;
        if let Some(status) = self.read(|db, reader| {
            if self.registered(db, &context.target).map_err(original)? {
                self.status_in(db, reader).map(Some)
            } else {
                Ok(None)
            }
        })? {
            return Ok(status);
        }
        let (bindings, previous) = self.read(|db, reader| {
            let bindings = self.bindings(db).map_err(original)?;
            let status = self.status_in(db, reader)?;
            Ok((bindings, status))
        })?;
        let access = self
            .operation
            .store
            .manuscript_files(&self.operation.manuscript_id);
        let original = if access
            .exists(&context.target)
            .map_err(|cause| self.file_error(cause))?
        {
            Some(
                access
                    .read(&context.target)
                    .map_err(|cause| self.file_error(cause))?,
            )
        } else {
            None
        };
        let PublicationDecision::Regeneration { plan } =
            self.publication_decision(PublicationInput::Regeneration {
                target: context.target.clone(),
                previous: previous.clone(),
                bindings_empty: bindings.is_empty(),
                supplemental_empty: context.entries.is_empty(),
                original: original.clone(),
                force,
            })?
        else {
            return Err(self.mismatch());
        };
        match plan {
            RegenerationPlan::Unchanged => return Ok(previous),
            RegenerationPlan::Conflict { expected } => {
                self.write(outcome, |db| {
                    self.record_status(db, &context.target, expected.as_deref(), "conflict")
                })?;
                return Err(self.error(WritingError::BibliographyConflict));
            }
            RegenerationPlan::Register => {}
        }
        self.write(outcome, |db| {
            db.execute(
                self.sql.get("project-bibliography-register-first.v1.sql"),
                params![
                    self.operation.manuscript_id,
                    context.target,
                    original
                        .as_ref()
                        .map(|(text, _)| text.as_str())
                        .unwrap_or_default(),
                    original
                        .as_ref()
                        .map(|(_, hash)| hash.as_str())
                        .unwrap_or_default()
                ],
            )
            .map_err(|cause| self.error(cause.into()))?;
            Ok(())
        })?;
        self.publish(outcome)?;
        self.read(|db, reader| self.status_in(db, reader))
    }
}

#[tauri::command]
pub(crate) async fn restore_runtime_manuscript_bibliography(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<NativePreviewService>>,
    catalog: tauri::State<'_, Arc<RuntimePluginCatalog>>,
    package_key: String,
    manuscript_id: String,
) -> NativeResult<BibliographyStatus> {
    match super::super::operate_files(
        store.inner().clone(),
        Arc::clone(service.inner()),
        Arc::clone(catalog.inner()),
        package_key.clone(),
        manuscript_id,
        FileRequest::Bibliography {
            call: Call::Restore,
        },
    )
    .await?
    {
        FileResult::BibliographyStatus(status) => Ok(status),
        _ => Err(failure(
            "plugin_host_response_mismatch",
            "写作书目恢复结果种类不匹配。",
            &package_key,
        )),
    }
}
