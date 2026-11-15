use super::*;
use rusqlite::params;
pub(super) struct Context {
    pub(super) target: String,
    pub(super) entries: Vec<String>,
    support: String,
}
impl Runtime<'_> {
    fn context(&self) -> NativeResult<Context> {
        let (files, root, registered) = self.read(|db, reader| {
            let registered = self
                .strings(db, "project-bibliography-registered.v1.sql")
                .map_err(original)?;
            let rows = self
                .operation
                .resources
                .files(db, &self.operation.manuscript_id)
                .map_err(|cause| {
                    Flow::Fail(Failure {
                        error: WritingError::Storage(crate::storage::StorageError::DataInterface(
                            crate::data_interface::DataError::Unavailable,
                        )),
                        cause: FailureCause::SourceData(cause),
                    })
                })?;
            let (project, manuscript) = self.project(db, reader)?;
            let decision = reader.call(Input::ListFiles {
                manuscript_id: self.operation.manuscript_id.clone(),
                manuscript,
                files: Some(rows),
            })?;
            let Decision::Files { files } = decision.as_ref() else {
                return reader.mismatch();
            };
            Ok((files.clone(), project.root_path, registered))
        })?;
        let PublicationDecision::SourcePaths { paths } =
            self.publication_decision(PublicationInput::SourcePaths {
                files,
                root_path: root,
            })?
        else {
            return Err(self.mismatch());
        };
        let mut declared = Vec::new();
        for path in paths {
            let (source, _) = self
                .operation
                .store
                .manuscript_files(&self.operation.manuscript_id)
                .read(&path)
                .map_err(|cause| self.file_error(cause))?;
            let PublicationDecision::Declarations { declared: next } =
                self.publication_decision(PublicationInput::Declarations { source, declared })?
            else {
                return Err(self.mismatch());
            };
            declared = next;
        }
        let PublicationDecision::Target { target, declared } =
            self.publication_decision(PublicationInput::Target {
                declared,
                registered: registered.clone(),
            })?
        else {
            return Err(self.mismatch());
        };
        let (entries, support) = self.read(|db, reader| {
            let rows = self
                .sql
                .prepare(db, "project-bibliography-supplemental-entries.v1.sql")
                .map_err(original)?
                .query_map([&self.operation.manuscript_id], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })
                .map_err(original)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(original)?;
            let mut entries = Vec::with_capacity(rows.len());
            for (id, bibtex_entry) in rows {
                let member_paths = self
                    .sql
                    .prepare(db, "project-bibliography-supplemental-members.v1.sql")
                    .map_err(original)?
                    .query_map(params![self.operation.manuscript_id, id], |row| row.get(0))
                    .map_err(original)?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(original)?;
                entries.push(SupplementalEntry {
                    bibtex_entry,
                    member_paths,
                });
            }
            let PublicationDecision::Supplemental { projection } =
                reader.publication(PublicationInput::Supplemental {
                    declared: declared.clone(),
                    registered: registered.clone(),
                    entries,
                })?
            else {
                return reader.mismatch();
            };
            let mut support = Vec::new();
            for path in projection.support_paths {
                support.push(
                    self.sql
                        .prepare(db, "project-bibliography-supplemental-support.v1.sql")
                        .map_err(original)?
                        .query_row(params![self.operation.manuscript_id, path], |row| {
                            row.get::<_, String>(0)
                        })
                        .map_err(original)?,
                );
            }
            Ok((projection.entries, support.join("\n")))
        })?;
        Ok(Context {
            target,
            entries,
            support,
        })
    }
    pub(super) fn publish(&self, outcome: &mut SaveOutcome) -> NativeResult<()> {
        self.publish_context(outcome).map(|_| ())
    }
    pub(super) fn publish_context(&self, outcome: &mut SaveOutcome) -> NativeResult<Context> {
        let proofs = self.operation.store.publication_proofs();
        proofs.invalidate(&self.operation.manuscript_id);
        let before = self.fingerprint()?;
        let retained = self.retained()?;
        let files = self.read(|db, _| {
            self.sql
                .prepare(db, "project-bibliography-projection-files.v1.sql")
                .map_err(original)?
                .query_map([&self.operation.manuscript_id], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                })
                .map_err(original)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(original)
        })?;
        let context = self.context()?;
        for (path, support, previous) in files {
            let entries = self.read(|db, _| {
                self.sql
                    .prepare(db, "project-bibliography-projection-entries.v1.sql")
                    .map_err(original)?
                    .query_map(params![self.operation.manuscript_id, path], |row| {
                        row.get::<_, String>(0)
                    })
                    .map_err(original)?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(original)
            })?;
            let mut file = PublicationFile {
                path: path.clone(),
                target: context.target.clone(),
                support,
                entries,
                supplemental_entries: context.entries.clone(),
                supplemental_support: context.support.clone(),
                automatic: None,
            };
            let decision = self.publication_decision(PublicationInput::Content {
                file: Box::new(file.clone()),
            })?;
            let decision = if matches!(decision, PublicationDecision::NeedAutomatic) {
                file.automatic = Some(self.read(|db, _| {
                    let imported_keys = self
                        .strings(db, "project-bibliography-adoption-occupied.v1.sql")
                        .map_err(original)?;
                    let bindings = self.bindings(db).map_err(original)?;
                    Ok(AutomaticProjection {
                        imported_keys,
                        retained_keys: retained.clone(),
                        bindings,
                    })
                })?);
                self.publication_decision(PublicationInput::Content {
                    file: Box::new(file),
                })?
            } else {
                decision
            };
            let PublicationDecision::Content { content } = decision else {
                return Err(self.mismatch());
            };
            let access = self
                .operation
                .store
                .manuscript_files(&self.operation.manuscript_id);
            let result = loop {
                let actual_sha256 = if access
                    .present(&path)
                    .map_err(|cause| self.file_error(cause))?
                {
                    Some(
                        access
                            .read(&path)
                            .map_err(|cause| self.file_error(cause))?
                            .1,
                    )
                } else {
                    None
                };
                let PublicationDecision::Compared { result } =
                    self.publication_decision(PublicationInput::Compare {
                        previous: previous.clone(),
                        target_sha256: content.sha256.clone(),
                        actual_sha256: actual_sha256.clone(),
                    })?
                else {
                    return Err(self.mismatch());
                };
                let current = if access
                    .present(&path)
                    .map_err(|cause| self.file_error(cause))?
                {
                    Some(
                        access
                            .read(&path)
                            .map_err(|cause| self.file_error(cause))?
                            .1,
                    )
                } else {
                    None
                };
                if current == actual_sha256 {
                    break result;
                }
            };
            match result {
                FileComparison::Current => {}
                FileComparison::Conflict => {
                    self.conflict(outcome)?;
                    return Err(self.error(WritingError::BibliographyConflict));
                }
                FileComparison::Write { expected } => {
                    *outcome = SaveOutcome::Unknown;
                    access
                        .save(&path, &content.content, expected.as_deref())
                        .map_err(|cause| self.file_error(cause))?;
                }
            }
            self.write(outcome, |db| {
                self.operation
                    .resources
                    .index(
                        db,
                        &self.operation.manuscript_id,
                        &path,
                        "bib",
                        content.content.len(),
                        &content.sha256,
                    )
                    .map_err(data_failure)?;
                if path == context.target {
                    self.record_status(db, &path, Some(&content.sha256), "ready")?;
                }
                db.execute(
                    self.sql
                        .get("project-bibliography-projection-record.v1.sql"),
                    params![self.operation.manuscript_id, path, content.sha256],
                )
                .map_err(|cause| self.error(cause.into()))?;
                Ok(())
            })?;
        }
        if let Some(before) = before
            && self.fingerprint()? == Some(before)
        {
            proofs.record(&self.operation.manuscript_id, before);
        }
        if *outcome == SaveOutcome::Unknown {
            *outcome = SaveOutcome::Committed {
                item_id: self.operation.manuscript_id.clone(),
            };
        }
        Ok(context)
    }
    fn conflict(&self, outcome: &mut SaveOutcome) -> NativeResult<()> {
        let status = self.read(|db, reader| self.status_in(db, reader))?;
        self.write(outcome, |db| {
            self.record_status(
                db,
                &status.relative_path,
                status.content_hash.as_deref(),
                "conflict",
            )
        })
    }
    pub(super) fn verify(&self, outcome: &mut SaveOutcome) -> NativeResult<()> {
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
        access
            .check_identity()
            .map_err(|cause| self.file_error(cause))?;
        for (path, expected) in files {
            let status = self.read(|db, reader| self.status_in(db, reader))?;
            let (actual, matches) = loop {
                let actual = if access
                    .present(&path)
                    .map_err(|cause| self.file_error(cause))?
                {
                    Some(access.read(&path).map_err(|cause| self.file_error(cause))?)
                } else {
                    None
                };
                let PublicationDecision::Verified { matches } =
                    self.publication_decision(PublicationInput::Verify {
                        expected: expected.clone(),
                        actual_sha256: actual.as_ref().map(|(_, hash)| hash.clone()),
                    })?
                else {
                    return Err(self.mismatch());
                };
                let current = if access
                    .present(&path)
                    .map_err(|cause| self.file_error(cause))?
                {
                    Some(
                        access
                            .read(&path)
                            .map_err(|cause| self.file_error(cause))?
                            .1,
                    )
                } else {
                    None
                };
                if current.as_deref() == actual.as_ref().map(|(_, hash)| hash.as_str()) {
                    break (actual, matches);
                }
            };
            if !matches {
                self.write(outcome, |db| {
                    self.record_status(
                        db,
                        &status.relative_path,
                        status.content_hash.as_deref(),
                        "conflict",
                    )
                })?;
                return Err(self.error(WritingError::BibliographyConflict));
            }
            if let Some((text, hash)) = actual {
                self.write(outcome, |db| {
                    self.operation
                        .resources
                        .index(
                            db,
                            &self.operation.manuscript_id,
                            &path,
                            "bib",
                            text.len(),
                            &hash,
                        )
                        .map_err(data_failure)?;
                    if path == status.relative_path {
                        self.record_status(db, &path, Some(&hash), "ready")?;
                    }
                    Ok(())
                })?;
            }
        }
        if *outcome == SaveOutcome::Unknown {
            *outcome = SaveOutcome::Committed {
                item_id: self.operation.manuscript_id.clone(),
            };
        }
        Ok(())
    }
}
