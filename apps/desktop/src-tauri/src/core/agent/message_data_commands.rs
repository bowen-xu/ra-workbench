//! Read-only message inspection. No execution, provider or credential access.
use super::export_commands::ExportCommandError;
use crate::storage::AppStore;
use research_agent_core::protocol::AgentEvent;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tauri::State;
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MessageDataRequest {
    conversation_id: Uuid,
    turn_id: Uuid,
    item_id: Uuid,
    #[serde(default)]
    references_only: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageData {
    schema_version: u16,
    conversation_id: Uuid,
    turn_id: Uuid,
    item_id: Uuid,
    events: Vec<AgentEvent>,
    frozen_submission: Option<Value>,
    user_input: Vec<research_agent_core::user_input::UserInput>,
}

fn read(db: &Connection, request: MessageDataRequest) -> Result<MessageData, ExportCommandError> {
    use ExportCommandError::{Corrupt, InvalidInput, NotFound, TooLarge, Unavailable};
    if request.conversation_id.is_nil() || request.turn_id.is_nil() || request.item_id.is_nil() {
        return Err(InvalidInput);
    }
    let mut query = db
        .prepare(
            "select schema_version,payload_json from agent_items where conversation_id=?1
         and json_extract(payload_json,'$.payload.turnId')=?2
         and json_extract(payload_json,'$.payload.itemId')=?3 order by ordinal limit 50001",
        )
        .map_err(|_| Unavailable)?;
    let rows = query
        .query_map(
            params![
                request.conversation_id.to_string(),
                request.turn_id.to_string(),
                request.item_id.to_string()
            ],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
        )
        .map_err(|_| Unavailable)?;
    let mut events = Vec::new();
    let mut bytes = 0;
    let mut found_item = false;
    for row in rows {
        let (version, raw) = row.map_err(|_| Unavailable)?;
        bytes += raw.len();
        if bytes > 8 * 1024 * 1024 || events.len() >= 50000 {
            return Err(TooLarge);
        }
        if version != 1 {
            return Err(Corrupt);
        }
        let event: AgentEvent = serde_json::from_str(&raw).map_err(|_| Corrupt)?;
        let value = serde_json::to_value(&event).map_err(|_| Corrupt)?;
        if value["conversationId"] != request.conversation_id.to_string()
            || value["payload"]["turnId"] != request.turn_id.to_string()
            || value["payload"]["itemId"] != request.item_id.to_string()
        {
            return Err(Corrupt);
        }
        found_item |= matches!(
            value["payload"]["type"].as_str(),
            Some("itemStarted" | "itemAccepted")
        );
        events.push(event);
    }
    if !found_item {
        return Err(NotFound);
    }
    let is_user = events.iter().any(|event| {
        matches!(
            event.payload(),
            research_agent_core::protocol::EventPayload::ItemStarted {
                kind: research_agent_core::protocol::ItemKind::UserMessage,
                ..
            }
        )
    });
    let mut user_input = if is_user {
        super::repository::steering::resources::read(db, request.item_id)
            .map_err(|_| Corrupt)?
            .0
    } else {
        vec![]
    };
    let request_id: Option<String> = db
        .query_row(
            "select request_id from agent_submissions where conversation_id=?1 and turn_id=?2",
            params![
                request.conversation_id.to_string(),
                request.turn_id.to_string()
            ],
            |row| row.get(0),
        )
        .optional()
        .map_err(|_| Unavailable)?;
    let frozen_submission = if let Some(id) = request_id {
        let id = Uuid::parse_str(&id).map_err(|_| Corrupt)?;
        let record = super::repository::submissions::read(db, request.conversation_id, id)
            .map_err(|_| Corrupt)?
            .ok_or(Corrupt)?;
        if record.turn_id != request.turn_id {
            return Err(Corrupt);
        }
        if is_user && user_input.is_empty() {
            let first_user: Option<String> = db
                .query_row(
                    "select json_extract(payload_json,'$.payload.itemId') from agent_items
                 where conversation_id=?1 and json_extract(payload_json,'$.payload.turnId')=?2
                 and json_extract(payload_json,'$.payload.type')='itemStarted'
                 and json_extract(payload_json,'$.payload.kind.type')='userMessage'
                 order by ordinal limit 1",
                    params![
                        request.conversation_id.to_string(),
                        request.turn_id.to_string()
                    ],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|_| Unavailable)?;
            if first_user.as_deref() == Some(request.item_id.to_string().as_str()) {
                user_input = record.input.user_input.clone();
            }
        }
        if request.references_only {
            None
        } else {
            Some(
                json!({"requestId": record.request_id, "contextSnapshotId": record.context_snapshot_id, "input": record.input}),
            )
        }
    } else {
        None
    };
    if serde_json::to_vec(&frozen_submission)
        .map_err(|_| Corrupt)?
        .len()
        + bytes
        + serde_json::to_vec(&user_input).map_err(|_| Corrupt)?.len()
        > 8 * 1024 * 1024
    {
        return Err(TooLarge);
    }
    Ok(MessageData {
        schema_version: 1,
        conversation_id: request.conversation_id,
        turn_id: request.turn_id,
        item_id: request.item_id,
        events,
        frozen_submission,
        user_input,
    })
}

#[tauri::command]
pub(crate) async fn agent_read_message_data(
    store: State<'_, AppStore>,
    request: MessageDataRequest,
) -> Result<MessageData, ExportCommandError> {
    execute(store.inner(), request).await
}

pub(crate) async fn execute(
    store: &AppStore,
    request: MessageDataRequest,
) -> Result<MessageData, ExportCommandError> {
    let store = store.clone();
    tokio::task::spawn_blocking(move || {
        let db = store
            .connection()
            .map_err(|_| ExportCommandError::Unavailable)?;
        read(&db, request)
    })
    .await
    .map_err(|_| ExportCommandError::Unavailable)?
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request() -> MessageDataRequest {
        MessageDataRequest {
            conversation_id: Uuid::from_u128(1),
            turn_id: Uuid::from_u128(2),
            item_id: Uuid::from_u128(3),
            references_only: false,
        }
    }
    fn fixture() -> Connection {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("create table agent_items(conversation_id text,ordinal integer,schema_version integer,payload_json text);
            create table agent_submissions(conversation_id text,request_id text,turn_id text,context_snapshot_id text,schema_version integer,input_json text);
            create table agent_steering_resources(input_id text,input_json text,skill_instructions_json text);").unwrap();
        for (seq, payload) in [
            json!({"type":"itemStarted","turnId":Uuid::from_u128(2),"itemId":Uuid::from_u128(3),"kind":{"type":"userMessage"}}),
            json!({"type":"textDelta","turnId":Uuid::from_u128(2),"itemId":Uuid::from_u128(3),"text":"合成输入🧪"}),
        ].into_iter().enumerate() {
            let event = json!({"schemaVersion":1,"conversationId":Uuid::from_u128(1),"requestId":Uuid::from_u128(4),"seq":seq+1,"timestampMs":1,"payload":payload});
            db.execute("insert into agent_items values(?1,?2,1,?3)", params![Uuid::from_u128(1).to_string(), seq as i64, event.to_string()]).unwrap();
        }
        db
    }
    #[test]
    fn reads_exact_item_without_writes_or_inventing_legacy_snapshot() {
        let db = fixture();
        let changes = db.total_changes();
        let data = read(&db, request()).unwrap();
        assert_eq!(data.events.len(), 2);
        assert!(data.frozen_submission.is_none());
        assert!(data.user_input.is_empty());
        assert_eq!(db.total_changes(), changes);
        for wrong in [
            MessageDataRequest {
                conversation_id: Uuid::from_u128(5),
                ..request()
            },
            MessageDataRequest {
                turn_id: Uuid::from_u128(5),
                ..request()
            },
            MessageDataRequest {
                item_id: Uuid::from_u128(5),
                ..request()
            },
        ] {
            assert!(matches!(
                read(&db, wrong),
                Err(ExportCommandError::NotFound)
            ));
        }
    }
    #[test]
    fn reads_atomic_item_without_requiring_a_draft_start() {
        let db = fixture();
        db.execute("delete from agent_items", []).unwrap();
        let event = json!({"schemaVersion":1,"conversationId":Uuid::from_u128(1),
            "requestId":Uuid::from_u128(4),"seq":1,"timestampMs":1,
            "payload":{"type":"itemAccepted","turnId":Uuid::from_u128(2),
            "itemId":Uuid::from_u128(3),"kind":{"type":"toolCall","callId":Uuid::from_u128(6),
            "name":"research_list_sources","providerCallId":"fixture_call"},"text":"{}"}});
        db.execute(
            "insert into agent_items values(?1,0,1,?2)",
            params![Uuid::from_u128(1).to_string(), event.to_string()],
        )
        .unwrap();
        let changes = db.total_changes();
        let data = read(&db, request()).unwrap();
        assert_eq!(data.events.len(), 1);
        assert!(data.frozen_submission.is_none());
        assert!(data.user_input.is_empty());
        assert_eq!(db.total_changes(), changes);
    }
    #[test]
    fn reference_only_read_preserves_exact_initial_and_steer_identity() {
        let db = fixture();
        let input = |server: &str| {
            json!([
                {"type":"text","text":"合成输入🧪","textElements":[]},
                {"type":"mention","name":"同名工具","target":{"kind":"tool","serverId":server,"environmentId":"fixture","revision":"v1"}}
            ])
        };
        let frozen = json!({"text":"合成输入🧪","model":{"providerId":"fixture","modelId":"test","revision":"v1"},
            "reasoningEffort":"high","context":[],"userInput":input("first")});
        db.execute(
            "insert into agent_submissions values(?1,?2,?3,?4,1,?5)",
            params![
                Uuid::from_u128(1).to_string(),
                Uuid::from_u128(4).to_string(),
                Uuid::from_u128(2).to_string(),
                Uuid::from_u128(5).to_string(),
                frozen.to_string()
            ],
        )
        .unwrap();
        for item in [6, 7] {
            let event = json!({"schemaVersion":1,"conversationId":Uuid::from_u128(1),"requestId":Uuid::from_u128(4),
                "seq":item,"timestampMs":1,"payload":{"type":"itemStarted","turnId":Uuid::from_u128(2),"itemId":Uuid::from_u128(item),"kind":{"type":"userMessage"}}});
            db.execute(
                "insert into agent_items values(?1,?2,1,?3)",
                params![
                    Uuid::from_u128(1).to_string(),
                    item as i64,
                    event.to_string()
                ],
            )
            .unwrap();
        }
        db.execute(
            "insert into agent_steering_resources values(?1,?2,'[]')",
            params![Uuid::from_u128(6).to_string(), input("steer").to_string()],
        )
        .unwrap();
        let changes = db.total_changes();
        for (id, expected) in [
            (3, Some(input("first"))),
            (6, Some(input("steer"))),
            (7, None),
        ] {
            let data = read(
                &db,
                MessageDataRequest {
                    item_id: Uuid::from_u128(id),
                    references_only: true,
                    ..request()
                },
            )
            .unwrap();
            assert!(data.frozen_submission.is_none());
            match expected {
                Some(input) => assert_eq!(serde_json::to_value(data.user_input).unwrap(), input),
                None => assert!(data.user_input.is_empty()),
            }
        }
        assert_eq!(db.total_changes(), changes);
    }
    #[test]
    fn returns_original_validated_submission_without_provider_access() {
        let db = fixture();
        let input = json!({"text":"frozen question","model":{"providerId":"fixture","modelId":"test","revision":"v1"},"reasoningEffort":"high",
            "context":[{"sourceId":"S1","kind":"manuscript","resourceId":"main.tex","revision":"buffer:42","text":"original frozen source"}]});
        db.execute(
            "insert into agent_submissions values(?1,?2,?3,?4,1,?5)",
            params![
                Uuid::from_u128(1).to_string(),
                Uuid::from_u128(4).to_string(),
                Uuid::from_u128(2).to_string(),
                Uuid::from_u128(5).to_string(),
                input.to_string()
            ],
        )
        .unwrap();
        let changes = db.total_changes();
        let data = read(&db, request()).unwrap();
        let frozen = data.frozen_submission.unwrap();
        assert_eq!(frozen["input"]["text"], "frozen question");
        assert_eq!(
            frozen["input"]["context"][0]["text"],
            "original frozen source"
        );
        assert_eq!(db.total_changes(), changes);
    }
    #[test]
    fn rejects_nil_unknown_fields_and_corrupt_events() {
        assert!(matches!(
            read(
                &fixture(),
                MessageDataRequest {
                    item_id: Uuid::nil(),
                    ..request()
                }
            ),
            Err(ExportCommandError::InvalidInput)
        ));
        assert!(serde_json::from_value::<MessageDataRequest>(json!({"conversationId":Uuid::from_u128(1),"turnId":Uuid::from_u128(2),"itemId":Uuid::from_u128(3),"resume":true})).is_err());
        let db = fixture();
        db.execute(
            "update agent_items set schema_version=2 where ordinal=0",
            [],
        )
        .unwrap();
        assert!(matches!(
            read(&db, request()),
            Err(ExportCommandError::Corrupt)
        ));
    }
}
