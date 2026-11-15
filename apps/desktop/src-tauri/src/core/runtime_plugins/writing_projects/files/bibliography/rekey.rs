//! Full key migration retains the original committed redo and scoped-file recovery.
use super::super::bibliography_rekey::{self, RekeyBatch, RekeySql, RekeyTable};
use super::*;
use crate::data_interface::guarded_transaction;
use research_workbench_native_sdk::{
    writing_bibliography::rekey::{
        Assignment, Decision as RekeyDecision, Input as RekeyInput, Migration,
    },
    writing_project::{CitationKeyWrite, ManuscriptFileContent},
};
impl Runtime<'_> {
    pub(super) fn migrate(
        &self,
        assignments: &[Assignment],
        outcome: &mut SaveOutcome,
    ) -> NativeResult<Migration> {
        // The outer operation performs migrate's first recovery. List performs the
        // second original recovery before adoption/publication and complete entry reads.
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
        let RekeyDecision::Mapping { mapping } = self.read(|_, reader| {
            reader.rekey(RekeyInput::Mapping {
                assignments: assignments.to_vec(),
                entries: entries.clone(),
            })
        })?
        else {
            return Err(self.mismatch());
        };
        let writes = self.plan_rekey(assignments, &mapping, outcome)?;
        self.operation.recover(outcome)?;
        self.regenerate(false, outcome)?;
        let checkpoints = self.operation.checkpoint_list(&self.sql.checkpoints)?;
        let files = writes
            .into_iter()
            .map(|write| ManuscriptFileContent {
                relative_path: write.path,
                sha256: crate::scoped_text_files::legacy::sha256(write.content.as_bytes()),
                content: write.content,
            })
            .collect();
        Ok(Migration {
            assignments: assignments.to_vec(),
            files,
            checkpoints,
        })
    }
    fn plan_rekey(
        &self,
        assignments: &[Assignment],
        mapping: &std::collections::BTreeMap<String, String>,
        outcome: &mut SaveOutcome,
    ) -> NativeResult<Vec<CitationKeyWrite>> {
        let sql = RekeySql {
            checkpoint_read: self.sql.get("project-checkpoint-read.v1.sql"),
            checkpoint_update: self
                .sql
                .get("project-bibliography-rekey-checkpoint-update.v1.sql"),
            tables: [
                RekeyTable {
                    records: self
                        .sql
                        .get("project-bibliography-rekey-bindings-records.v1.sql"),
                    temporary: self
                        .sql
                        .get("project-bibliography-rekey-bindings-temporary.v1.sql"),
                    update: self
                        .sql
                        .get("project-bibliography-rekey-bindings-update.v1.sql"),
                },
                RekeyTable {
                    records: self
                        .sql
                        .get("project-bibliography-rekey-entries-records.v1.sql"),
                    temporary: self
                        .sql
                        .get("project-bibliography-rekey-entries-temporary.v1.sql"),
                    update: self
                        .sql
                        .get("project-bibliography-rekey-entries-update.v1.sql"),
                },
            ],
            base: self.sql.get("project-bibliography-rekey-base.v1.sql"),
            writes: self.sql.get("project-bibliography-rekey-writes.v1.sql"),
        };
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
                    let shared = RefCell::new(&mut reader);
                    let attempt = bibliography_rekey::plan_in(
                        db,
                        &sql,
                        &RekeyBatch {
                            manuscript_id: &self.operation.manuscript_id,
                            assignments,
                            mapping,
                        },
                        &mut |db| {
                            let mut reader = shared.borrow_mut();
                            let (_, manuscript) = self.project(db, &mut reader)?;
                            self.files(db, &mut reader, manuscript)
                        },
                        &mut |path| {
                            self.operation
                                .store
                                .manuscript_files(&self.operation.manuscript_id)
                                .read(path)
                                .map_err(Into::into)
                        },
                        &mut |input| shared.borrow_mut().rekey(input),
                    );
                    if attempt.requires_recovery {
                        *outcome = SaveOutcome::Unknown;
                    } else if attempt.result.is_ok() {
                        *outcome = SaveOutcome::Committed {
                            item_id: self.operation.manuscript_id.clone(),
                        };
                    } else if matches!(attempt.result, Err(Flow::Fail(_)))
                        && *outcome == SaveOutcome::NotStarted
                    {
                        *outcome = SaveOutcome::RolledBack;
                    }
                    let unknown = attempt.requires_recovery;
                    (Ok::<_, std::convert::Infallible>(attempt), unknown)
                })
                .expect("transaction result carries its finite settlement")
            };
            match attempt.result {
                Ok(writes) => return Ok(writes),
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
pub(crate) async fn rekey_runtime_manuscript_citations(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<NativePreviewService>>,
    catalog: tauri::State<'_, Arc<RuntimePluginCatalog>>,
    package_key: String,
    manuscript_id: String,
    assignments: Vec<Assignment>,
) -> NativeResult<Migration> {
    match super::super::operate_files(
        store.inner().clone(),
        Arc::clone(service.inner()),
        Arc::clone(catalog.inner()),
        package_key.clone(),
        manuscript_id,
        FileRequest::Bibliography {
            call: Call::Rekey { assignments },
        },
    )
    .await?
    {
        FileResult::CitationKeyMigration(migration) => Ok(*migration),
        _ => Err(failure(
            "plugin_host_response_mismatch",
            "引用键迁移结果种类不匹配。",
            &package_key,
        )),
    }
}
