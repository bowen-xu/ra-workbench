//! ResponseItem fields from fixed Codex protocol/models.rs,
//! e363b08c9175ac1cbe5893615dd2cb9ddf95043b, Apache-2.0, Copyright 2025 OpenAI.
//! Decode for validation only; never reserialize away provider extension fields.
use serde::Deserialize;
use serde_json::Value;
#[path = "validation/discarded.rs"]
mod discarded;

pub(in crate::agent::model::responses_transport) fn valid(item: &Value) -> bool {
    let Some(kind) = item["type"].as_str() else {
        return false;
    };
    if let Some(valid) = discarded::valid(kind, item) {
        return valid
            && match kind {
                // These two protocol variants do not carry passthrough metadata.
                "compaction_trigger" => true,
                "additional_tools" => item
                    .get("id")
                    .is_none_or(|id| id.is_null() || id.is_string()),
                _ => valid_envelope(item),
            };
    }
    if !matches!(
        kind,
        "message" | "agent_message" | "compaction" | "compaction_summary" | "context_compaction"
    ) {
        // Pinned ResponseItem::Other accepts unknown families before filtering.
        return true;
    }
    valid_envelope(item) && serde_json::from_value::<RetainedItem>(item.clone()).is_ok()
}

fn valid_envelope(item: &Value) -> bool {
    // Metadata is a JSON object in the wire contract; do not let Serde
    // positional struct decoding accept an array as a metadata object.
    if item
        .get("internal_chat_message_metadata_passthrough")
        .is_some_and(|value| !value.is_null() && !value.is_object())
    {
        return false;
    }
    serde_json::from_value::<Envelope>(item.clone()).is_ok()
}

#[derive(Deserialize)]
struct Envelope {
    #[serde(default, rename = "id")]
    _id: Option<String>,
    #[serde(default, rename = "internal_chat_message_metadata_passthrough")]
    _metadata: Option<Metadata>,
}
#[derive(Deserialize)]
struct Metadata {
    #[serde(default, rename = "turn_id")]
    _turn_id: Option<String>,
}
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum RetainedItem {
    Message {
        #[serde(rename = "role")]
        _role: String,
        #[serde(rename = "content")]
        _content: Vec<Content>,
        #[serde(default, rename = "phase")]
        _phase: Option<Phase>,
    },
    AgentMessage {
        #[serde(rename = "author")]
        _author: String,
        #[serde(rename = "recipient")]
        _recipient: String,
        #[serde(rename = "content")]
        _content: Vec<AgentContent>,
    },
    #[serde(alias = "compaction_summary")]
    Compaction {
        #[serde(rename = "encrypted_content")]
        _encrypted_content: String,
    },
    ContextCompaction {
        #[serde(default, rename = "encrypted_content")]
        _encrypted_content: Option<String>,
    },
}
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Phase {
    Commentary,
    FinalAnswer,
}
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Content {
    InputText {
        #[serde(rename = "text")]
        _text: String,
    },
    OutputText {
        #[serde(rename = "text")]
        _text: String,
    },
    InputImage {
        #[serde(rename = "image_url")]
        _image_url: String,
        #[serde(default, rename = "detail")]
        _detail: Option<ImageDetail>,
    },
    InputAudio {
        #[serde(rename = "audio_url")]
        _audio_url: String,
    },
}
#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum ImageDetail {
    Auto,
    Low,
    High,
    Original,
}
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum AgentContent {
    InputText {
        #[serde(rename = "text")]
        _text: String,
    },
    EncryptedContent {
        #[serde(rename = "encrypted_content")]
        _encrypted_content: String,
    },
}
