//! The original ordered read/search evaluator with one finite metadata producer.
use super::*;
use crate::{
    agent::{
        paper_metadata::PaperMetadataError,
        writing_context::{ContextRead, WritingContextReader},
    },
    ai::structured_resource_validation::{Failure, FailureCause},
    data_interface::guarded_transaction,
    hbs_storage::boundary::HbsOutcome,
    runtime_plugins::{
        context_metadata::{ContextMetadataCapture, ContextMetadataResources},
        literature_hbs::HbsOperationError,
        literature_trace::service::SourceReadError,
        native::{NativeHostError, NativeProfile},
    },
};

/// Ordered pure results belong to this one owned tool invocation, never a global cache.
#[derive(Clone, Default)]
pub(crate) struct Cache {
    rows: Vec<(Arc<ContextMetadataCapture>, String)>,
    scope: Option<crate::agent::writing_resources::OwnedWritingScope>,
    writing: bool,
}
impl Cache {
    pub(crate) fn resolved(&mut self, capture: Arc<ContextMetadataCapture>, body: String) {
        self.rows.push((capture, body));
    }
}
pub(crate) enum Step {
    Complete(Value),
    Resources,
    Writing,
    Project(Arc<ContextMetadataCapture>),
}
enum Halt {
    Context(Error),
    Data(crate::ai::structured_resource_validation::Failure<PaperMetadataError>),
    Resources,
    WritingResources,
    Writing(
        crate::ai::structured_resource_validation::Flow<
            crate::agent::writing_context::WritingContextError,
            crate::agent::writing_context::ContextNeed,
        >,
    ),
    Project(Arc<ContextMetadataCapture>),
}
impl From<Error> for Halt {
    fn from(value: Error) -> Self {
        Self::Context(value)
    }
}

impl ResourceTools {
    pub(super) async fn runtime_metadata_read(&self, call: &ToolCall) -> Option<ToolOutput> {
        if self.store.paper_metadata_source().is_some()
            || !matches!(
                call.name.as_str(),
                "research_read_source" | "research_search_sources"
            )
            || (call.name == "research_read_source"
                && call.arguments.get("view").and_then(Value::as_str) == Some("structured"))
        {
            return None;
        }
        let tools = self.clone();
        let request = call.clone();
        let selected = tokio::task::spawn_blocking(move || {
            let db = tools.store.connection().map_err(|_| Error::Unavailable)?;
            tools.selected_entries(&db, &request).map(|entries| {
                entries
                    .iter()
                    .any(|entry| matches!(entry.resource, Some(Resource::PaperMetadata { .. })))
            })
        })
        .await;
        // Unrelated resources and original authorization failures retain their evaluator.
        if !matches!(selected, Ok(Ok(true))) {
            return None;
        }
        let Some(binding) = self.store.native_note_trace().cloned() else {
            // Missing metadata does not take precedence over an earlier Core read
            // failure or search budget. Reach the producer in the original order.
            let mut cache = Cache::default();
            let result = loop {
                match attempt(
                    self.store.clone(),
                    self.record.clone(),
                    call.clone(),
                    None,
                    cache,
                )
                .await
                {
                    Ok((next, Ok(Step::Writing))) => cache = next,
                    Ok((_, Ok(Step::Complete(value)))) => break Ok(value),
                    Ok((_, Ok(Step::Resources | Step::Project(_)))) => {
                        break Err(SourceReadError::missing_binding());
                    }
                    Ok((_, Err(error))) | Err(error) => break Err(error),
                }
            };
            return Some(metadata_output(result, &self.record, call));
        };
        let result = binding
            .service
            .read_context_metadata(
                self.store.clone(),
                binding.package_key,
                self.record.clone(),
                call.clone(),
            )
            .await;
        Some(metadata_output(result, &self.record, call))
    }
}

fn metadata_output(
    result: Result<Value, SourceReadError>,
    record: &SubmissionRecord,
    call: &ToolCall,
) -> ToolOutput {
    match result {
        Ok(value) => context_output(Ok(value)),
        Err(SourceReadError::Context(Failure {
            error,
            cause: FailureCause::Original,
        })) => context_output(Err(error)),
        Err(error) => error.into_output(record, call),
    }
}

pub(crate) fn worker_unsettled() -> SourceReadError {
    SourceReadError::Native(Box::new(HbsOperationError::native(
        NativeHostError::for_profile(
            NativeProfile::NoteTraceDecision,
            "context_metadata_worker_unsettled",
            "文献来源读取尚未确认收尾；请核对原调用，不要自动重复写入。",
        ),
        "execution",
        HbsOutcome::Unknown,
    )))
}

