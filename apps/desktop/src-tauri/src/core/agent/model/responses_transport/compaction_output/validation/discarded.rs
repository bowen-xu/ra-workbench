//! Known filtered ResponseItem contracts from pinned Codex models.rs,
//! e363b08c9175ac1cbe5893615dd2cb9ddf95043b, Apache-2.0, Copyright 2025 OpenAI.
//! Validation never executes returned tools or retains their historical output.
use super::ImageDetail;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;

pub(super) fn valid(kind: &str, item: &Value) -> Option<bool> {
    matches!(
        kind,
        "additional_tools"
            | "reasoning"
            | "local_shell_call"
            | "function_call"
            | "function_call_output"
            | "custom_tool_call"
            | "custom_tool_call_output"
            | "tool_search_call"
            | "tool_search_output"
            | "web_search_call"
            | "image_generation_call"
            | "compaction_trigger"
    )
    .then(|| serde_json::from_value::<Item>(item.clone()).is_ok())
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Item {
    AdditionalTools {
        #[serde(rename = "role")]
        _role: String,
        #[serde(rename = "tools")]
        _tools: Vec<Value>,
    },
    Reasoning {
        #[serde(rename = "summary")]
        _summary: Vec<Summary>,
        #[serde(rename = "content")]
        _content: Option<Vec<ReasoningContent>>,
        #[serde(rename = "encrypted_content")]
        _encrypted_content: Option<String>,
    },
    LocalShellCall {
        #[serde(rename = "call_id")]
        _call_id: Option<String>,
        #[serde(rename = "status")]
        _status: ShellStatus,
        #[serde(rename = "action")]
        _action: ShellAction,
    },
    FunctionCall {
        #[serde(rename = "name")]
        _name: String,
        #[serde(rename = "namespace")]
        _namespace: Option<String>,
        #[serde(rename = "arguments")]
        _arguments: String,
        #[serde(rename = "call_id")]
        _call_id: String,
    },
    FunctionCallOutput {
        #[serde(rename = "call_id")]
        _call_id: String,
        #[serde(rename = "output", deserialize_with = "output")]
        _output: (),
    },
    CustomToolCall {
        #[serde(rename = "status")]
        _status: Option<String>,
        #[serde(rename = "call_id")]
        _call_id: String,
        #[serde(rename = "name")]
        _name: String,
        #[serde(rename = "namespace")]
        _namespace: Option<String>,
        #[serde(rename = "input")]
        _input: String,
    },
    CustomToolCallOutput {
        #[serde(rename = "call_id")]
        _call_id: String,
        #[serde(rename = "name")]
        _name: Option<String>,
        #[serde(rename = "output", deserialize_with = "output")]
        _output: (),
    },
    ToolSearchCall {
        #[serde(rename = "call_id")]
        _call_id: Option<String>,
        #[serde(rename = "status")]
        _status: Option<String>,
        #[serde(rename = "execution")]
        _execution: String,
        #[serde(rename = "arguments")]
        _arguments: Value,
    },
    ToolSearchOutput {
        #[serde(rename = "call_id")]
        _call_id: Option<String>,
        #[serde(rename = "status")]
        _status: String,
        #[serde(rename = "execution")]
        _execution: String,
        #[serde(rename = "tools")]
        _tools: Vec<Value>,
    },
    WebSearchCall {
        #[serde(rename = "status")]
        _status: Option<String>,
        #[serde(rename = "action")]
        _action: Option<WebAction>,
    },
    ImageGenerationCall {
        #[serde(rename = "status")]
        _status: String,
        #[serde(rename = "revised_prompt")]
        _revised_prompt: Option<String>,
        #[serde(rename = "result")]
        _result: String,
    },
    CompactionTrigger {},
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Summary {
    SummaryText {
        #[serde(rename = "text")]
        _text: String,
    },
}
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ReasoningContent {
    ReasoningText {
        #[serde(rename = "text")]
        _text: String,
    },
    Text {
        #[serde(rename = "text")]
        _text: String,
    },
}
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum ShellStatus {
    Completed,
    InProgress,
    Incomplete,
}
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ShellAction {
    Exec {
        #[serde(rename = "command")]
        _command: Vec<String>,
        #[serde(rename = "timeout_ms")]
        _timeout_ms: Option<u64>,
        #[serde(rename = "working_directory")]
        _working_directory: Option<String>,
        #[serde(rename = "env")]
        _env: Option<HashMap<String, String>>,
        #[serde(rename = "user")]
        _user: Option<String>,
    },
}
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum WebAction {
    Search {
        #[serde(rename = "query")]
        _query: Option<String>,
        #[serde(rename = "queries")]
        _queries: Option<Vec<String>>,
    },
    OpenPage {
        #[serde(rename = "url")]
        _url: Option<String>,
    },
    FindInPage {
        #[serde(rename = "url")]
        _url: Option<String>,
        #[serde(rename = "pattern")]
        _pattern: Option<String>,
    },
    #[serde(other)]
    Other,
}
fn output<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<(), D::Error> {
    let value = Value::deserialize(deserializer)?;
    if value.is_string() {
        return Ok(());
    }
    serde_json::from_value::<Vec<OutputPart>>(value)
        .map(|_| ())
        .map_err(serde::de::Error::custom)
}
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum OutputPart {
    InputText {
        #[serde(rename = "text")]
        _text: String,
    },
    InputImage {
        #[serde(rename = "image_url")]
        _image_url: String,
        #[serde(rename = "detail")]
        _detail: Option<ImageDetail>,
    },
    InputAudio {
        #[serde(rename = "audio_url")]
        _audio_url: String,
    },
    EncryptedContent {
        #[serde(rename = "encrypted_content")]
        _encrypted_content: String,
    },
}
