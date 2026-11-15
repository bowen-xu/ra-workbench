use research_agent_core::protocol::{AgentEvent, EventPayload};
use rusqlite::{Transaction, params};
use std::collections::{HashMap, hash_map::Entry};
use uuid::Uuid;

use crate::agent::repository::JournalError;

struct Pending {
    conversation: Uuid,
    turn: Uuid,
    item: Uuid,
    last_ordinal: u64,
    text: String,
}
#[derive(Default)]
pub(super) struct BodyCoalescer {
    index: HashMap<(Uuid, Uuid, Uuid), usize>,
    pending: Vec<Pending>,
}
impl BodyCoalescer {
    pub(super) fn before_insert(
        &mut self,
        tx: &Transaction<'_>,
        ordinal: u64,
        event: &AgentEvent,
    ) -> Result<(), JournalError> {
        let (turn_id, item_id, text) = match event.payload() {
            EventPayload::TextDelta {
                turn_id,
                item_id,
                text,
            }
            | EventPayload::ReasoningSummaryDelta {
                turn_id,
                item_id,
                text,
                ..
            } => (turn_id, item_id, text),
            _ => return self.flush(tx),
        };
        let conversation = *event.conversation_id().as_uuid();
        let index = match self.index.entry((conversation, *turn_id, *item_id)) {
            Entry::Occupied(entry) => *entry.get(),
            Entry::Vacant(entry) => {
                let index = self.pending.len();
                self.pending.push(Pending {
                    conversation,
                    turn: *turn_id,
                    item: *item_id,
                    last_ordinal: ordinal,
                    text: String::new(),
                });
                entry.insert(index);
                index
            }
        };
        let pending = &mut self.pending[index];
        pending.text.push_str(text);
        pending.last_ordinal = ordinal;
        Ok(())
    }
    pub(super) fn flush(&mut self, tx: &Transaction<'_>) -> Result<(), JournalError> {
        self.index.clear();
        // Draft text follows arrival order. A summary's canonical acceptance
        // replaces that draft; its part-aware byte proof is owned by SQL. Keep
        // first-seen key order without changing any journal event ordering.
        for pending in self.pending.drain(..) {
            tx.execute("update agent_messages set body=body || ?4,last_ordinal=?5
            where conversation_id=?1 and turn_id=?2 and item_id=?3 and completed=0 and last_ordinal<?5",
            params![pending.conversation.to_string(),pending.turn.to_string(),pending.item.to_string(),pending.text,
                i64::try_from(pending.last_ordinal).map_err(|_|JournalError::InvalidInput)?])?;
        }
        Ok(())
    }
}
