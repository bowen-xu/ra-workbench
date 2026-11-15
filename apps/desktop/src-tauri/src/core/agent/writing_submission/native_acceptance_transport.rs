//! Opt-in model response fixture for disposable native writing acceptance.
//! The production runner, tool registry, admission and file delivery are unchanged.
use crate::storage::AppStore;
use research_agent_core::model::{
    ContentPart, FinishReason, ModelCapabilities, ModelError, ModelFuture, ModelMessage,
    ModelRequest, ModelStream, ModelStreamEvent, ModelTransport,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    collections::VecDeque,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
use uuid::Uuid;

const ENABLE: &str = "RESEARCH_ASSISTANT_NATIVE_WRITING_ACCEPTANCE";
const MAX_SCRIPT_BYTES: u64 = 128 * 1024;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Script {
    schema_version: u8,
    manuscript_id: Uuid,
    required_input: String,
    pause_before_call: Option<usize>,
    steps: Vec<Step>,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
enum Step {
    Tool {
        name: String,
        arguments: serde_json::Value,
    },
    Text {
        text: String,
    },
}

/// This module is compiled only for debug development-data builds. A configured
/// fixture outside the exact isolated data root fails instead of using a provider.
pub(super) async fn replace(
    store: &AppStore,
    manuscript_id: Uuid,
    capabilities: ModelCapabilities,
    fallback: Arc<dyn ModelTransport>,
) -> Result<Arc<dyn ModelTransport>, ModelError> {
    if !enabled(std::env::var_os(ENABLE).as_deref())? {
        return Ok(fallback);
    }
    let root = store
        .content_directory()
        .parent()
        .ok_or(ModelError::InvalidRequest)?
        .to_path_buf();
    tokio::task::spawn_blocking(move || {
        let script = read_script(&root, manuscript_id)?;
        Ok(Arc::new(FixtureTransport {
            script,
            capabilities,
            calls: AtomicUsize::new(0),
            audit: root.join("native-acceptance-model-observations.jsonl"),
        }) as Arc<dyn ModelTransport>)
    })
    .await
    .map_err(|_| ModelError::Unavailable)?
}

fn enabled(value: Option<&std::ffi::OsStr>) -> Result<bool, ModelError> {
    match value {
        None => Ok(false),
        Some(value) if value == "1" => Ok(true),
        Some(_) => Err(ModelError::InvalidRequest),
    }
}

enum FileAccess {
    Read,
    Append,
}

fn open_file(path: &Path, access: FileAccess) -> Result<std::fs::File, ModelError> {
    let append = matches!(access, FileAccess::Append);
    let mut options = std::fs::OpenOptions::new();
    options.read(!append).append(append).create(append);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(nix::libc::O_NOFOLLOW).mode(0o600);
    }
    let file = options.open(path).map_err(|_| ModelError::InvalidRequest)?;
    if !file
        .metadata()
        .map_err(|_| ModelError::InvalidRequest)?
        .is_file()
    {
        return Err(ModelError::InvalidRequest);
    }
    Ok(file)
}

fn read_script(root: &Path, manuscript_id: Uuid) -> Result<Script, ModelError> {
    if root.parent() != Some(Path::new("/private/tmp"))
        || !root
            .file_name()
            .and_then(|value| value.to_str())
            .is_some_and(|name| name.starts_with("research-assistant-writing-diff-agent-"))
        || std::fs::canonicalize(root).map_err(|_| ModelError::InvalidRequest)? != root
    {
        return Err(ModelError::InvalidRequest);
    }
    let read = |name: &str| {
        let path = root.join(name);
        if std::fs::canonicalize(&path).map_err(|_| ModelError::InvalidRequest)? != path {
            return Err(ModelError::InvalidRequest);
        }
        use std::io::Read;
        let mut bytes = Vec::new();
        open_file(&path, FileAccess::Read)?
            .take(MAX_SCRIPT_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| ModelError::InvalidRequest)?;
        if bytes.len() as u64 > MAX_SCRIPT_BYTES {
            return Err(ModelError::InvalidRequest);
        }
        Ok(bytes)
    };
    let manifest: serde_json::Value = serde_json::from_slice(&read("native-acceptance.json")?)
        .map_err(|_| ModelError::InvalidRequest)?;
    if manifest
        .get("dataDirectory")
        .and_then(|value| value.as_str())
        != root.to_str()
        || manifest
            .get("manuscriptId")
            .and_then(|value| value.as_str())
            != Some(manuscript_id.to_string().as_str())
    {
        return Err(ModelError::InvalidRequest);
    }
    let script: Script = serde_json::from_slice(&read("native-acceptance-model.json")?)
        .map_err(|_| ModelError::InvalidRequest)?;
    if script.schema_version != 1
        || script.manuscript_id != manuscript_id
        || script.required_input.is_empty()
        || script.steps.is_empty()
        || script.steps.len() > 8
        || script
            .pause_before_call
            .is_some_and(|call| call >= script.steps.len())
        || !matches!(script.steps.last(), Some(Step::Text { .. }))
        || script.steps.iter().any(|step| match step {
            Step::Tool { name, arguments } => {
                !matches!(
                    name.as_str(),
                    "research_apply_patch" | "research_review_workspace_changes"
                ) || !arguments.is_object()
            }
            Step::Text { text } => text.is_empty(),
        })
    {
        return Err(ModelError::InvalidRequest);
    }
    Ok(script)
}

struct FixtureTransport {
    script: Script,
    capabilities: ModelCapabilities,
    calls: AtomicUsize,
    audit: std::path::PathBuf,
}

impl ModelTransport for FixtureTransport {
    fn capabilities(&self) -> ModelCapabilities {
        self.capabilities.clone()
    }
    fn start(&self, request: ModelRequest) -> ModelFuture<'_, Box<dyn ModelStream>> {
        Box::pin(async move {
            let call = self.calls.fetch_add(1, Ordering::SeqCst);
            let Some(step) = self.script.steps.get(call) else {
                return Err(ModelError::InvalidRequest);
            };
            if request
                .messages
                .iter()
                .any(|message| matches!(message, ModelMessage::ToolResult { is_error: true, .. }))
                || (call == 0 && !has_input(&request, &self.script.required_input))
            {
                return Err(ModelError::InvalidRequest);
            }
            let id = format!("native-acceptance-{call}");
            let mut events = VecDeque::from([ModelStreamEvent::Started {
                response_id: id.clone(),
                actual_model: request.model,
            }]);
            match step {
                Step::Tool { name, arguments } => {
                    if !request.tools.iter().any(|tool| &tool.name == name) {
                        return Err(ModelError::UnsupportedCapability);
                    }
                    events.extend([
                        ModelStreamEvent::ToolCallStarted {
                            call_id: id.clone(),
                            name: name.clone(),
                        },
                        ModelStreamEvent::ToolArgumentsDelta {
                            call_id: id.clone(),
                            delta: arguments.to_string(),
                        },
                        ModelStreamEvent::ToolCallCompleted { call_id: id },
                        ModelStreamEvent::Completed {
                            reason: FinishReason::ToolCalls,
                            end_turn: None,
                        },
                    ]);
                }
                Step::Text { text } => events.extend([
                    ModelStreamEvent::TextDelta(text.clone()),
                    ModelStreamEvent::Completed {
                        reason: FinishReason::Stop,
                        end_turn: None,
                    },
                ]),
            }
            // Record only synthetic scope and hashes, never request content or credentials.
            let observation = serde_json::json!({"manuscriptId":self.script.manuscript_id,
                "requestId":request.request_id,"call":call,"requiredInputSha256":format!("{:x}",Sha256::digest(self.script.required_input.as_bytes()))});
            let audit = self.audit.clone();
            tokio::task::spawn_blocking(move || {
                use std::io::Write;
                let mut file = open_file(&audit, FileAccess::Append)?;
                writeln!(file, "{observation}").map_err(|_| ModelError::Unavailable)
            })
            .await
            .map_err(|_| ModelError::Unavailable)??;
            if self.script.pause_before_call == Some(call) {
                #[cfg(unix)]
                crate::native_acceptance_pause::Gate::bind(
                    self.audit.parent().ok_or(ModelError::InvalidRequest)?,
                    crate::native_acceptance_pause::EndpointName::ModelResponse,
                )
                .await?
                .wait(std::time::Duration::from_secs(120))
                .await?;
                #[cfg(not(unix))]
                return Err(ModelError::UnsupportedCapability);
            }
            Ok(Box::new(FixtureStream(events)) as Box<dyn ModelStream>)
        })
    }
}

fn has_input(request: &ModelRequest, expected: &str) -> bool {
    // The production execution adapter includes frozen sources as JSON text.
    // Match the complete quoted value, preserving newlines and backslashes.
    let Ok(quoted) = serde_json::to_string(expected) else {
        return false;
    };
    let contains = |text: &str| text.contains(expected) || text.contains(&quoted);
    request.messages.iter().any(|message| match message {
        ModelMessage::System(text) => contains(text),
        ModelMessage::User { content: parts, .. } => parts
            .iter()
            .any(|part| matches!(part, ContentPart::Text(text) if contains(text))),
        _ => false,
    })
}

struct FixtureStream(VecDeque<ModelStreamEvent>);
impl ModelStream for FixtureStream {
    fn next(&mut self) -> ModelFuture<'_, Option<ModelStreamEvent>> {
        Box::pin(async move { Ok(self.0.pop_front()) })
    }
}

#[cfg(all(test, target_os = "macos"))]
#[path = "native_acceptance_transport/tests.rs"]
mod tests;
