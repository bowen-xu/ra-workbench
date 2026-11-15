//! One registered text-tool owner over approved files and note authoring views.
#[path = "addresses.rs"]
mod addresses;
#[path = "apply.rs"]
mod apply;
#[path = "backends.rs"]
mod backends;
#[path = "decision_requests.rs"]
mod decision_requests;
#[path = "decisions.rs"]
pub(crate) mod decisions;
#[path = "delivery.rs"]
mod delivery;
#[path = "editor_barrier.rs"]
pub(crate) mod editor_barrier;
#[path = "file.rs"]
mod file;
#[cfg(target_os = "macos")]
#[path = "file_workspace.rs"]
pub(crate) mod file_workspace;
#[path = "journal.rs"]
pub(crate) mod journal;
#[path = "legacy.rs"]
mod legacy;
#[path = "note.rs"]
mod note;
#[path = "note_delivery.rs"]
mod note_delivery;
#[path = "patch_diagnostic.rs"]
mod patch_diagnostic;
#[path = "patch_journal.rs"]
mod patch_journal;
#[path = "patch_preview.rs"]
mod patch_preview;
#[path = "structured.rs"]
mod structured;
#[path = "structured_freeze.rs"]
mod structured_freeze;
pub(crate) use structured::read_view as read_structured_view;
#[path = "note_review/mod.rs"]
pub(crate) mod note_review;
#[cfg(test)]
#[path = "tests.rs"]
mod tests;
use crate::storage::AppStore;
use research_agent_core::{
    model::{ToolCall, ToolDefinition},
    runner::{RunError, RunFuture, ToolExecutor, ToolNameResolution, ToolOutput},
    submission::SubmissionRecord,
    text_patch::{
        self, Hunk,
        backend::{
            BackendFuture, TargetResult, TargetStatus, TextBackend, TextChange, TextSnapshot,
        },
    },
    tool_registry::OwnedToolRegistration,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use uuid::Uuid;

pub(super) const READ: &str = "research_read_file";
pub(super) const PATCH: &str = "research_apply_patch";
pub(crate) struct TextTools {
    store: AppStore,
    record: SubmissionRecord,
    previous: Arc<dyn ToolExecutor>,
    previous_names: std::collections::HashSet<String>,
    previous_qualified_names: std::collections::HashSet<(String, String)>,
    delivery_attempted: std::sync::atomic::AtomicBool,
    delivery_sealed: std::sync::atomic::AtomicBool,
    delivery_lease: std::sync::Mutex<Option<Uuid>>,
    cancellation: crate::agent::cancellation::AgentCancellation,
    #[cfg(target_os = "macos")]
    file_workspace: file_workspace::FileWorkspaceHandle,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Read {
    path: String,
    #[serde(default)]
    start_char: usize,
    #[serde(default = "read_limit")]
    max_chars: usize,
    #[serde(default)]
    view: Option<String>,
}
fn read_limit() -> usize {
    4000
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Patch {
    patch: String,
}

#[path = "registration.rs"]
mod registration;
pub(crate) use registration::registration;
#[cfg(target_os = "macos")]
pub(crate) use registration::registration_with_workspace;

impl TextTools {
    fn qualified_previous(&self, namespace: &str, name: &str) -> bool {
        self.previous_qualified_names
            .contains(&(namespace.to_owned(), name.to_owned()))
            && self.previous.resolve_qualified_name(Some(namespace), name)
                == ToolNameResolution::Registered
    }
    fn version(&self) -> Option<i64> {
        let db = self.store.connection().ok()?;
        let present: bool = db.query_row("select exists(select 1 from sqlite_master where type='table' and name='agent_text_tool_contracts')", [], |row| row.get(0)).ok()?;
        if !present {
            return Some(1);
        }
        db.query_row("select version from agent_text_tool_contracts where conversation_id=?1 and request_id=?2",rusqlite::params![self.record.conversation_id.to_string(),self.record.request_id.to_string()],|row|row.get(0)).ok()
    }
    fn definitions(&self) -> Vec<ToolDefinition> {
        let note_syntax =
            if self.record.input.context.iter().any(|source| {
                source.kind == research_agent_core::submission::ContextSourceKind::Note
            }) {
                crate::notes::text_view::INSTRUCTIONS
            } else {
                ""
            };
        let mut definitions = vec![
        ToolDefinition{namespace: None,name:READ.into(),description:format!("Read a registered source://<id-path> address returned by source discovery or resource references. Reuse existing sourceUrl values; do not invent a mount or append .md to notes. name is the original resource title/filename, not identity. Offsets count Unicode characters; follow nextChar. Read before editing. Notes read current proposals by default; view=saved reads the saved baseline. {note_syntax}"),parameters:json!({"type":"object","properties":{"path":{"type":"string"},"startChar":{"type":"integer","minimum":0},"maxChars":{"type":"integer","minimum":1,"maximum":8000},"view":{"enum":["saved","candidate"]}},"required":["path"],"additionalProperties":false})},
        ToolDefinition{namespace: None,name:PATCH.into(),description:"Apply Codex text patch syntax: *** Begin Patch, Add File/Update File/Delete File, optional Move to, @@ context, - old/+ new/space context, *** End Patch. Reuse registered source://<id-path> addresses from read/discovery, with original titles/filenames and no artificial .md suffix. For notes, Add File addresses the authorized parent note and creates a child with title frontmatter; the tool returns its allocated source address. Move to addresses the existing parent note source://notes/<parent-id>, or source://notes/ to move to root; the moved note keeps its source://notes/<page-id> identity. Exact UNIQUE matching only; add context on ambiguity. Syntax failure changes nothing. Mixed targets may partially succeed: inspect every result. applied means the target changed: an authorized file copy, or a note saved with its review baseline. Pending note Diff remains until user Keep/Undo; saving never implies Keep. Historical proposed results retain their deferred-save contract. Built-in source://agents/prompts/... and source://agents/skills/... documents are read-only until the user approves this exact Update in the composer panel; never create/delete/move them or treat a mention as permission. Their saved content is active immediately; Keep is review only. Retry the original request identity after unknown outcomes, never blindly replay writes.".into(),parameters:json!({"type":"object","properties":{"patch":{"type":"string","maxLength":text_patch::MAX_PATCH_BYTES}},"required":["patch"],"additionalProperties":false})}
    ];
        if self.file_backend().receipt().is_ok() {
            definitions.push(ToolDefinition { namespace: None, name: crate::agent::writing_delivery_tool::NAME.into(),
                description: "Deliver the frozen execution-copy changes through the manuscript's file owner. Called automatically before natural completion when files changed. Inspect the result before claiming files were saved. A failure or unknown result may include partial writes; never replay patches or delivery. After confirmed delivery, further edits use a fresh execution copy of the current files.".into(),
                parameters: json!({"type":"object","properties":{},"additionalProperties":false}) });
        }
        definitions
    }
    async fn read(&self, args: Read) -> Result<Value, String> {
        if !(1..=8000).contains(&args.max_chars)
            || args
                .view
                .as_deref()
                .is_some_and(|value| value != "saved" && value != "candidate")
        {
            return Err("invalid read window or view".into());
        }
        let args = Read {
            path: if args.path.starts_with("source://") {
                text_patch::address::format(&text_patch::address::parse(&args.path)?)
            } else {
                args.path
            },
            ..args
        };
        let scope = if self.is_note_path(&args.path) {
            crate::agent::writing_resources::native_scope_for_context(&self.store, &self.record)
                .await
        } else {
            None
        };
        let targets = if self.is_note_path(&args.path) {
            if args.view.as_deref() == Some("saved") && args.path.starts_with("source://") {
                let db = self.store.connection().map_err(|error| error.to_string())?;
                let addresses::Target::Note { page, .. } =
                    addresses::resolve_saved(&db, &self.record, &args.path)?
                else {
                    return Err("source is not a saved note".into());
                };
                Some(std::collections::BTreeSet::from([page]))
            } else {
                Some(
                    note::NoteBackend {
                        scope: scope.clone(),
                        store: self.store.clone(),
                        record: self.record.clone(),
                    }
                    .editor_targets(std::slice::from_ref(&args.path), false)?,
                )
            }
        } else {
            None
        };
        let _editor_guard = if let Some(targets) = targets {
            Some(
                self.store
                    .text_editors()
                    .acquire_pages(Some(&targets))
                    .await?,
            )
        } else {
            None
        };
        let snapshot = self
            .backend_with_scope(&args.path, scope)?
            .read(&args.path, args.view.as_deref() == Some("saved"), true)
            .await?;
        let total = snapshot.text.chars().count();
        let text = snapshot
            .text
            .chars()
            .skip(args.start_char)
            .take(args.max_chars)
            .collect::<String>();
        if args.start_char > total {
            return Err("offset is outside the text; read from zero".into());
        }
        let end = args.start_char + text.chars().count();
        // Offsets count Unicode characters; line numbers describe this exact text view.
        let start_line = snapshot
            .text
            .chars()
            .take(args.start_char)
            .filter(|c| *c == '\n')
            .count()
            + 1;
        let end_line = start_line + text.chars().filter(|c| *c == '\n').count()
            - usize::from(text.ends_with('\n'));
        let is_note = self.is_note_path(&snapshot.path);
        let page = snapshot.path.rsplit('/').next().unwrap_or("");
        let selection = self.record.input.context.iter().any(|source| {
            source
                .anchor
                .as_deref()
                .and_then(|value| serde_json::from_str::<Value>(value).ok())
                .is_some_and(|anchor| anchor["pageId"] == page && anchor["blockId"].is_string())
        });
        let saved = args.view.as_deref() == Some("saved");
        let mut result = json!({"path":snapshot.path,"name":snapshot.name,"version":snapshot.version,"textViewVersion":is_note.then(crate::notes::text_view::version),"textFileScope":is_note.then_some(if selection {"selection"} else {"full"}),"text":text,"startChar":args.start_char,"nextChar":(end<total).then_some(end),"view":args.view.unwrap_or("candidate".into())});
        if !text.is_empty() {
            result["startLine"] = json!(start_line);
            result["endLine"] = json!(end_line);
        }
        structured::read_metadata(&self.store, &self.record, &args.path, saved, &mut result)?;
        Ok(result)
    }
    fn is_note_path(&self, path: &str) -> bool {
        addresses::is_note(path)
    }
}
impl ToolExecutor for TextTools {
    fn child_creation_intent(
        &self,
        call: &ToolCall,
    ) -> Option<research_agent_core::conversation::CreateBoundConversation> {
        if let Some(namespace) = &call.namespace {
            if !self.qualified_previous(namespace, &call.name) {
                return None;
            }
            return self.previous.child_creation_intent(call);
        }
        if call.name != crate::agent::writing_delivery_tool::NAME
            && call.name != READ
            && (call.name != PATCH || call.arguments.get("patch").is_none())
        {
            self.previous.child_creation_intent(call)
        } else {
            None
        }
    }
    fn resolve_name(&self, name: &str) -> ToolNameResolution {
        if matches!(
            name,
            READ | PATCH | crate::agent::writing_delivery_tool::NAME
        ) {
            ToolNameResolution::Registered
        } else if self.previous_names.contains(name) {
            self.previous.resolve_name(name)
        } else {
            ToolNameResolution::NotFound
        }
    }
    fn resolve_qualified_name(&self, namespace: Option<&str>, name: &str) -> ToolNameResolution {
        match namespace {
            Some(value) if self.qualified_previous(value, name) => ToolNameResolution::Registered,
            Some(_) => ToolNameResolution::NotFound,
            None => self.resolve_name(name),
        }
    }
    fn cancel_pending(&self, call: ToolCall) -> RunFuture<'_, ()> {
        if let Some(namespace) = &call.namespace {
            if !self.qualified_previous(namespace, &call.name) {
                return Box::pin(async { Err(RunError::ToolNotFound) });
            }
            return self.previous.cancel_pending(call);
        }
        self.cancellation.cancel();
        if self.previous_names.contains(&call.name) {
            self.previous.cancel_pending(call)
        } else {
            Box::pin(async { Ok(()) })
        }
    }
    fn supports_parallel(&self, call: &ToolCall) -> bool {
        if let Some(namespace) = &call.namespace {
            if !self.qualified_previous(namespace, &call.name) {
                return false;
            }
            return self.previous.supports_parallel(call);
        }
        // Even reads may take the live editor gate; keep text and source reads
        // exclusive with patch/delivery rather than infer safety from a name.
        !matches!(
            call.name.as_str(),
            READ | PATCH | "research_read_source" | "research_search_sources"
        ) && call.name != crate::agent::writing_delivery_tool::NAME
            && self.previous_names.contains(&call.name)
            && self.previous.supports_parallel(call)
    }
    fn cancellation_safe_to_drop(&self, call: &ToolCall) -> bool {
        if let Some(namespace) = &call.namespace {
            if !self.qualified_previous(namespace, &call.name) {
                return false;
            }
            return self.previous.cancellation_safe_to_drop(call);
        }
        // Patches and delivery must finish their commit/acknowledgement. Source
        // reads with live note context can also own the editor gate. Only a
        // separately proven inner policy can opt a paper-only parse waiter in.
        !(matches!(call.name.as_str(), READ | PATCH)
            || (matches!(
                call.name.as_str(),
                "research_read_source" | "research_search_sources"
            ) && self.record.input.context.iter().any(|source| {
                source.kind == research_agent_core::submission::ContextSourceKind::Note
            })))
            && call.name != crate::agent::writing_delivery_tool::NAME
            && self.previous_names.contains(&call.name)
            && self.previous.cancellation_safe_to_drop(call)
    }
    fn finalization_call(&self) -> RunFuture<'_, Option<ToolCall>> {
        self.delivery_call()
    }
    fn release_for_finalization(&self) -> RunFuture<'_, bool> {
        self.previous.release_for_finalization()
    }
    fn result_committed(&self, call: ToolCall) -> RunFuture<'_, ()> {
        if let Some(namespace) = &call.namespace {
            if !self.qualified_previous(namespace, &call.name) {
                return Box::pin(async { Err(RunError::ToolNotFound) });
            }
            return self.previous.result_committed(call);
        }
        if call.name == crate::agent::writing_delivery_tool::NAME {
            return self.delivery_result_committed(call);
        }
        if call.name != READ && (call.name != PATCH || call.arguments.get("patch").is_none()) {
            self.previous.result_committed(call)
        } else {
            Box::pin(async { Ok(()) })
        }
    }
    fn requires_result_receipt(&self, call: &ToolCall) -> bool {
        if let Some(namespace) = &call.namespace {
            if !self.qualified_previous(namespace, &call.name) {
                return true;
            }
            return self.previous.requires_result_receipt(call);
        }
        if call.name != crate::agent::writing_delivery_tool::NAME
            && call.name != READ
            && (call.name != PATCH || call.arguments.get("patch").is_none())
        {
            self.previous.requires_result_receipt(call)
        } else {
            true
        }
    }
    fn validate(&self, call: &ToolCall) -> Result<(), RunError> {
        if let Some(namespace) = &call.namespace {
            if !self.qualified_previous(namespace, &call.name) {
                return Err(RunError::ToolNotFound);
            }
            return self.previous.validate(call);
        }
        if matches!(call.name.as_str(), READ | PATCH)
            && (call.call_id.is_empty() || call.call_id.len() > 512)
        {
            return Err(RunError::ToolArguments(
                "invalid text tool call identity".into(),
            ));
        }
        match call.name.as_str() {
            crate::agent::writing_delivery_tool::NAME => {
                if call
                    .arguments
                    .as_object()
                    .is_some_and(|object| object.is_empty())
                {
                    Ok(())
                } else {
                    Err(RunError::ToolArguments(
                        "delivery takes no arguments".into(),
                    ))
                }
            }
            READ => {
                if self.version() != Some(2) {
                    return Err(RunError::ToolRejected);
                }
                crate::agent::tool_arguments::decode_object::<Read>(
                    &call.arguments,
                    "provide path and optional read window/view",
                )?;
                Ok(())
            }
            PATCH if call.arguments.get("patch").is_some() => {
                if self.version() != Some(2) {
                    return Err(RunError::ToolArguments(
                        "this restored turn requires the version-one patch contract".into(),
                    ));
                }
                let args: Patch = crate::agent::tool_arguments::decode_object(
                    &call.arguments,
                    "provide only patch text",
                )?;
                text_patch::parse(&args.patch)
                    .map_err(|error| RunError::ToolArguments(error.to_string()))?;
                Ok(())
            }
            PATCH => {
                if self.version() == Some(1) && self.previous_names.contains(PATCH) {
                    self.previous.validate(call)
                } else {
                    Err(RunError::ToolArguments(
                        "new calls require {patch: string}".into(),
                    ))
                }
            }
            _ if self.previous_names.contains(&call.name) => self.previous.validate(call),
            _ => Err(RunError::ToolRejected),
        }
    }
    fn authorize_and_execute(&self, call: ToolCall) -> RunFuture<'_, ToolOutput> {
        if let Some(namespace) = &call.namespace {
            if !self.qualified_previous(namespace, &call.name) {
                return Box::pin(async { Err(RunError::ToolNotFound) });
            }
            return self.previous.authorize_and_execute(call);
        }
        Box::pin(async move {
            self.validate(&call)?;
            #[cfg(target_os = "macos")]
            self.ensure_file_owner_for_call(&call).await?;
            if call.name == crate::agent::writing_delivery_tool::NAME {
                return self.deliver_files(&call.call_id).await;
            }
            if call.name == PATCH && call.arguments.get("patch").is_none() {
                return self.file_backend().legacy(&call).await;
            }
            if call.name != READ && call.name != PATCH {
                let _guard = if self.version() == Some(2)
                    && matches!(
                        call.name.as_str(),
                        "research_read_source" | "research_search_sources"
                    )
                    && self.record.input.context.iter().any(|source| {
                        source.kind == research_agent_core::submission::ContextSourceKind::Note
                    }) {
                    Some(
                        self.store
                            .text_editors()
                            .acquire(None)
                            .await
                            .map_err(RunError::ToolArguments)?,
                    )
                } else {
                    None
                };
                let mut output = self.previous.authorize_and_execute(call).await?;
                if self.version() == Some(2)
                    && let Ok(mut value) = serde_json::from_str::<Value>(&output.content)
                {
                    public_output(&mut value);
                    output.content = value.to_string();
                }
                return Ok(output);
            }
            let result = if call.name == READ {
                self.read(
                    serde_json::from_value(call.arguments).map_err(|_| RunError::ToolRejected)?,
                )
                .await
            } else {
                let args: Patch = serde_json::from_value(call.arguments.clone())
                    .map_err(|_| RunError::ToolRejected)?;
                self.apply(&call.call_id, &args.patch).await
            };
            if call.name == PATCH
                && let Ok(value) = &result
            {
                // Only the owned patch journal/backend produces this typed
                // target list. Keep unknown effects out of model retry feedback.
                let targets = structured::decode_targets(value["targets"].clone())
                    .map_err(|_| RunError::EventSink)?;
                if value["status"] == "unknown"
                    || targets
                        .iter()
                        .any(|target| target.status == TargetStatus::Unknown)
                {
                    return Err(patch_diagnostic::outcome_error(
                        &self.record,
                        value,
                        &targets,
                    ));
                }
            }
            Ok(match result {
                Ok(result) => ToolOutput {
                    is_error: result["status"] == "failed"
                        || result["status"] == "declined"
                        || result["status"] == "unknown"
                        || result["status"] == "partial",
                    content: result.to_string(),
                },
                Err(error) => ToolOutput {
                    content: json!({"status":"failed","message":error}).to_string(),
                    is_error: true,
                },
            })
        })
    }
}
pub(crate) fn read_note_view(
    store: &AppStore,
    record: &SubmissionRecord,
    db: &rusqlite::Connection,
    page: &str,
    scope: Option<crate::agent::writing_resources::OwnedWritingScope>,
) -> Result<TextSnapshot, String> {
    let saved = note::load(db, page)?;
    let path = note::canonical(&saved);
    let snapshot = note::NoteBackend {
        scope,
        store: store.clone(),
        record: record.clone(),
    }
    .snapshot(db, &path, false)?;
    journal::remember(db, record, &snapshot)?;
    Ok(snapshot)
}
pub(crate) fn current_contract(db: &rusqlite::Connection, record: &SubmissionRecord) -> bool {
    db.query_row(
        "select version from agent_text_tool_contracts where conversation_id=?1 and request_id=?2",
        rusqlite::params![
            record.conversation_id.to_string(),
            record.request_id.to_string()
        ],
        |row| row.get::<_, i64>(0),
    )
    .ok()
        == Some(2)
}
pub(crate) fn model_source(source: &research_agent_core::submission::ContextText) -> Value {
    let mut value = serde_json::to_value(source).unwrap_or(Value::Null);
    public_output(&mut value);
    value
}
fn public_output(value: &mut Value) {
    if let Some(anchor) = value.get("anchor").and_then(Value::as_str)
        && let Ok(mut parsed) = serde_json::from_str::<Value>(anchor)
        && parsed["textViewVersion"].is_number()
    {
        if let Some(object) = parsed.as_object_mut() {
            object.remove("blockId");
            object.remove("virtualPath");
            if !object.contains_key("sourceUrl")
                && let Some(page) = object.get("pageId").and_then(Value::as_str)
            {
                object.insert(
                    "sourceUrl".into(),
                    json!(text_patch::address::object(
                        text_patch::address::SourceRoot::Notes,
                        &[page.to_owned()]
                    )),
                );
            }
        }
        value["anchor"] = json!(parsed.to_string());
    }
    match value {
        Value::Array(values) => values.iter_mut().for_each(public_output),
        Value::Object(values) => values.values_mut().for_each(public_output),
        _ => {}
    }
}
