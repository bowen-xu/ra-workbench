//! Live application resources over the existing list/read/search and runner contracts.
use super::{
    Error, resource_grants,
    sources::{self, Resource},
};
use crate::{
    agent::{composed_tools::ComposedToolExecutor, source_tools},
    storage::AppStore,
};
use research_agent_core::{
    model::{ToolCall, ToolDefinition},
    runner::{RunError, RunFuture, ToolExecutor, ToolOutput},
    submission::{ContextText, SubmissionRecord},
    tool_registry::OwnedToolRegistration,
};
use rusqlite::{Connection, params};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;
use uuid::Uuid;
#[path = "resource_tools/directory.rs"]
mod directory;
#[path = "resource_tools/discovery.rs"]
mod discovery;
#[path = "resource_tools/evidence.rs"]
mod evidence;
#[path = "resource_tools/metadata_runtime.rs"]
pub(crate) mod metadata_runtime;
#[path = "resource_tools/structured_runtime.rs"]
pub(crate) mod structured_runtime;
#[path = "resource_tools/writing_runtime.rs"]
mod writing_runtime;
pub(crate) use evidence::{FrozenReadingEvidence, frozen_source_snapshot_with, frozen_view_with};
// Preserve the stable codec diagnosis, without exposing unrelated raw errors.
fn note_read_error(reason: String, page_id: String, request_id: Uuid) -> Error {
    if reason.starts_with("MODEL_NOTE_TEXT_VIEW_UNSTABLE:") {
        Error::NoteTextViewUnstable {
            page_id,
            request_id,
        }
    } else {
        Error::SourceUnavailable
    }
}
#[path = "resource_tools/file_handle.rs"]
mod file_handle;
#[path = "resource_tools/preparation.rs"]
mod preparation;
#[path = "resource_tools/source_link.rs"]
pub(crate) mod source_link;

pub(crate) fn registration(
    store: AppStore,
    record: SubmissionRecord,
    existing: OwnedToolRegistration,
) -> OwnedToolRegistration {
    registration_inner(
        store,
        record,
        existing,
        #[cfg(target_os = "macos")]
        None,
    )
}

#[cfg(target_os = "macos")]
pub(crate) fn registration_with_workspace(
    store: AppStore,
    record: SubmissionRecord,
    existing: OwnedToolRegistration,
    workspace: crate::agent::text_tools::file_workspace::FileWorkspaceHandle,
) -> OwnedToolRegistration {
    registration_inner(store, record, existing, workspace)
}

fn registration_inner(
    store: AppStore,
    record: SubmissionRecord,
    existing: OwnedToolRegistration,
    #[cfg(target_os = "macos")]
    workspace: crate::agent::text_tools::file_workspace::FileWorkspaceHandle,
) -> OwnedToolRegistration {
    if !record
        .input
        .context
        .iter()
        .any(|source| resource_grants::resource(source).is_some())
    {
        return text_registration(
            store,
            record,
            existing,
            #[cfg(target_os = "macos")]
            workspace,
        );
    }
    let tools = Arc::new(ResourceTools {
        store: store.clone(),
        record: record.clone(),
        scope: None,
    });
    let (mut previous, executor) = existing.into_parts();
    let own = tools.definitions();
    previous.retain(|definition| {
        !own.iter()
            .any(|tool| tool.name == definition.name && tool.namespace == definition.namespace)
    });
    let mut definitions = previous.clone();
    definitions.extend(own.clone());
    text_registration(
        store,
        record,
        OwnedToolRegistration::new(
            definitions,
            Arc::new(ComposedToolExecutor::new(
                tools,
                own,
                Some(executor),
                previous,
            )),
        ),
        #[cfg(target_os = "macos")]
        workspace,
    )
}

