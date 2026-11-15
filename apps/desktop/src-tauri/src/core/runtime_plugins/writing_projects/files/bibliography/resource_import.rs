//! Original resource upload over shared Core bytes and Native bibliography rules.
use super::*;
use crate::{
    data_interface::guarded_transaction,
    runtime_plugins::writing_projects::files::bibliography_transaction::{self, Phases},
    scoped_text_files::legacy::{manuscript_source_directory, sha256},
    writing_resource_files::{self, MAX_RESOURCE_FILES, StagedResource},
};
use research_workbench_native_sdk::writing_project::ManuscriptResourceImportResult;
use std::{collections::HashSet, fs, path::Path};

impl Runtime<'_> {
    pub(super) fn import_resources(
        &self,
        destination: &str,
        sources: &[String],
        outcome: &mut SaveOutcome,
    ) -> NativeResult<ManuscriptResourceImportResult> {
        if sources.is_empty() || sources.len() > MAX_RESOURCE_FILES {
            return Err(self.error(WritingError::FileTooLarge));
        }
        let destination =
            research_workbench_native_sdk::resource_path::validate_relative_path(destination)
                .map_err(|_| self.error(WritingError::InvalidPath))?;
        self.read(|db, reader| self.project(db, reader).map(|_| ()))?;
        let root = manuscript_source_directory(
            self.operation.store.manuscript_directory(),
            &self.operation.manuscript_id,
        )
        .map_err(|cause| self.error(cause.into()))?;
        let staging = root
            .parent()
            .ok_or_else(|| self.error(WritingError::InvalidPath))?
            .join(format!(".writing-resources-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&staging).map_err(|cause| self.error(cause.into()))?;
        let result = (|| {
            let mut seen = HashSet::new();
            let mut total = 0;
            let mut resources = Vec::new();
            for source in sources {
                let mut resource = writing_resource_files::stage_resource(
                    source,
                    &root,
                    &destination,
                    &staging,
                    &mut seen,
                    &mut total,
                )
                .map_err(|cause| self.error(cause))?;
                let (_, bibliography) = self.resource_kind(&resource.relative_path)?;
                if bibliography {
                    resource.original_bibtex = Some(
                        fs::read_to_string(&resource.staged_path)
                            .map_err(|cause| self.error(cause.into()))?,
                    );
                }
                resources.push(resource);
            }
            self.normalize_resources(&mut resources, outcome)?;
            self.publish_resources(&root, &resources, outcome)
        })();
        let _ = fs::remove_dir_all(staging);
        result
    }

    fn resource_kind(&self, path: &str) -> NativeResult<(String, bool)> {
        match self.adoption_decision(AdoptionInput::ResourcePath {
            relative_path: path.into(),
        })? {
            AdoptionDecision::ResourcePath {
                file_kind,
                managed_bibliography,
            } => Ok((file_kind, managed_bibliography)),
            _ => Err(self.mismatch()),
        }
    }

    fn resource_source(
        &self,
        source: String,
        mapping: &std::collections::BTreeMap<String, String>,
        existing: bool,
    ) -> NativeResult<String> {
        match self.adoption_decision(AdoptionInput::ResourceSource {
            source,
            mapping: mapping.clone(),
            existing,
        })? {
            AdoptionDecision::ResourceSource { text } => Ok(text),
            _ => Err(self.mismatch()),
        }
    }

    fn normalize_resources(
        &self,
        resources: &mut [StagedResource],
        outcome: &mut SaveOutcome,
    ) -> NativeResult<()> {
        let mut sources = Vec::new();
        for resource in resources.iter() {
            if resource.original_bibtex.is_some() {
                sources.push((
                    resource.relative_path.clone(),
                    fs::read_to_string(&resource.staged_path)
                        .map_err(|cause| self.error(cause.into()))?,
                ));
            }
        }
        if sources.is_empty() {
            return Ok(());
        }
        // Original static adopt includes redo recovery and Always publication.
        self.operation.recover(outcome)?;
        self.adopt(outcome)?;
        self.publish(outcome)?;
        let plan = self.read(|db, reader| {
            let mut occupied = self
                .strings(db, "project-bibliography-adoption-occupied.v1.sql")
                .map_err(original)?;
            occupied.extend(
                self.strings(db, "project-bibliography-adoption-bound-keys.v1.sql")
                    .map_err(original)?,
            );
            match reader.adoption(AdoptionInput::NormalizeResources {
                sources: sources.clone(),
                occupied,
            })? {
                AdoptionDecision::ResourcesNormalized { plan } => Ok(plan),
                _ => reader.mismatch(),
            }
        })?;
        if !plan.document_mapping.is_empty() {
            let files = self.read(|db, reader| {
                let (_, manuscript) = self.project(db, reader)?;
                self.files(db, reader, manuscript)
            })?;
            for file in files.iter().filter(|file| file.kind == "tex") {
                let (text, _) = self
                    .operation
                    .store
                    .manuscript_files(&self.operation.manuscript_id)
                    .read(&file.relative_path)
                    .map_err(|cause| self.file_error(cause))?;
                self.resource_source(text, &plan.document_mapping, true)?;
            }
        }
        for resource in resources {
            let next = if let Some((_, text)) = plan
                .files
                .iter()
                .find(|(path, _)| path == &resource.relative_path)
            {
                Some(text.clone())
            } else if self.resource_kind(&resource.relative_path)?.0 == "tex"
                && !plan.mapping.is_empty()
            {
                Some(
                    self.resource_source(
                        fs::read_to_string(&resource.staged_path)
                            .map_err(|cause| self.error(cause.into()))?,
                        &plan.mapping,
                        false,
                    )?,
                )
            } else {
                None
            };
            if let Some(next) = next {
                fs::write(&resource.staged_path, &next)
                    .map_err(|cause| self.error(cause.into()))?;
                resource.byte_size = next.len() as u64;
                resource.sha256 = sha256(next.as_bytes());
            }
        }
        Ok(())
    }

    fn publish_resources(
        &self,
        root: &Path,
        resources: &[StagedResource],
        outcome: &mut SaveOutcome,
    ) -> NativeResult<ManuscriptResourceImportResult> {
        let kinds = resources
            .iter()
            .map(|resource| {
                self.resource_kind(&resource.relative_path)
                    .map(|kind| kind.0)
            })
            .collect::<NativeResult<Vec<_>>>()?;
        let published = writing_resource_files::publish_files(root, resources)
            .map_err(|cause| self.error(cause))?;
        *outcome = SaveOutcome::Unknown;
        let mut db = match self.operation.store.connection() {
            Ok(db) => db,
            Err(cause) => {
                writing_resource_files::remove_published(&published);
                *outcome = SaveOutcome::RolledBack;
                return Err(self.error(cause.into()));
            }
        };
        let attempt = guarded_transaction::run(&mut db, |db| {
            let attempt = bibliography_transaction::run::<_, std::convert::Infallible>(
                db,
                Phases {
                    begin: "writing_resource_import_begin",
                    commit: "writing_resource_import_commit",
                    rollback: "writing_resource_import_rollback",
                    unsettled: "writing_resource_import_unsettled",
                },
                |tx| {
                    for (resource, kind) in resources.iter().zip(&kinds) {
                        self.operation
                            .resources
                            .index(
                                tx,
                                &self.operation.manuscript_id,
                                &resource.relative_path,
                                kind,
                                resource.byte_size as usize,
                                &resource.sha256,
                            )
                            .map_err(|cause| {
                                Flow::Fail(Failure {
                                    error: WritingError::Storage(
                                        crate::storage::StorageError::DataInterface(
                                            crate::data_interface::DataError::Unavailable,
                                        ),
                                    ),
                                    cause: FailureCause::SourceData(cause),
                                })
                            })?;
                    }
                    Ok(())
                },
            );
            let unknown = attempt.requires_recovery;
            (Ok::<_, std::convert::Infallible>(attempt), unknown)
        })
        .expect("transaction result carries resource import settlement");
        drop(db);
        if let Err(cause) = attempt.result {
            if !attempt.requires_recovery {
                writing_resource_files::remove_published(&published);
                *outcome = SaveOutcome::RolledBack;
            }
            let mut error = match cause {
                Flow::Fail(cause) => self.failed(cause, &Reader::default()),
                Flow::Need(never) => match never {},
            };
            error.save_rollback_error = attempt.rollback_error.map(Box::new);
            return Err(error);
        }
        *outcome = SaveOutcome::Committed {
            item_id: self.operation.manuscript_id.clone(),
        };
        let originals = resources
            .iter()
            .filter_map(|resource| {
                resource
                    .original_bibtex
                    .as_ref()
                    .map(|text| (resource.relative_path.clone(), text.clone()))
            })
            .collect::<Vec<_>>();
        if !originals.is_empty() {
            self.operation.recover(outcome)?;
            self.adopt_with_originals(outcome, Some(&originals))?;
            self.publish(outcome)?;
            self.regenerate(false, outcome)?;
        }
        Ok(ManuscriptResourceImportResult {
            imported_paths: resources
                .iter()
                .map(|resource| resource.relative_path.clone())
                .collect(),
            byte_size: resources.iter().map(|resource| resource.byte_size).sum(),
        })
    }
}

#[tauri::command]
pub(crate) async fn import_runtime_manuscript_resources(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<NativePreviewService>>,
    catalog: tauri::State<'_, Arc<RuntimePluginCatalog>>,
    package_key: String,
    manuscript_id: String,
    destination_directory: String,
    source_paths: Vec<String>,
) -> NativeResult<ManuscriptResourceImportResult> {
    match super::super::operate_files(
        store.inner().clone(),
        Arc::clone(service.inner()),
        Arc::clone(catalog.inner()),
        package_key.clone(),
        manuscript_id,
        FileRequest::Bibliography {
            call: Call::ImportResources {
                destination_directory: destination_directory.trim().into(),
                source_paths,
            },
        },
    )
    .await?
    {
        FileResult::ResourcesImported(result) => Ok(result),
        _ => Err(failure(
            "plugin_host_response_mismatch",
            "资源上传结果种类不匹配。",
            &package_key,
        )),
    }
}
