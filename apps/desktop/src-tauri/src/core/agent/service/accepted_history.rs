//! Bounded delivery of already accepted events. SQLite remains the resync source.
use research_agent_core::{
    conversation::{ConversationId, StoreError},
    protocol::AgentEvent,
    session::CommittedBatch,
};
use std::collections::VecDeque;

const MAX_BYTES: usize = 4 * 1024 * 1024;
const MAX_RECORDS: usize = 4096;
const MAX_CONVERSATIONS: usize = 256;
const REPLY_RECORDS: usize = 100;
const REPLY_BYTES: usize = 256 * 1024;

pub(crate) struct AcceptedHistoryUpdate {
    pub watermark: Option<u64>,
    pub events: Vec<AgentEvent>,
    pub resync_required: bool,
}
impl AcceptedHistoryUpdate {
    pub(super) fn idle() -> Self {
        Self {
            watermark: None,
            events: vec![],
            resync_required: false,
        }
    }
}
struct Record {
    event: AgentEvent,
    bytes: usize,
}
pub(super) struct AcceptedHistory {
    records: VecDeque<Record>,
    heads: VecDeque<(ConversationId, u64)>,
    bytes: usize,
    max_bytes: usize,
    max_records: usize,
}
impl Default for AcceptedHistory {
    fn default() -> Self {
        Self {
            records: VecDeque::new(),
            heads: VecDeque::new(),
            bytes: 0,
            max_bytes: MAX_BYTES,
            max_records: MAX_RECORDS,
        }
    }
}
impl AcceptedHistory {
    pub(super) fn watermark(&self, id: ConversationId) -> Option<u64> {
        self.heads
            .iter()
            .find(|(known, _)| *known == id)
            .map(|(_, seq)| *seq)
    }
    pub(super) fn observe_snapshot(&mut self, id: ConversationId, watermark: u64) {
        if let Some((_, known)) = self.heads.iter_mut().find(|(known, _)| *known == id) {
            *known = (*known).max(watermark);
        } else {
            if self.heads.len() == MAX_CONVERSATIONS {
                self.heads.pop_front();
            }
            self.heads.push_back((id, watermark));
        }
    }
    pub(super) fn push(&mut self, id: ConversationId, receipt: &CommittedBatch) {
        self.observe_snapshot(id, receipt.watermark);
        for event in &receipt.events {
            if event.conversation_id() != id {
                self.invalidate(id);
                return;
            }
            let Ok(encoded) = serde_json::to_vec(event) else {
                self.invalidate(id);
                return;
            };
            let bytes = encoded.len();
            // Oversized items remain durable; a missing range requires resync.
            if bytes > self.max_bytes {
                continue;
            }
            self.records.push_back(Record {
                event: event.clone(),
                bytes,
            });
            self.bytes += bytes;
            while self.bytes > self.max_bytes || self.records.len() > self.max_records {
                if let Some(record) = self.records.pop_front() {
                    self.bytes -= record.bytes;
                }
            }
        }
    }
    pub(super) fn invalidate(&mut self, id: ConversationId) {
        self.heads.retain(|(known, _)| *known != id);
        self.records.retain(|record| {
            if record.event.conversation_id() == id {
                self.bytes -= record.bytes;
                false
            } else {
                true
            }
        });
    }
    fn clear(&mut self) {
        self.records.clear();
        self.heads.clear();
        self.bytes = 0;
    }
    pub(super) fn read(
        &self,
        id: ConversationId,
        after: u64,
    ) -> Result<Option<AcceptedHistoryUpdate>, StoreError> {
        let Some(watermark) = self.watermark(id) else {
            return Ok(Some(AcceptedHistoryUpdate {
                watermark: None,
                events: vec![],
                resync_required: true,
            }));
        };
        if watermark > 9_007_199_254_740_991 {
            return Err(StoreError::CorruptHistory);
        }
        if after > watermark {
            return Err(StoreError::InvalidPage);
        }
        if after == watermark {
            return Ok(None);
        }
        let mut events = Vec::new();
        let mut bytes: usize = 0;
        for record in self
            .records
            .iter()
            .filter(|record| record.event.conversation_id() == id && record.event.seq() > after)
        {
            if record.event.seq() != after + events.len() as u64 + 1 {
                break;
            }
            if events.len() == REPLY_RECORDS
                || (!events.is_empty() && bytes.saturating_add(record.bytes) > REPLY_BYTES)
            {
                break;
            }
            events.push(record.event.clone());
            bytes += record.bytes;
        }
        Ok(Some(AcceptedHistoryUpdate {
            watermark: Some(watermark),
            resync_required: events.is_empty(),
            events,
        }))
    }
}