/// Every attempt releases the connection before returning a Native need. Existing
/// Core reads retain their original order, proof owner and confirmed upsert semantics.
pub(crate) async fn attempt(
    store: AppStore,
    record: SubmissionRecord,
    call: ToolCall,
    resources: Option<ContextMetadataResources>,
    mut cache: Cache,
) -> Result<(Cache, Result<Step, SourceReadError>), SourceReadError> {
    if cache.scope.is_none() {
        cache.scope =
            crate::agent::writing_resources::native_scope_for_context(&store, &record).await;
    }
    if cache.writing
        && let Some(binding) = store.native_writing_context().cloned()
    {
        let tools = ResourceTools {
            store: store.clone(),
            record,
            scope: cache.scope.clone(),
        };
        return binding
            .read_context(
                &store,
                tools.record.request_id,
                crate::data_interface::ContextReadMode::CoreProofs,
                move |db, reader| {
                    // Each writing pause replays the same metadata prefix against current inputs.
                    let mut next = cache.clone();
                    let result = evaluate(
                        &tools,
                        db,
                        &call,
                        resources.as_ref(),
                        &mut next,
                        Some(reader),
                    )?;
                    Ok((next, result))
                },
            )
            .await
            .map_err(|error| SourceReadError::Writing(Box::new(error)));
    }
    tokio::task::spawn_blocking(move || {
        let mut db = store
            .connection()
            .map_err(|_| context_error(Error::Unavailable))?;
        let tools = ResourceTools {
            store: store.clone(),
            record,
            scope: cache.scope.clone(),
        };
        let result = guarded_transaction::run_mut(&mut db, |db| {
            if !db.is_autocommit() {
                return (Err(worker_unsettled()), true);
            }
            // Only an installed writing reader can produce a writing flow.
            let result = evaluate(&tools, db, &call, resources.as_ref(), &mut cache, None)
                .unwrap_or_else(|_| Err(worker_unsettled()));
            let unknown = !db.is_autocommit();
            (
                if unknown {
                    Err(worker_unsettled())
                } else {
                    result
                },
                unknown,
            )
        });
        Ok((cache, result))
    })
    .await
    .map_err(|_| worker_unsettled())?
}

fn evaluate(
    tools: &ResourceTools,
    db: &mut Connection,
    call: &ToolCall,
    resources: Option<&ContextMetadataResources>,
    cache: &mut Cache,
    mut writing: Option<&mut dyn WritingContextReader>,
) -> ContextRead<Result<Step, SourceReadError>> {
    let mut cursor = 0;
    let result = tools.evaluate_with(db, call, &mut |db, entry| {
        if tools.store.writing_resource_source().is_none()
            && let Some(Resource::ManuscriptFile { resource_id, path }) = &entry.resource
        {
            let Some(reader) = writing.as_deref_mut() else {
                if cache.writing {
                    return tools.read(db, entry).map_err(Halt::Context);
                }
                return Err(Halt::WritingResources);
            };
            let mut pieces = reader
                .current_file(db, tools.store.manuscript_directory(), *resource_id, path)
                .map_err(Halt::Writing)?;
            if pieces.len() != 1 {
                return Err(Halt::Context(Error::SourceUnavailable));
            }
            let mut source = pieces.remove(0);
            source.source_id = entry.identity.source_id.clone();
            return Ok(source);
        }
        let Some(Resource::PaperMetadata { resource_id }) = &entry.resource else {
            return tools.read(db, entry).map_err(Halt::Context);
        };
        let resources = resources.ok_or(Halt::Resources)?;
        let capture = resources.capture(db, resource_id).map_err(Halt::Data)?;
        let index = cursor;
        cursor += 1;
        let Some((_, body)) = cache
            .rows
            .get(index)
            .filter(|(old, _)| old.as_ref() == &capture)
        else {
            cache.rows.truncate(index);
            return Err(Halt::Project(Arc::new(capture)));
        };
        let mut source = super::super::catalog::source(resource_id, "metadata", body.clone());
        source.source_id = entry.identity.source_id.clone();
        Ok(source)
    });
    Ok(match result {
        Ok(value) => Ok(Step::Complete(value)),
        Err(Halt::Resources) => Ok(Step::Resources),
        Err(Halt::WritingResources) => {
            cache.writing = true;
            Ok(Step::Writing)
        }
        Err(Halt::Writing(flow)) => return Err(flow),
        Err(Halt::Project(capture)) => Ok(Step::Project(capture)),
        Err(Halt::Context(error)) => Err(context_error(error)),
        Err(Halt::Data(failure)) => {
            Err(SourceReadError::Context(failure.map_error(
                |error| match error {
                    PaperMetadataError::SourceUnavailable => Error::SourceUnavailable,
                    PaperMetadataError::Unavailable => Error::Unavailable,
                },
            )))
        }
    })
}

fn context_error(error: Error) -> SourceReadError {
    SourceReadError::Context(Failure::original(error))
}
