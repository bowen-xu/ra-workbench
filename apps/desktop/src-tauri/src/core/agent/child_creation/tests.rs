//! Creation-phase proofs and recovery identities without a model or spawn RPC.
use super::*;
use crate::agent::repository::SqliteConversationCatalog;
use crate::data_interface::{Database, SharedDatabase};
use research_agent_core::conversation::{BindingKind, CreateBoundStatus};
use rusqlite::Connection;
use serde_json::{Value, json};
use std::path::Path;

#[path = "tests/recovery.rs"]
mod recovery;
#[path = "tests/wire.rs"]
mod wire;

const RESOURCE: &str = " note ";

#[test]
fn writing_tool_child_identity_is_rebuilt_from_the_accepted_call() {
    use research_agent_core::protocol::ItemKind;
    let turn = Uuid::from_u128(70);
    let provider_id = "call:复现/1";
    let accepted = ItemKind::ToolCall {
        call_id: Uuid::from_u128(71),
        name: "research_spawn_subtask".into(),
        provider_call_id: Some(provider_id.into()),
        namespace: None,
    };
    let restored: ItemKind =
        serde_json::from_value(serde_json::to_value(accepted).unwrap()).unwrap();
    let ItemKind::ToolCall {
        provider_call_id: Some(restored),
        ..
    } = restored
    else {
        panic!()
    };
    let child = writing_tool_child_id(turn, provider_id);
    assert_eq!(child, writing_tool_child_id(turn, &restored));
    assert_ne!(child, writing_tool_child_id(turn, "call:复现/2"));
    assert_ne!(
        child,
        writing_tool_child_id(Uuid::from_u128(72), provider_id)
    );
    assert_eq!(
        intent(child, command(Host::Writing).binding, Host::Writing),
        intent(
            writing_tool_child_id(turn, &restored),
            command(Host::Writing).binding,
            Host::Writing
        )
    );
}

#[test]
fn paper_and_note_tool_child_identities_are_stable_and_host_scoped() {
    let turn = Uuid::from_u128(70);
    let call = "same-call";
    let paper = paper_tool_child_id(turn, call);
    let note = note_tool_child_id(turn, call);
    assert_eq!(paper, paper_tool_child_id(turn, call));
    assert_eq!(note, note_tool_child_id(turn, call));
    assert_ne!(paper, note);
    assert_ne!(paper, writing_tool_child_id(turn, call));
    assert_ne!(note, note_tool_child_id(turn, "another-call"));
    assert_ne!(note, note_tool_child_id(Uuid::from_u128(71), call));
}
fn command(host: Host) -> CreateBoundConversation {
    intent(
        Uuid::from_u128(41),
        ConversationBinding {
            kind: BindingKind::Note,
            resource_id: RESOURCE.into(),
        },
        host,
    )
}
fn open(root: &Path) -> SharedDatabase {
    let mut sql = Connection::open(root.join("child-creation.sqlite3")).unwrap();
    crate::storage::migrate(&mut sql).unwrap();
    sql.execute("insert into note_pages(id,title,created_at,updated_at) values(?1,'Synthetic note','now','now') on conflict do nothing",[RESOURCE]).unwrap();
    Database::open(sql, &root.join("data-log")).unwrap()
}
fn restored_intent(value: &Value) -> CreateBoundConversation {
    let saved: CreationIntent = serde_json::from_value(value["creation"].clone()).unwrap();
    CreateBoundConversation {
        operation_id: saved.operation_id,
        id: ConversationId::from_uuid(saved.id),
        binding: saved.binding,
    }
}

#[test]
fn child_creation_phases_are_stable_distinct_and_preserve_raw_binding() {
    let writing = command(Host::Writing);
    let paper = command(Host::Paper);
    let note = command(Host::Note);
    assert_eq!(writing, command(Host::Writing));
    assert_eq!(paper, command(Host::Paper));
    assert_eq!(note, command(Host::Note));
    for (command, phase) in [
        (&writing, b"agent.writing_child.create_bound:v1".as_slice()),
        (&paper, b"agent.reading_child.create_bound:v1".as_slice()),
        (&note, b"agent.note_child.create_bound:v1".as_slice()),
    ] {
        assert_eq!(
            command.operation_id,
            Uuid::new_v5(command.id.as_uuid(), phase)
        );
        assert_ne!(command.operation_id, *command.id.as_uuid());
        assert_eq!(command.binding.resource_id, RESOURCE);
    }
    assert_ne!(writing.operation_id, paper.operation_id);
    assert_ne!(writing.operation_id, note.operation_id);
    assert_ne!(paper.operation_id, note.operation_id);
}

#[tokio::test]
async fn managed_child_creation_returns_its_phase_proof_and_replays_without_submitting() {
    let root = tempfile::tempdir().unwrap();
    let db = open(root.path());
    let catalog = SqliteConversationCatalog::new(db.clone());
    let command = command(Host::Note);
    let mut proof = create(&catalog, &command).await.unwrap();
    assert_eq!(proof.operation_id, command.operation_id);
    assert_eq!(proof.conversation_id, command.id);
    assert_eq!(proof.sql_fence, 0);
    assert!(!proof.replayed);
    proof.replayed = true;
    assert_eq!(create(&catalog, &command).await.unwrap(), proof);
    assert_eq!(
        catalog.create_bound_status(command.clone()).await.unwrap(),
        CreateBoundStatus::Committed(proof)
    );
    assert_eq!(
        catalog
            .read(command.id, Some(command.binding))
            .await
            .unwrap()
            .durable_count,
        0
    );
    db.shutdown().unwrap();
}