impl super::AgentSessionService {
    /// Called only after a validated SQL history response. It publishes no events.
    pub(crate) fn observe_history_snapshot(&self, id: ConversationId, watermark: u64) {
        self.accepted_history
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .observe_snapshot(id, watermark);
    }
    /// Deletion/reset owners invalidate the derived ring before reusing identities.
    pub(crate) fn invalidate_accepted_history(&self, id: ConversationId) {
        self.accepted_history
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .invalidate(id);
    }
    /// Store-epoch replacement first drains this service through shutdown.
    pub(crate) fn invalidate_all_accepted_history(&self) {
        self.accepted_history
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use research_agent_core::protocol::{EventPayload, ItemKind};
    use uuid::Uuid;
    fn batch(id: ConversationId, seq: u64, text: &str) -> CommittedBatch {
        CommittedBatch {
            watermark: seq,
            events: vec![
                AgentEvent::new(
                    id,
                    seq,
                    1,
                    Uuid::from_u128(1),
                    EventPayload::ItemAccepted {
                        turn_id: Uuid::from_u128(2),
                        item_id: Uuid::from_u128(u128::from(seq) + 10),
                        kind: ItemKind::AssistantMessage,
                        text: text.into(),
                        phase: None,
                        source: None,
                    },
                )
                .unwrap(),
            ],
        }
    }
    #[test]
    fn bounded_ring_keeps_conversation_sequences_and_explicitly_reports_gaps() {
        let a = ConversationId::from_uuid(Uuid::from_u128(1));
        let b = ConversationId::from_uuid(Uuid::from_u128(2));
        let mut cache = AcceptedHistory {
            max_records: 2,
            ..Default::default()
        };
        cache.push(a, &batch(a, 1, "a"));
        cache.push(b, &batch(b, 1, "b"));
        cache.push(a, &batch(a, 2, "a2"));
        assert!(cache.read(a, 0).unwrap().unwrap().resync_required);
        let update = cache.read(a, 1).unwrap().unwrap();
        assert_eq!(update.events[0].seq(), 2);
        assert!(!update.resync_required);
        assert_eq!(
            cache.read(b, 0).unwrap().unwrap().events[0].conversation_id(),
            b
        );
        assert!(cache.read(a, 2).unwrap().is_none());
        assert!(matches!(cache.read(a, 3), Err(StoreError::InvalidPage)));
    }
    #[test]
    fn byte_bound_and_large_item_require_resync_without_publishing_partial_records() {
        let id = ConversationId::from_uuid(Uuid::from_u128(1));
        let mut cache = AcceptedHistory {
            max_bytes: 600,
            ..Default::default()
        };
        cache.push(id, &batch(id, 1, "small"));
        cache.push(id, &batch(id, 2, &"x".repeat(1000)));
        assert!(cache.bytes <= 600);
        assert!(cache.records.len() <= 1);
        assert_eq!(cache.read(id, 0).unwrap().unwrap().events.len(), 1);
        assert!(cache.read(id, 1).unwrap().unwrap().resync_required);
        cache.observe_snapshot(id, 2);
        assert!(cache.read(id, 2).unwrap().is_none());
        cache.invalidate(id);
        assert!(cache.read(id, 2).unwrap().unwrap().resync_required);
    }
    #[test]
    fn metadata_is_bounded_and_snapshot_does_not_regress_an_accepted_head() {
        let id = ConversationId::from_uuid(Uuid::from_u128(1));
        let mut cache = AcceptedHistory::default();
        cache.observe_snapshot(id, 5);
        cache.observe_snapshot(id, 2);
        assert_eq!(cache.watermark(id), Some(5));
        for n in 2..=MAX_CONVERSATIONS + 1 {
            cache.observe_snapshot(ConversationId::from_uuid(Uuid::from_u128(n as u128)), 0);
        }
        assert_eq!(cache.heads.len(), MAX_CONVERSATIONS);
        assert_eq!(cache.watermark(id), None);
    }
}