fn text_registration(
    store: AppStore,
    record: SubmissionRecord,
    existing: OwnedToolRegistration,
    #[cfg(target_os = "macos")]
    workspace: crate::agent::text_tools::file_workspace::FileWorkspaceHandle,
) -> OwnedToolRegistration {
    #[cfg(target_os = "macos")]
    if let Some(workspace) = workspace {
        return crate::agent::text_tools::registration_with_workspace(
            store,
            record,
            existing,
            Some(workspace),
        );
    }
    crate::agent::text_tools::registration(store, record, existing)
}
#[derive(Clone)]
struct ResourceTools {
    store: AppStore,
    record: SubmissionRecord,
    scope: Option<crate::agent::writing_resources::OwnedWritingScope>,
}
pub(crate) fn note_access(
    store: &AppStore,
    record: &SubmissionRecord,
    db: &Connection,
    page: &str,
    write: bool,
    scope: Option<crate::agent::writing_resources::OwnedWritingScope>,
) -> Result<bool, String> {
    let tools = ResourceTools {
        store: store.clone(),
        record: record.clone(),
        scope,
    };
    let entries = tools.entries(db).map_err(|error| error.to_string())?;
    Ok(entries.iter().any(|entry| {
        matches!(&entry.resource,Some(Resource::Note{resource_id}) if resource_id.to_string()==page)
            && (!write
                || entry
                    .identity
                    .anchor
                    .as_deref()
                    .and_then(|value| serde_json::from_str::<Value>(value).ok())
                    .is_some_and(|anchor| anchor["contextAccess"] == "readWrite"))
    }))
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Read {
    source_id: Option<String>,
    file_id: Option<String>,
    #[serde(default)]
    start_char: usize,
    #[serde(default = "max_chars")]
    max_chars: usize,
    view: Option<String>,
}
fn max_chars() -> usize {
    8000
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Search {
    query: String,
    source_id: Option<String>,
    file_id: Option<String>,
    #[serde(default)]
    offset: usize,
    resource_id: Option<String>,
    name_query: Option<String>,
    path_glob: Option<String>,
    resource_type: Option<String>,
}
#[derive(Clone)]
struct Entry {
    identity: ContextText,
    resource: Option<Resource>,
    owner: Option<String>,
}
impl ResourceTools {
    fn writing_scope(&self) -> Option<crate::agent::writing_resources::WritingScope<'_>> {
        self.scope
            .as_deref()
            .map(crate::agent::writing_resources::WritingScope::Native)
            .or_else(|| {
                self.store
                    .writing_resource_source()
                    .map(crate::agent::writing_resources::WritingScope::Static)
            })
    }

    fn definitions(&self) -> Vec<ToolDefinition> {
        let mut definitions = source_tools::definitions(&self.record);
        for tool in &mut definitions {
            match tool.name.as_str() {
                "research_list_sources" => {
                    *tool = discovery::definition(false);
                }
                "research_read_source" => {
                    tool.description="Read current authorized content by fileId from the source prompt/directory, or by the listed sourceId for a precise scope. Exactly one handle is required. Offsets count Unicode characters; maxChars defaults to 8000, accepts 1–8000; follow nextChar. Re-read when content may have changed. PDF reads automatically prepare missing current-version Docling content and wait for it; saved parses are reused. Results remain in conversation history.".into();
                    tool.parameters["properties"]["fileId"] = file_handle::schema();
                    tool.parameters["properties"]["view"] = json!({"type":"string","enum":["raw","structured"],"description":"PDF default raw returns current Docling. structured reads normalized evidence with stable evidenceItemId values for writing portraits/structured reading; no authority is added."});
                    if let Some(parameters) = tool.parameters.as_object_mut() {
                        parameters.remove("required");
                    }
                }
                "research_search_sources" => {
                    tool.description="Search current authorized CONTENT for a case-sensitive literal query; 20 matching snippets per page. Narrow with fileId or sourceId, resourceId, nameQuery, pathGlob or resourceType. Matching PDFs automatically prepare missing current-version Docling content; saved parses are reused. For NAME/PATH discovery use research_find_sources. Scan budget is 8 MiB; narrow the search on a budget error.".into();
                    tool.parameters["properties"]["fileId"] = file_handle::schema();
                    for (key, schema) in discovery::filter_properties() {
                        tool.parameters["properties"][key] = schema;
                    }
                }
                _ => {}
            }
        }
        definitions.push(discovery::definition(true));
        definitions
    }
    fn read(&self, db: &Connection, entry: &Entry) -> Result<ContextText, Error> {
        if let Some(Resource::StructuredObject { resource_id, .. }) = &entry.resource {
            let snapshot = crate::agent::text_tools::read_structured_view(
                &self.store,
                &self.record,
                db,
                resource_id,
            )
            .map_err(|_| Error::SourceUnavailable)?;
            let mut source = entry.identity.clone();
            source.text = snapshot.text;
            source.revision = snapshot.version;
            return Ok(source);
        }
        if crate::agent::text_tools::current_contract(db, &self.record)
            && let Some(Resource::Note { resource_id }) = &entry.resource
        {
            let snapshot = crate::agent::text_tools::read_note_view(
                &self.store,
                &self.record,
                db,
                &resource_id.to_string(),
                self.scope.clone(),
            )
            .map_err(|reason| {
                note_read_error(reason, resource_id.to_string(), self.record.request_id)
            })?;
            let mut source = entry.identity.clone();
            source.text = snapshot.text;
            source.revision = snapshot.version;
            return Ok(source);
        }
        if let Some(resource) = &entry.resource {
            let mut sources =
                sources::materialize(db, &self.store, std::slice::from_ref(resource))?;
            if sources.len() != 1 {
                return Err(Error::SourceUnavailable);
            }
            let mut source = sources.remove(0);
            source.source_id = entry.identity.source_id.clone();
            Ok(source)
        } else {
            Ok(entry.identity.clone())
        }
    }
    fn evaluate(&self, call: &ToolCall) -> Result<Value, Error> {
        let mut db = self.store.connection().map_err(|_| Error::Unavailable)?;
        self.evaluate_with(&mut db, call, &mut |db, entry| self.read(db, entry))
    }
    /// The same ordered evaluator serves static reads and finite Native metadata.
    /// Only the metadata body producer changes; tools and other resource owners do not.
    fn evaluate_with<E: From<Error>>(
        &self,
        db: &mut Connection,
        call: &ToolCall,
        read: &mut impl FnMut(&Connection, &Entry) -> Result<ContextText, E>,
    ) -> Result<Value, E> {
        let entries = self.selected_entries(db, call)?;
        match call.name.as_str() {
            "research_list_sources" | "research_find_sources" => {
                discovery::evaluate(self, db, entries, call).map_err(E::from)
            }
            "research_read_source" => {
                let args: Read = serde_json::from_value(call.arguments.clone())
                    .map_err(|_| Error::InvalidInput)?;
                let entry = file_handle::find(
                    &entries,
                    args.source_id.as_deref(),
                    args.file_id.as_deref(),
                )?;
                let source = if args.view.as_deref() == Some("structured") {
                    self.structured_pdf_source(db, entry)?
                } else {
                    read(db, entry)?
                };
                let mut record = self.record.clone();
                record.input.context = vec![source];
                let mut request = call.clone();
                request.arguments = json!({"sourceId":entry.identity.source_id,"startChar":args.start_char,"maxChars":args.max_chars});
                source_tools::evaluate(&record, &request)
                    .map(without_versions)
                    .map(|mut result| {
                        result["name"] = json!(discovery::name(entry));
                        if let Some(address) = source_link::address(entry) {
                            result["sourceUrl"] = json!(address);
                        }
                        if let Some(id) = file_handle::id(entry) {
                            result["fileId"] = json!(id);
                        }
                        result
                    })
                    .map_err(|_| E::from(Error::InvalidInput))
            }
            "research_search_sources" => {
                let args: Search = serde_json::from_value(call.arguments.clone())
                    .map_err(|_| Error::InvalidInput)?;
                if args.query.is_empty() || args.query.len() > 512 {
                    return Err(E::from(Error::InvalidInput));
                }
                let source_id = if args.source_id.is_some() || args.file_id.is_some() {
                    Some(
                        entries
                            .first()
                            .ok_or(Error::SourceUnavailable)?
                            .identity
                            .source_id
                            .clone(),
                    )
                } else {
                    None
                };
                let chosen = entries.iter();
                let mut context = Vec::new();
                let mut bytes = 0;
                for entry in chosen {
                    let source = read(db, entry)?;
                    bytes += source.text.len();
                    if bytes > 8 * 1024 * 1024 {
                        return Err(E::from(Error::Budget));
                    }
                    context.push(source);
                }
                if source_id.is_some() && context.is_empty() {
                    return Err(E::from(Error::SourceUnavailable));
                }
                let scope = source_id.as_ref().and_then(|_| entries.first()).map(|entry| {
                    json!({"name":discovery::name(entry),"sourceUrl":source_link::address(entry)})
                });
                let mut record = self.record.clone();
                record.input.context = context;
                let mut request = call.clone();
                request.arguments =
                    json!({"query":args.query,"sourceId":source_id,"offset":args.offset});
                source_tools::evaluate(&record, &request)
                    .map(without_versions)
                    .map(|mut result| {
                        if let Some(scope) = scope {
                            result["scope"] = scope;
                        }
                        if let Some(matches) =
                            result.get_mut("matches").and_then(Value::as_array_mut)
                        {
                            for matched in matches {
                                if let Some(entry) = entries
                                    .iter()
                                    .find(|entry| matched["sourceId"] == entry.identity.source_id)
                                    && let Some(address) = source_link::address(entry)
                                {
                                    matched["sourceUrl"] = json!(address);
                                }
                            }
                        }
                        result
                    })
                    .map_err(|_| E::from(Error::InvalidInput))
                    .and_then(|value| discovery::bound_matches(value, args.offset).map_err(E::from))
            }
            _ => Err(E::from(Error::InvalidInput)),
        }
    }
}
fn without_versions(mut value: Value) -> Value {
    if let Some(object) = value.as_object_mut() {
        object.remove("revision");
        object.remove("contextSnapshotId");
    }
    if let Some(matches) = value.get_mut("matches").and_then(Value::as_array_mut) {
        for matched in matches {
            if let Some(object) = matched.as_object_mut() {
                object.remove("revision");
            }
        }
    }
    value
}
impl ToolExecutor for ResourceTools {
    fn requires_result_receipt(&self, call: &ToolCall) -> bool {
        // This owner uses the trait's no-op result_committed; all resource
        // preparation and reads have already joined before returning output.
        !matches!(
            call.name.as_str(),
            "research_list_sources"
                | "research_find_sources"
                | "research_read_source"
                | "research_search_sources"
        )
    }
    fn cancellation_safe_to_drop(&self, call: &ToolCall) -> bool {
        // Only raw PDF reads/searches use the existing shared preparation owner:
        // dropping its waiter detaches, and the last waiter cancels its parse.
        // Structured views can persist read-scope/evidence state; other resource
        // kinds and editor-backed access retain their conservative owner policy.
        let raw_read = call.name == "research_read_source"
            && call
                .arguments
                .get("view")
                .and_then(Value::as_str)
                .is_none_or(|view| view == "raw");
        (raw_read || call.name == "research_search_sources")
            && !self.record.input.context.is_empty()
            && self.record.input.context.iter().all(|source| {
                matches!(
                    resource_grants::resource(source),
                    Some(Resource::Paper { .. })
                )
            })
    }
    fn validate(&self, call: &ToolCall) -> Result<(), RunError> {
        if matches!(
            call.name.as_str(),
            "research_list_sources" | "research_find_sources"
        ) {
            return discovery::validate(call);
        }
        if call.name == "research_read_source" {
            let args: Read = crate::agent::tool_arguments::decode_object(
                &call.arguments,
                "Provide fileId or sourceId, optional startChar and maxChars (1–8000); no extra fields.",
            )?;
            if args
                .view
                .as_deref()
                .is_some_and(|view| !matches!(view, "raw" | "structured"))
            {
                return Err(RunError::ToolArguments(
                    "view must be raw or structured".into(),
                ));
            }
            file_handle::validate(args.source_id.as_deref(), args.file_id.as_deref(), true)?;
            let mut request = call.clone();
            request.arguments = json!({"sourceId":args.source_id.or(args.file_id),"startChar":args.start_char,"maxChars":args.max_chars});
            return source_tools::validate_arguments(&request);
        }
        if call.name == "research_search_sources" {
            let args: Search = crate::agent::tool_arguments::decode_object(
                &call.arguments,
                "Provide query, optional fileId or sourceId, resourceId, nameQuery, pathGlob, resourceType and offset; no extra fields.",
            )?;
            file_handle::validate(args.source_id.as_deref(), args.file_id.as_deref(), false)?;
            discovery::validate_filters(&discovery::Filters {
                resource_id: args.resource_id,
                name_query: args.name_query,
                path_glob: args.path_glob,
                resource_type: args.resource_type,
            })?;
            let mut request = call.clone();
            request.arguments = json!({"query":args.query,"sourceId":args.source_id.or(args.file_id),"offset":args.offset});
            return source_tools::validate_arguments(&request);
        }
        source_tools::validate_arguments(call)
    }
    fn authorize_and_execute(&self, call: ToolCall) -> RunFuture<'_, ToolOutput> {
        Box::pin(async move {
            self.validate(&call)?;
            let mut tools = self.clone();
            tools.scope = crate::agent::writing_resources::native_scope_for_context(
                &tools.store,
                &tools.record,
            )
            .await;
            let result = match tools.prepare_content(&call).await {
                Ok(()) => {
                    if let Some(output) = tools.runtime_structured_read(&call).await {
                        return Ok(output);
                    }
                    if let Some(output) = tools.runtime_metadata_read(&call).await {
                        return Ok(output);
                    }
                    if let Some(output) = tools.runtime_writing_read(&call).await {
                        return Ok(output);
                    }
                    tokio::task::spawn_blocking(move || tools.evaluate(&call))
                        .await
                        .map_err(|_| RunError::ToolUnavailable)?
                }
                Err(error) => Err(error),
            };
            Ok(context_output(result))
        })
    }
}
fn context_output(result: Result<Value, Error>) -> ToolOutput {
    let (content,is_error)=match result {Ok(value)=>(value.to_string(),false),Err(error)=>(json!({"error":error.to_string(),"guidance":match &error {
        Error::PaperParseFailed=>"Automatic Docling preparation failed. Check the local Docling service and retry; never use the original PDF as a fallback.",
        Error::NoteTextViewUnstable{..}=>"The note text codec could not preserve its content. Preserve editor input and report this diagnostic; refreshing or reducing maxChars will not repair it.",
        _=>"Re-list resources or re-read current authorized content; never use the original PDF as a fallback."
    }}).to_string(),true)};
    ToolOutput { content, is_error }
}
#[cfg(test)]
#[path = "resource_tools/tests.rs"]
mod tests;
