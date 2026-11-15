//! Deterministic synthetic facts, identical across all persistence modes.
use super::*;
use research_agent_core::{conversation::ConversationId, protocol::*};
use research_data_runtime::{Durability, WriteOptions};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum Mode {
    LegacyNormal,
    LegacyFull,
    AutoProcess,
    AutoDevice,
}
impl Mode {
    pub fn all() -> [Self; 4] {
        [
            Self::LegacyNormal,
            Self::AutoProcess,
            Self::LegacyFull,
            Self::AutoDevice,
        ]
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::LegacyNormal => "legacy_normal",
            Self::LegacyFull => "legacy_full",
            Self::AutoProcess => "auto_process",
            Self::AutoDevice => "auto_device",
        }
    }
    pub fn logged(self) -> bool {
        matches!(self, Self::AutoProcess | Self::AutoDevice)
    }
    pub fn options(self) -> WriteOptions {
        WriteOptions {
            durability: if matches!(self, Self::LegacyFull | Self::AutoDevice) {
                Durability::Device
            } else {
                Durability::Process
            },
            ..Default::default()
        }
    }
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum Shape {
    Short,
    FineLowCompression,
    LongHistory,
    MultiTools,
    LargeToolResult,
    EightConversations,
}
impl Shape {
    pub fn all() -> [Self; 6] {
        [
            Self::Short,
            Self::FineLowCompression,
            Self::LongHistory,
            Self::MultiTools,
            Self::LargeToolResult,
            Self::EightConversations,
        ]
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Short => "short",
            Self::FineLowCompression => "fine_low_compression",
            Self::LongHistory => "long_history",
            Self::MultiTools => "multi_tools",
            Self::LargeToolResult => "large_tool_result",
            Self::EightConversations => "eight_conversations",
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct Job {
    pub mode: Mode,
    pub shape: Shape,
    pub query_every: Option<usize>,
    pub fragments: usize,
    pub fragment_bytes: usize,
}
#[derive(Clone)]
pub(super) struct Mutation {
    pub conversation: ConversationId,
    pub expected: u64,
    pub batch: Uuid,
    pub event: AgentEvent,
}
impl Mutation {
    pub fn command(&self) -> journal::JournalCommand {
        journal::JournalCommand::new(
            self.conversation,
            self.batch,
            self.expected,
            vec![self.event.clone()],
            None,
            1_790_000_000_000,
        )
        .unwrap()
    }
}
pub(super) struct Dataset {
    pub conversations: Vec<ConversationId>,
    pub setup: Vec<Mutation>,
    pub measured: Vec<Mutation>,
}
struct Builder {
    conversation: ConversationId,
    events: Vec<Mutation>,
    seq: u64,
}
impl Builder {
    fn push(&mut self, payload: EventPayload) {
        let seq = self.seq + 1;
        self.events.push(Mutation {
            conversation: self.conversation,
            expected: self.seq,
            batch: Uuid::new_v5(self.conversation.as_uuid(), &seq.to_le_bytes()),
            event: AgentEvent::new(self.conversation, seq, seq, Uuid::from_u128(42), payload)
                .unwrap(),
        });
        self.seq = seq;
    }
    fn turn(&mut self, number: u128, parts: Vec<String>, tools: usize, result_bytes: usize) {
        let turn = Uuid::from_u128(number * 100 + 1);
        let model = ModelIdentity {
            provider_id: "fixture".into(),
            model_id: "synthetic".into(),
            revision: None,
        };
        self.push(EventPayload::TurnStarted {
            turn_id: turn,
            context_snapshot_id: Uuid::from_u128(number * 100 + 2),
            model: model.clone(),
            task_kind: TaskKind::Regular,
        });
        self.push(EventPayload::PhaseChanged {
            turn_id: turn,
            phase: TurnPhase::RunningModel,
        });
        for round in 0..=tools {
            let attempt = Uuid::from_u128(number * 100 + 10 + round as u128);
            self.push(EventPayload::ModelRequestStarted {
                turn_id: turn,
                attempt_id: attempt,
                model: model.clone(),
            });
            self.push(EventPayload::ModelResponseStarted {
                turn_id: turn,
                attempt_id: attempt,
                response_id: format!("synthetic-{number}-{round}"),
                model: Some(model.clone()),
            });
            let kind;
            let body;
            let item = Uuid::from_u128(number * 100 + 30 + round as u128);
            if round < tools {
                kind = ItemKind::ToolCall {
                    call_id: item,
                    name: "synthetic_read".into(),
                    provider_call_id: Some(format!("call-{number}-{round}")),
                    namespace: None,
                };
                body = serde_json::json!({"index":round}).to_string();
            } else {
                kind = ItemKind::AssistantMessage;
                self.push(EventPayload::ItemStarted {
                    turn_id: turn,
                    item_id: item,
                    kind: kind.clone(),
                });
                for part in &parts {
                    self.push(EventPayload::TextDelta {
                        turn_id: turn,
                        item_id: item,
                        text: part.clone(),
                    });
                }
                body = parts.concat();
            }
            self.push(EventPayload::ItemAccepted {
                turn_id: turn,
                item_id: item,
                kind,
                text: body,
                phase: (round == tools).then_some(AssistantMessagePhase::FinalAnswer),
                source: Some(CanonicalItemSource {
                    attempt_id: attempt,
                    ordinal: 0,
                    provider_item_id: Some(format!("item-{number}-{round}")),
                    provider_data_json: Some("{\"synthetic\":true}".into()),
                }),
            });
            self.push(EventPayload::ModelRequestFinished {
                turn_id: turn,
                attempt_id: attempt,
                input_tokens: Some(128),
                output_tokens: Some(64),
            });
            if round < tools {
                self.push(EventPayload::PhaseChanged {
                    turn_id: turn,
                    phase: TurnPhase::RunningTools,
                });
                self.push(EventPayload::ItemAccepted {
                    turn_id: turn,
                    item_id: Uuid::from_u128(number * 100 + 50 + round as u128),
                    kind: ItemKind::ToolResult {
                        call_id: item,
                        is_error: Some(false),
                    },
                    text: noise(number as u64 + round as u64, result_bytes),
                    phase: None,
                    source: None,
                });
                self.push(EventPayload::PhaseChanged {
                    turn_id: turn,
                    phase: TurnPhase::RunningModel,
                });
            }
        }
        self.push(EventPayload::TurnFinished {
            turn_id: turn,
            outcome: TurnOutcome::Completed,
        });
    }
}
pub(super) fn noise(seed: u64, bytes: usize) -> String {
    let mut value = seed.wrapping_add(0x9e3779b97f4a7c15);
    (0..bytes)
        .map(|_| {
            value ^= value << 13;
            value ^= value >> 7;
            value ^= value << 17;
            char::from(b'!' + (value % 94) as u8)
        })
        .collect()
}
pub(super) fn dataset(job: &Job) -> Dataset {
    let count = if job.shape == Shape::EightConversations {
        8
    } else {
        1
    };
    let mut setup = vec![];
    let mut streams = vec![];
    let mut conversations = vec![];
    for index in 0..count {
        let conversation = ConversationId::from_uuid(Uuid::from_u128(1000 + index as u128));
        conversations.push(conversation);
        let mut builder = Builder {
            conversation,
            events: vec![],
            seq: 0,
        };
        if job.shape == Shape::LongHistory {
            for old in 1..=32 {
                builder.turn(old, vec![noise(old as u64, 8192)], 0, 0);
            }
        }
        setup.append(&mut builder.events);
        let fragments = if job.shape == Shape::Short {
            16
        } else {
            job.fragments.div_ceil(count)
        };
        let bytes = if job.shape == Shape::Short {
            64
        } else {
            job.fragment_bytes
        };
        let parts = (0..fragments)
            .map(|part| noise(1_000_000 + index as u64 * 4096 + part as u64, bytes))
            .collect();
        builder.turn(
            100 + index as u128,
            parts,
            match job.shape {
                Shape::MultiTools => 4,
                Shape::LargeToolResult => 1,
                _ => 0,
            },
            if job.shape == Shape::LargeToolResult {
                1024 * 1024
            } else {
                4096
            },
        );
        streams.push(builder.events.into_iter());
    }
    let mut measured = vec![];
    loop {
        let before = measured.len();
        for stream in &mut streams {
            if let Some(event) = stream.next() {
                measured.push(event);
            }
        }
        if measured.len() == before {
            break;
        }
    }
    Dataset {
        conversations,
        setup,
        measured,
    }
}
