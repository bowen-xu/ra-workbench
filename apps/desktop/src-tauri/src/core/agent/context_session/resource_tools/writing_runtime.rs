//! Current manuscript bodies use the original ordered evaluator and Native reader.
use super::*;
use crate::data_interface::ContextReadMode;

impl ResourceTools {
    pub(super) async fn runtime_writing_read(&self, call: &ToolCall) -> Option<ToolOutput> {
        if !matches!(
            call.name.as_str(),
            "research_read_source" | "research_search_sources"
        ) || self.store.writing_resource_source().is_some()
            || call.arguments.get("view").and_then(Value::as_str) == Some("structured")
        {
            return None;
        }
        let binding = self.store.native_writing_context()?.clone();
        let tools = self.clone();
        let request = call.clone();
        let selected = tokio::task::spawn_blocking(move || {
            let db = tools.store.connection().map_err(|_| Error::Unavailable)?;
            let entries = tools.selected_entries(&db, &request)?;
            let manuscript = entries
                .iter()
                .any(|entry| matches!(entry.resource, Some(Resource::ManuscriptFile { .. })));
            let supported = entries.iter().all(|entry| {
                matches!(
                    entry.resource,
                    Some(
                        Resource::ManuscriptFile { .. }
                            | Resource::Note { .. }
                            | Resource::Paper { .. }
                            | Resource::StructuredObject { .. }
                            | Resource::TemplateCatalog { .. }
                    ) | None
                )
            });
            Ok::<_, Error>((manuscript && supported).then(|| {
                if entries
                    .iter()
                    .all(|entry| matches!(entry.resource, Some(Resource::ManuscriptFile { .. })))
                {
                    ContextReadMode::Snapshot
                } else {
                    ContextReadMode::CoreProofs
                }
            }))
        })
        .await;
        let mode = match selected {
            Ok(Ok(Some(mode))) => mode,
            _ => return None,
        };
        let tools = self.clone();
        let request = call.clone();
        let result = binding
            .read_context(
                &self.store,
                self.record.request_id,
                mode,
                move |db, reader| {
                    // The evaluator rechecks live authorization on every original read-prefix replay.
                    // Core Note proofs retain their existing autocommit owner.
                    let result = tools.evaluate_with(db, &request, &mut |db, entry| {
                        let Some(Resource::ManuscriptFile { resource_id, path }) = &entry.resource
                        else {
                            return tools
                                .read(db, entry)
                                .map_err(sources::MaterializationError::Source);
                        };
                        let mut pieces = reader
                            .current_file(
                                db,
                                tools.store.manuscript_directory(),
                                *resource_id,
                                path,
                            )
                            .map_err(sources::MaterializationError::Writing)?;
                        if pieces.len() != 1 {
                            return Err(sources::MaterializationError::Source(
                                Error::SourceUnavailable,
                            ));
                        }
                        let mut source = pieces.remove(0);
                        source.source_id = entry.identity.source_id.clone();
                        Ok(source)
                    });
                    match result {
                        Ok(value) => Ok(Ok(value)),
                        Err(sources::MaterializationError::Source(error)) => Ok(Err(error)),
                        Err(sources::MaterializationError::Writing(flow)) => Err(flow),
                    }
                },
            )
            .await;
        Some(match result {
            Ok(result) => context_output(result),
            Err(error) => error.into_source_tool_output(&self.record, call),
        })
    }
}
