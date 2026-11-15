//! Codex endpoint/compact.rs turn-state contract, pinned e363b08c (Apache-2.0).
//! Transport-local routing state, never model content or a durable checkpoint.
use super::*;

const HEADER: &str = "x-codex-turn-state";

impl ResponsesTransport {
    pub(super) fn capture_turn_state(&self, response: &Response, account: Uuid) {
        if let Some(header) = response.headers().get(HEADER)
            && header.to_str().is_ok()
        {
            let mut value = header.clone();
            value.set_sensitive(true);
            // Pinned CLI OnceLock preserves the first response's state in a turn.
            let _ = self.turn_state.set((account, value));
        }
    }

    pub(super) fn apply_turn_state(&self, request: &mut reqwest::Request, account: Uuid) {
        if let Some((scope, value)) = self.turn_state.get()
            && *scope == account
        {
            request.headers_mut().insert(HEADER, value.clone());
        }
    }
}
