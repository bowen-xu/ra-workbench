//! Adapted from fixed Codex session::current_window_id and responses_metadata,
//! e363b08c9175ac1cbe5893615dd2cb9ddf95043b, Apache-2.0, Copyright 2025 OpenAI.
//! The native owner supplies this snapshot with the prepared input. It is not
//! model text, a checkpoint UUID, or a client-provided metadata override.
use research_agent_core::model::CapturedHistoryRef;
use research_agent_core::model::ModelError;
use serde_json::{Value, json};
use uuid::Uuid;

/// Immutable source handles captured with the model request, before any later
/// steer or background append can reach SQL. Never sent to the provider.
#[derive(Clone)]
pub struct CapturedResponsesInput {
    pub request_id: Uuid,
    pub sampling: bool,
    pub authority_items: usize,
    pub text_compaction: bool,
    pub history: Option<CapturedHistoryRef>,
    pub items: Vec<CapturedInputItem>,
    pub users: Vec<CapturedInputItem>,
}
#[derive(Clone)]
pub struct CapturedInputItem {
    pub item_id: Uuid,
    pub input_index: usize,
    pub native: bool,
}

#[derive(Clone, Copy)]
pub enum ResponsesInputKind {
    Sampling,
    TextCompaction,
    /// Boundary captured before System and Developer share a wire role.
    Scoped {
        authority_items: usize,
        text_compaction: bool,
    },
}
impl ResponsesInputKind {
    pub(crate) fn authority_items(self) -> Option<usize> {
        match self {
            Self::Scoped {
                authority_items, ..
            } => Some(authority_items),
            _ => None,
        }
    }
    pub(crate) fn text_compaction(self) -> bool {
        matches!(
            self,
            Self::TextCompaction
                | Self::Scoped {
                    text_compaction: true,
                    ..
                }
        )
    }
}
pub struct PreparedResponsesInput {
    pub input: Vec<Value>,
    pub metadata: Option<ResponsesRequestMetadata>,
}
pub struct ResponsesRequestMetadata {
    conversation: Uuid,
    turn: Uuid,
    window_number: u64,
}
impl ResponsesRequestMetadata {
    pub fn new(conversation: Uuid, turn: Uuid, window_number: u64) -> Result<Self, ModelError> {
        if conversation.is_nil() || turn.is_nil() {
            return Err(ModelError::InvalidRequest);
        }
        Ok(Self {
            conversation,
            turn,
            window_number,
        })
    }
    pub fn window_id(&self) -> String {
        format!("{}:{}", self.conversation, self.window_number)
    }
    pub(crate) fn apply_body(&self, body: &mut Value) {
        // Fixed ModelClient::prompt_cache_key defaults to the session identity.
        // Compaction advances the window, not this conversation-scoped key.
        body["prompt_cache_key"] = json!(self.conversation.to_string());
        body["client_metadata"] = json!({
            "thread_id": self.conversation.to_string(),
            "turn_id": self.turn.to_string(),
            "x-codex-window-id": self.window_id(),
        });
    }
    pub(crate) fn apply_headers(&self, request: &mut reqwest::Request) -> Result<(), ModelError> {
        request.headers_mut().insert(
            "x-codex-window-id",
            self.window_id()
                .parse()
                .map_err(|_| ModelError::InvalidRequest)?,
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_key_survives_turn_and_window_changes_but_not_a_new_conversation() {
        let parent = Uuid::new_v4();
        let child = Uuid::new_v4();
        let mut previous = json!({});
        ResponsesRequestMetadata::new(parent, Uuid::new_v4(), 0)
            .unwrap()
            .apply_body(&mut previous);
        let mut next = json!({});
        ResponsesRequestMetadata::new(parent, Uuid::new_v4(), 2)
            .unwrap()
            .apply_body(&mut next);
        let mut fork = json!({});
        ResponsesRequestMetadata::new(child, Uuid::new_v4(), 2)
            .unwrap()
            .apply_body(&mut fork);
        assert_eq!(previous["prompt_cache_key"], next["prompt_cache_key"]);
        assert_ne!(previous["client_metadata"], next["client_metadata"]);
        assert_eq!(fork["prompt_cache_key"], child.to_string());
        assert_ne!(next["prompt_cache_key"], fork["prompt_cache_key"]);
    }
}
