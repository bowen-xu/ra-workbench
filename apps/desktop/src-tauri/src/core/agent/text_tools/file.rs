use super::*;
use crate::agent::{
    repository::execution_workspaces,
    workspace::{TextPatchOperation, WorkspaceLease},
    writing_submission::WritingSubmissionReceipt,
};
use rusqlite::Connection;
#[path = "file/owner.rs"]
mod owner;

#[derive(Clone)]
pub(crate) enum WorkspaceGrant {
    Original,
    #[cfg(target_os = "macos")]
    Generation {
        generation: u32,
        predecessor: uuid::Uuid,
    },
}

#[derive(Clone)]
pub(super) struct FileBackend {
    pub store: AppStore,
    pub record: SubmissionRecord,
    pub scope: Option<crate::agent::writing_resources::OwnedWritingScope>,
    #[cfg(target_os = "macos")]
    pub workspace: super::file_workspace::FileWorkspaceHandle,
}
impl FileBackend {
    pub(super) fn receipt(&self) -> Result<WritingSubmissionReceipt, String> {
        let db = self.store.connection().map_err(|error| error.to_string())?;
        self.receipt_in(&db)
    }
    pub(super) fn receipt_in(&self, db: &Connection) -> Result<WritingSubmissionReceipt, String> {
        // The immutable submission identifies the manuscript even after its first
        // lease is revoked. Actual file access is separately authorized against
        // the currently held original or successor lease in with_lease.
        let (binding, _) = execution_workspaces::read_for_review(
            db,
            self.record.conversation_id,
            self.record.request_id,
        )
        .map_err(|error| error.to_string())?
        .ok_or("no approved execution workspace")?;
        if binding.turn_id != self.record.turn_id
            || binding.context_snapshot_id != self.record.context_snapshot_id
        {
            return Err("workspace binding changed".into());
        }
        Ok(WritingSubmissionReceipt {
            conversation_id: self.record.conversation_id,
            request_id: self.record.request_id,
            manuscript_id: binding.manuscript_id,
        })
    }
    pub(super) fn path_in(
        &self,
        db: &Connection,
        path: &str,
        write: bool,
    ) -> Result<String, String> {
        let receipt = self.receipt_in(db)?;
        let prefix = format!("workspace/{}/", receipt.manuscript_id);
        let decoded;
        let relative = if path.starts_with("source://") {
            let addresses::Target::File {
                project,
                path: relative,
            } = addresses::resolve(db, &self.record, path)?
            else {
                return Err("source is not a project file".into());
            };
            if project != receipt.manuscript_id.to_string() {
                return Err("project is not registered for this submission".into());
            }
            decoded = relative;
            decoded.as_str()
        } else {
            path.strip_prefix(&prefix)
                .ok_or("workspace is not registered for this submission")?
        };
        research_agent_core::workspace::validate_path(relative)
            .map_err(|error| error.to_string())?;
        let allowed = if let Some(scope) =
            crate::agent::context_session::permissions::writing_scope(&self.record)
        {
            let source = self
                .scope
                .as_deref()
                .map(crate::agent::writing_resources::WritingScope::Native)
                .or_else(|| {
                    self.store
                        .writing_resource_source()
                        .map(crate::agent::writing_resources::WritingScope::Static)
                });
            crate::agent::context_session::group_scope::path_access_with_scope(
                &scope, relative, write, source,
            )
            .map_err(|error| error.to_string())?
        } else {
            let paths = if write {
                crate::agent::context_session::permissions::writing_paths(&self.record)
            } else {
                crate::agent::context_session::permissions::selected_writing_paths(&self.record)
            };
            paths.is_none_or(|paths| paths.contains(relative))
        };
        if !allowed {
            return Err("file is outside the selected access scope".into());
        }
        Ok(relative.into())
    }
    fn preflight_in(
        &self,
        db: &Connection,
        lease: &WorkspaceLease,
        changes: &[TextChange],
    ) -> Result<Vec<(String, Option<String>)>, String> {
        let paths = changes
            .iter()
            .map(|change| {
                Ok((
                    self.path_in(db, &change.path, true)?,
                    change
                        .destination
                        .as_ref()
                        .map(|path| self.path_in(db, path, true))
                        .transpose()?,
                ))
            })
            .collect::<Result<Vec<_>, String>>()?;
        for (change, (relative, destination)) in changes.iter().zip(&paths) {
            let current = lease
                .read_text(relative)
                .map_err(|error| error.to_string())?;
            if current.as_deref()
                != change
                    .before
                    .as_ref()
                    .map(|snapshot| snapshot.text.as_str())
            {
                return Err("file changed or creation target already exists".into());
            }
            if let Some(destination) = destination
                && lease
                    .read_text(destination)
                    .map_err(|error| error.to_string())?
                    .is_some()
            {
                return Err("move destination exists".into());
            }
        }
        Ok(paths)
    }
}
impl TextBackend for FileBackend {
    fn read<'a>(
        &'a self,
        path: &'a str,
        _saved: bool,
        remember: bool,
    ) -> BackendFuture<'a, TextSnapshot> {
        Box::pin(async move {
            let path = path.to_owned();
            self.with_lease(move |backend, db, lease, _, _| {
                let relative = backend.path_in(db, &path, false)?;
                let text = lease
                    .read_text(&relative)
                    .map_err(|error| error.to_string())?
                    .ok_or("file does not exist")?;
                let snapshot = TextSnapshot {
                    path,
                    name: relative.rsplit('/').next().map(str::to_owned),
                    version: format!("sha256:{:x}", Sha256::digest(text.as_bytes())),
                    text,
                };
                if remember {
                    journal::remember(db, &backend.record, &snapshot)?;
                }
                Ok(snapshot)
            })
            .await
        })
    }
    fn preflight<'a>(&'a self, changes: &'a [TextChange]) -> BackendFuture<'a, ()> {
        Box::pin(async move {
            let changes = changes.to_owned();
            self.with_lease(move |backend, db, lease, _, _| {
                backend.preflight_in(db, lease, &changes).map(|_| ())
            })
            .await
        })
    }
    fn commit<'a>(
        &'a self,
        request_id: &'a str,
        changes: Vec<TextChange>,
    ) -> BackendFuture<'a, Vec<TargetResult>> {
        Box::pin(async move {
            match self.query(request_id).await {
                Ok(Some(result)) => return Ok(result),
                Ok(None) => {}
                Err(error) => {
                    return Ok(patch_journal::unknown_changes(
                        &changes,
                        &patch_journal::unavailable_receipt(&self.record, request_id, Some(&error)),
                    ));
                }
            }
            let request_id = request_id.to_owned();
            let input = serde_json::to_string(&changes).map_err(|error| error.to_string())?;
            self.with_lease(move |backend, db, lease, _, _| {
                // Inspect an existing operation before reading current bytes.
                // A lost original receipt never becomes a new CAS/write.
                match journal::lookup_sql(db,backend.record.conversation_id,&request_id,&input) {
                    Ok(Some(result)) if result["status"]!="unknown"=>return serde_json::from_value(result["targets"].clone()).map_err(|_|"invalid file receipt".into()),
                    Ok(Some(_))=>return Ok(patch_journal::unknown_changes(&changes,&patch_journal::unavailable_receipt(&backend.record,&request_id,None))),
                    Ok(None)=>{},
                    Err(error) if error.uncertain()=>{
                        let mut detail=error.detail(backend.record.conversation_id,Some(backend.record.request_id),&request_id);
                        patch_journal::target_identity(&mut detail,&backend.record,&request_id);
                        return Ok(patch_journal::unknown_changes(&changes,&detail));
                    }
                    Err(error)=>return Err(error.target_message(&backend.record,&request_id)),
                }
                // CAS and the confirmed-absent journal insertion share both
                // original locks with mutation. A rejected insert writes no file.
                let paths = backend.preflight_in(db, lease, &changes)?;
                journal::insert_sql(db,backend.record.conversation_id,&request_id,&input).map_err(|error|error.target_message(&backend.record,&request_id))?;
            let mut results = vec![];
            for (change, (relative, destination)) in changes.into_iter().zip(paths) {
                let name = relative.rsplit('/').next().map(str::to_owned);
                let mut operations = vec![TextPatchOperation {
                    path: relative,
                    before: change.before.as_ref().map(|snapshot| snapshot.text.clone()),
                    after: change.after.clone(),
                }];
                if let Some(destination) = destination {
                    operations[0].after = None;
                    operations.push(TextPatchOperation {
                        path: destination,
                        before: None,
                        after: change.after,
                    });
                }
                let status = match lease.apply_text_operations(&operations) {
                    Ok(_) => TargetStatus::Applied,
                    Err(_) => TargetStatus::Unknown,
                };
                results.push(TargetResult{path:change.path,status,message:if status==TargetStatus::Applied{"Authorized execution copy updated; formal delivery remains separate."}else{"File operation did not confirm completion; inspect original source and destination before retry."}.into(),change_id:None,generation:None,name});
                if let Err(error)=journal::record_sql(
                    db,
                    backend.record.conversation_id,
                    &request_id,
                    &json!({"targets":results}),
                    false,
                ) {
                    let detail=patch_journal::target_diagnostic(&backend.record,&request_id,"target-progress",&error);
                    patch_journal::uncertain_targets(&mut results,&detail);
                    return Ok(results);
                }
            }
            if let Err(error)=journal::record_sql(
                db,
                backend.record.conversation_id,
                &request_id,
                &json!({"targets":results}),
                !results.iter().any(|result|result.status==TargetStatus::Unknown),
            ) {
                let detail=patch_journal::target_diagnostic(&backend.record,&request_id,"target-terminal",&error);
                patch_journal::uncertain_targets(&mut results,&detail);
            }
            Ok(results)
            }).await
        })
    }
    fn query<'a>(&'a self, request_id: &'a str) -> BackendFuture<'a, Option<Vec<TargetResult>>> {
        Box::pin(async move {
            let db = self.store.connection().map_err(|error| {
                patch_journal::query_owner_diagnostic(&self.record, request_id, &error).to_string()
            })?;
            let value:Option<String>=rusqlite::OptionalExtension::optional(db.query_row("select results_json from agent_text_patch_calls where conversation_id=?1 and request_id=?2 and state='complete'",rusqlite::params![self.record.conversation_id.to_string(),request_id],|row|row.get(0))).map_err(|error|patch_journal::target_diagnostic(&self.record,request_id,"target-query",&error).to_string())?;
            value
                .map(|value| {
                    let results: Vec<TargetResult> = serde_json::from_str::<Value>(&value)
                        .and_then(|result| serde_json::from_value(result["targets"].clone()))
                        .map_err(|_| {
                            patch_journal::unavailable_receipt(&self.record, request_id, None)
                                .to_string()
                        })?;
                    if results.is_empty() {
                        return Err(patch_journal::unavailable_receipt(
                            &self.record,
                            request_id,
                            None,
                        )
                        .to_string());
                    }
                    Ok(results)
                })
                .transpose()
        })
    }
}
