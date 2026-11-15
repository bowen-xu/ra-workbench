//! Independent canonical/history checks and exact original-operation replay.
use super::{scenario::*, *};
use research_agent_core::{
    conversation::ConversationId,
    protocol::{
        AgentEvent, AssistantMessagePhase, CanonicalItemSource, ConversationProjection,
        EventPayload, ItemKind,
    },
};
use rusqlite::{Connection, params};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct Facts {
    pub events: u64,
    pub event_sha256: String,
    pub text_sha256: String,
    pub terminal_turns: usize,
    pub canonical_items: usize,
}
#[derive(Default)]
pub(super) struct Prefix {
    counts: BTreeMap<String, u64>,
    bodies: BTreeMap<String, String>,
    messages: BTreeMap<String, Message>,
    tools: BTreeMap<String, Tool>,
}
struct Message {
    conversation: ConversationId,
    turn: uuid::Uuid,
    role: &'static str,
    completed: bool,
}
struct Tool {
    conversation: ConversationId,
    ordinal: u64,
    turn: uuid::Uuid,
    kind: ItemKind,
    phase: Option<AssistantMessagePhase>,
    source: Option<CanonicalItemSource>,
}
fn message_role(kind: &ItemKind) -> Option<&'static str> {
    match kind {
        ItemKind::UserMessage => Some("userMessage"),
        ItemKind::AssistantMessage => Some("assistantMessage"),
        ItemKind::ReasoningSummary { .. } => Some("reasoningSummary"),
        _ => None,
    }
}
impl Prefix {
    pub fn accept(&mut self, m: &Mutation) {
        self.counts
            .insert(m.conversation.as_uuid().to_string(), m.expected + 1);
        match m.event.payload() {
            EventPayload::ItemStarted {
                item_id,
                turn_id,
                kind,
            } => {
                self.bodies.insert(item_id.to_string(), String::new());
                self.messages.insert(
                    item_id.to_string(),
                    Message {
                        conversation: m.conversation,
                        turn: *turn_id,
                        role: message_role(kind).expect(
                            "synthetic streamed items use the production message-body store",
                        ),
                        completed: false,
                    },
                );
            }
            EventPayload::TextDelta { item_id, text, .. } => self
                .bodies
                .get_mut(&item_id.to_string())
                .unwrap()
                .push_str(text),
            EventPayload::ItemAccepted {
                item_id,
                turn_id,
                kind,
                text,
                phase,
                source,
            } => {
                self.bodies.insert(item_id.to_string(), text.clone());
                if let Some(role) = message_role(kind) {
                    self.messages.insert(
                        item_id.to_string(),
                        Message {
                            conversation: m.conversation,
                            turn: *turn_id,
                            role,
                            completed: true,
                        },
                    );
                } else if matches!(
                    kind,
                    ItemKind::ToolCall { .. } | ItemKind::ToolResult { .. }
                ) {
                    self.tools.insert(
                        item_id.to_string(),
                        Tool {
                            conversation: m.conversation,
                            ordinal: m.expected,
                            turn: *turn_id,
                            kind: kind.clone(),
                            phase: *phase,
                            source: source.clone(),
                        },
                    );
                } else {
                    panic!(
                        "add an explicit production storage verifier before benchmarking another canonical kind"
                    );
                }
            }
            _ => {}
        }
    }
    pub fn verify(&self, db: &Connection) {
        for (c, count) in &self.counts {
            assert_eq!(
                db.query_row(
                    "select durable_count from agent_conversations where id=?1",
                    [c],
                    |row| row.get::<_, u64>(0)
                )
                .unwrap(),
                *count
            );
        }
        for (item, body) in &self.bodies {
            if let Some(message) = self.messages.get(item) {
                let actual:(String,String,String,bool)=db.query_row("select body,turn_id,role,completed from agent_messages where conversation_id=?1 and item_id=?2",
                    params![message.conversation.as_uuid().to_string(),item],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?))).unwrap();
                assert_eq!(actual.0, *body);
                assert_eq!(actual.1, message.turn.to_string());
                assert_eq!(actual.2, message.role);
                assert_eq!(actual.3, message.completed);
            } else {
                let tool = self
                    .tools
                    .get(item)
                    .expect("every expected body has a production storage contract");
                verify_tool(db, item, body, tool);
            }
        }
    }
}
fn verify_tool(db: &Connection, item: &str, body: &str, tool: &Tool) {
    // 208/210 materialize only user/assistant/reasoning message bodies. Tool
    // canonical parameters/results remain complete in the authoritative journal.
    let raw:String=db.query_row("select payload_json from agent_journal_records where conversation_id=?1 and ordinal=?2",
        params![tool.conversation.as_uuid().to_string(),tool.ordinal],|row|row.get(0)).unwrap();
    let event: AgentEvent = serde_json::from_str(&raw).unwrap();
    assert_eq!(event.conversation_id(), tool.conversation);
    assert_eq!(event.seq(), tool.ordinal + 1);
    let EventPayload::ItemAccepted {
        turn_id,
        item_id,
        kind,
        text,
        phase,
        source,
    } = event.payload()
    else {
        panic!("tool canonical text must remain in its real journal record")
    };
    assert_eq!(*turn_id, tool.turn);
    assert_eq!(item_id.to_string(), item);
    // Kind comparison includes callId, name, providerCallId, namespace and
    // result error status. Text compares original argument/result UTF-8 bytes.
    assert_eq!(kind, &tool.kind);
    assert_eq!(text, body);
    assert_eq!(*phase, tool.phase);
    assert_eq!(source, &tool.source);
    assert_eq!(
        db.query_row(
            "select count(*) from agent_messages where conversation_id=?1 and item_id=?2",
            params![tool.conversation.as_uuid().to_string(), item],
            |row| row.get::<_, u64>(0)
        )
        .unwrap(),
        0
    );
}
pub(super) fn verify(db: &Connection, data: &Dataset) -> Facts {
    let mut prefix = Prefix::default();
    for m in data.setup.iter().chain(&data.measured) {
        prefix.accept(m);
    }
    prefix.verify(db);
    let mut events_hash = Sha256::new();
    let mut text_hash = Sha256::new();
    let mut events = 0;
    let mut terminals = 0;
    let mut canonical = 0;
    for conversation in &data.conversations {
        let mut statement = db
            .prepare(
                "select payload_json from agent_items where conversation_id=?1 order by ordinal",
            )
            .unwrap();
        let rows = statement
            .query_map([conversation.as_uuid().to_string()], |row| {
                row.get::<_, String>(0)
            })
            .unwrap();
        let expected = data
            .setup
            .iter()
            .chain(&data.measured)
            .filter(|m| m.conversation == *conversation)
            .map(|m| &m.event)
            .collect::<Vec<_>>();
        let mut projection = ConversationProjection::new(*conversation);
        let actual = rows
            .map(|row| serde_json::from_str::<AgentEvent>(&row.unwrap()).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(actual.len(), expected.len());
        for (value, expected) in actual.iter().zip(expected) {
            assert_eq!(value, expected);
            projection.apply(value).unwrap();
            let bytes = serde_json::to_vec(value).unwrap();
            events_hash.update((bytes.len() as u64).to_le_bytes());
            events_hash.update(bytes);
            events += 1;
        }
        assert_eq!(projection.active_turn(), None);
        for turn_id in projection.turn_order() {
            let turn = projection.turn(*turn_id).unwrap();
            assert_eq!(
                turn.outcome,
                Some(research_agent_core::protocol::TurnOutcome::Completed)
            );
            terminals += 1;
            assert!(
                turn.attempts
                    .values()
                    .all(|attempt| attempt.outcome.is_some())
            );
            for item in turn.items.values() {
                assert!(item.completed);
                if item.accepted {
                    canonical += 1;
                }
            }
        }
    }
    for (item, body) in prefix.bodies {
        text_hash.update(item);
        text_hash.update(body);
    }
    Facts {
        events,
        event_sha256: format!("{:x}", events_hash.finalize()),
        text_sha256: format!("{:x}", text_hash.finalize()),
        terminal_turns: terminals,
        canonical_items: canonical,
    }
}
#[derive(Debug, Serialize, Deserialize)]
pub(super) struct Ack {
    pub conversation: String,
    pub batch: uuid::Uuid,
    pub operation: Option<uuid::Uuid>,
    pub position: Option<u64>,
    pub result: u64,
    pub replayed: Option<bool>,
}
#[derive(Debug, Serialize, Deserialize)]
pub(super) struct Ledger {
    pub job: Job,
    pub setup_acknowledgements: Vec<Ack>,
    pub acknowledgements: Vec<Ack>,
    pub facts: Facts,
}
pub(super) fn verify_original(target: &mut workload::Target, data: &Dataset, ledger: &Ledger) {
    assert_eq!(ledger.acknowledgements.len(), data.measured.len());
    assert_eq!(ledger.setup_acknowledgements.len(), data.setup.len());
    let changes = target.sql(|db| db.total_changes());
    for (mutation, ack) in data.setup.iter().chain(&data.measured).zip(
        ledger
            .setup_acknowledgements
            .iter()
            .chain(&ledger.acknowledgements),
    ) {
        assert_eq!(
            ack.conversation,
            mutation.conversation.as_uuid().to_string()
        );
        assert_eq!(ack.batch, mutation.batch);
        if let Some(operation) = ack.operation {
            target.sql(|db|{let (position,result,fingerprint):(u64,String,Vec<u8>)=db.query_row("select position,result_json,fingerprint from data_operation_receipts where operation_id=?1",[operation.to_string()],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).unwrap();
                assert_eq!(Some(position),ack.position);assert_eq!(serde_json::from_str::<u64>(&result).unwrap(),ack.result);assert_eq!(fingerprint.len(),32);});
        } else {
            target.sql(|db|{let (expected,result,hash):(u64,u64,String)=db.query_row("select expected_count,result_count,payload_hash from agent_journal_batches where conversation_id=?1 and batch_id=?2",params![ack.conversation,ack.batch.to_string()],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).unwrap();assert_eq!(expected,mutation.expected);assert_eq!(result,ack.result);assert_eq!(hash,crate::agent::repository::journal::payload_hash(std::slice::from_ref(&mutation.event)).unwrap());});
        }
        // The real coordinator verifies its canonical command fingerprint on
        // exact replay. No benchmark copy of its private codec is substituted.
        let replay = target.append(mutation, ledger.job.mode);
        assert_eq!(replay.operation, ack.operation);
        assert_eq!(replay.position, ack.position);
        assert_eq!(replay.result, ack.result);
        if ack.operation.is_some() {
            assert_eq!(ack.replayed, Some(false));
            assert_eq!(replay.replayed, Some(true));
        }
    }
    assert_eq!(target.sql(|db| db.total_changes()), changes);
}

pub(super) fn expected(data: &Dataset) -> Facts {
    let mut event_hash = Sha256::new();
    let mut text_hash = Sha256::new();
    let mut prefix = Prefix::default();
    let mut events = 0;
    let mut terminals = 0;
    let mut canonical = 0;
    for m in data.setup.iter().chain(&data.measured) {
        prefix.accept(m);
    }
    for c in &data.conversations {
        let mut projection = ConversationProjection::new(*c);
        for m in data
            .setup
            .iter()
            .chain(&data.measured)
            .filter(|m| m.conversation == *c)
        {
            projection.apply(&m.event).unwrap();
            let bytes = serde_json::to_vec(&m.event).unwrap();
            event_hash.update((bytes.len() as u64).to_le_bytes());
            event_hash.update(bytes);
            events += 1;
        }
        assert_eq!(projection.active_turn(), None);
        for turn_id in projection.turn_order() {
            let turn = projection.turn(*turn_id).unwrap();
            assert_eq!(
                turn.outcome,
                Some(research_agent_core::protocol::TurnOutcome::Completed)
            );
            terminals += 1;
            canonical += turn.items.values().filter(|item| item.accepted).count();
        }
    }
    for (item, body) in prefix.bodies {
        text_hash.update(item);
        text_hash.update(body);
    }
    Facts {
        events,
        event_sha256: format!("{:x}", event_hash.finalize()),
        text_sha256: format!("{:x}", text_hash.finalize()),
        terminal_turns: terminals,
        canonical_items: canonical,
    }
}
