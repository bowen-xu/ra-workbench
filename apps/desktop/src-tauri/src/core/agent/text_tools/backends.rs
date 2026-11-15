//! Route text targets while keeping the original file workspace capability.
use super::*;

impl TextTools {
    /// Preserve a successor preparation recovery barrier as a typed runner
    /// error before a file read or patch journal can turn it into model retry
    /// feedback. Non-file targets never acquire a writing workspace.
    #[cfg(target_os = "macos")]
    pub(super) async fn ensure_file_owner_for_call(&self, call: &ToolCall) -> Result<(), RunError> {
        let Some(source) = &self.file_workspace else {
            return Ok(());
        };
        let is_file =
            |path: &str| path.starts_with("source://writings/") || path.starts_with("workspace/");
        let needs_owner = if call.name == READ {
            call.arguments
                .get("path")
                .and_then(Value::as_str)
                .is_some_and(is_file)
        } else if call.name == PATCH {
            match call.arguments.get("patch").and_then(Value::as_str) {
                Some(patch) => {
                    // An existing call must be reconciled by its original journal;
                    // it must not initiate a successor just to return a receipt.
                    let key = format!("{}:{}", self.record.request_id, call.call_id);
                    let prior = self.store.connection().ok().map(|db| {
                        journal::lookup_sql(&db, self.record.conversation_id, &key, patch)
                    });
                    if !matches!(prior, Some(Ok(None))) {
                        return Ok(());
                    }
                    text_patch::parse(patch).is_ok_and(|hunks| {
                        hunks.iter().any(|hunk| match hunk {
                            Hunk::AddFile { path, .. } | Hunk::DeleteFile { path } => {
                                is_file(&path.to_string_lossy())
                            }
                            Hunk::UpdateFile {
                                path, move_path, ..
                            } => {
                                is_file(&path.to_string_lossy())
                                    || move_path
                                        .as_ref()
                                        .is_some_and(|path| is_file(&path.to_string_lossy()))
                            }
                        })
                    })
                }
                None => false,
            }
        } else {
            false
        };
        if needs_owner
            && let Err(
                error @ (RunError::ToolOutcomeUnknown | RunError::ToolOutcomeUnknownDiagnostic(_)),
            ) = source.workspace(&self.record).await
        {
            return Err(error);
        }
        Ok(())
    }
    pub(super) fn file_backend(&self) -> file::FileBackend {
        file::FileBackend {
            store: self.store.clone(),
            record: self.record.clone(),
            scope: None,
            #[cfg(target_os = "macos")]
            workspace: self.file_workspace.clone(),
        }
    }
    pub(super) fn backend(&self, path: &str) -> Result<Box<dyn TextBackend>, String> {
        self.backend_with_scope(path, None)
    }
    pub(super) fn backend_with_scope(
        &self,
        path: &str,
        scope: Option<crate::agent::writing_resources::OwnedWritingScope>,
    ) -> Result<Box<dyn TextBackend>, String> {
        if path.starts_with("source://") {
            let parts = text_patch::address::parse(path)?;
            if parts[0] == "agents" {
                let reading = ["skills", "edit-structured-resources", "assets", "reading"];
                if parts.get(1..5).is_none_or(|parts| parts != reading) {
                    crate::system_documents::document_id_from_path(path)?;
                    return Ok(Box::new(crate::system_documents::backend::Backend {
                        store: self.store.clone(),
                        record: self.record.clone(),
                        cancellation: self.cancellation.clone(),
                    }));
                }
            } else if parts[0] == "assets" {
                return Err("找不到文件对象".into());
            }
        }
        if path.starts_with("source://") {
            let db = self.store.connection().map_err(|error| error.to_string())?;
            let resolved = addresses::resolve(&db, &self.record, path);
            if resolved.is_err() && addresses::is_note(path) {
                return Ok(Box::new(note::NoteBackend {
                    scope: scope.clone(),
                    store: self.store.clone(),
                    record: self.record.clone(),
                }));
            }
            return match resolved? {
                addresses::Target::Note { .. } => Ok(Box::new(note::NoteBackend {
                    scope: scope.clone(),
                    store: self.store.clone(),
                    record: self.record.clone(),
                })),
                addresses::Target::File { path, .. } if !path.is_empty() => {
                    Ok(Box::new(self.file_backend()))
                }
                addresses::Target::Structured(_) | addresses::Target::TemplateCatalog => {
                    Ok(Box::new(structured::StructuredBackend {
                        store: self.store.clone(),
                        record: self.record.clone(),
                    }))
                }
                _ => Err("resource has no editable text view".into()),
            };
        }
        research_agent_core::workspace::validate_path(path).map_err(|_| "invalid legacy path")?;
        if path.starts_with("workspace/") {
            Ok(Box::new(self.file_backend()))
        } else if path.starts_with("notes/") {
            Ok(Box::new(note::NoteBackend {
                scope: scope.clone(),
                store: self.store.clone(),
                record: self.record.clone(),
            }))
        } else {
            Err("reuse an existing source:// address returned by resource discovery".into())
        }
    }
}
