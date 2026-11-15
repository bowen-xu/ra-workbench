use super::*;
use crate::agent::repository::SqliteConversationCatalog;
use research_agent_core::conversation::{
    BindingKind, CatalogError, ConversationBinding, ConversationForkRequest,
};
use std::sync::{Arc, Mutex};

#[test]
fn root_checkpoint_identity_collision_is_rejected_without_publication() {
    let (mut db, batch) = setup();
    let child = Uuid::new_v4();
    let tx = db.transaction().unwrap();
    tx.execute(
        "insert into agent_conversations(id,title) values (?1,'child')",
        [child.to_string()],
    )
    .unwrap();
    assert_eq!(
        inherit_for_fork(
            &tx,
            batch.conversation_id,
            child,
            7,
            std::collections::HashMap::from([(batch.window_id, Uuid::new_v4())])
        ),
        Err(CatalogError::Corrupt)
    );
    tx.rollback().unwrap();
    let exists: bool = db
        .query_row(
            "select exists(select 1 from agent_conversations where id=?1)",
            [child.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    assert!(!exists);
}

#[tokio::test]
async fn installed_window_prompts_follow_the_remapped_child_checkpoint() {
    installed_window_fixture(false).await;
}

#[tokio::test]
async fn mismatched_metadata_identity_rolls_back_child_publication() {
    installed_window_fixture(true).await;
}

async fn installed_window_fixture(corrupt_identity: bool) {
    use research_agent_core::context::{
        checkpoint::{CanonicalContext, ContextCheckpoint, StoredMessage},
        compaction_strategy::{
            CompactionBinding, CompactionKind, CompactionOutput, CompactionPhase,
        },
    };
    let (mut db, mut batch) = setup();
    let root = batch.window_id;
    record(&mut db, &batch).unwrap();
    let mut checkpoint = ContextCheckpoint {
        schema_version: 3,
        window_id: Uuid::now_v7(),
        conversation_id: batch.conversation_id,
        turn_id: batch.turn_id,
        source_watermark: 7,
        previous_window_id: None,
        binding: CompactionBinding {
            model: context_windows::read_projection(&db, batch.conversation_id)
                .unwrap()
                .turn(batch.turn_id)
                .unwrap()
                .model
                .clone(),
            strategy_id: "fixture.reset".into(),
            state_schema: 1,
            kind: CompactionKind::ResetWindow,
        },
        credential_scope_id: None,
        phase: CompactionPhase::MidTurn,
        output: CompactionOutput::ResetWindow,
        retained: vec![],
        canonical_context: Some(CanonicalContext {
            before_retained: 0,
            messages: vec![StoredMessage::Developer {
                text: "canonical".into(),
            }],
        }),
    };
    let research_agent_core::model::ModelMessage::Developer(metadata) =
        research_agent_core::context::token_budget::TokenBudgetContext {
            conversation_id: if corrupt_identity {
                Uuid::new_v4()
            } else {
                checkpoint.conversation_id
            },
            first_window_id: root,
            previous_window_id: Some(root),
            window_id: checkpoint.window_id,
            thread_hint: Some("saved thread notes".into()),
        }
        .into_message()
    else {
        panic!("developer role expected")
    };
    checkpoint
        .canonical_context
        .as_mut()
        .unwrap()
        .messages
        .push(StoredMessage::Developer { text: metadata });
    context_windows::install(&mut db, &checkpoint).unwrap();
    let anchor = Uuid::new_v4();
    append(
        &mut db,
        batch.conversation_id,
        7,
        EventPayload::ItemStarted {
            turn_id: batch.turn_id,
            item_id: anchor,
            kind: ItemKind::AssistantMessage,
        },
    );
    append(
        &mut db,
        batch.conversation_id,
        8,
        EventPayload::ItemCompleted {
            turn_id: batch.turn_id,
            item_id: anchor,
        },
    );
    batch.anchor_item_id = anchor;
    batch.window_id = checkpoint.window_id;
    batch.boundary_seq = 9;
    batch.fragments = vec!["new window prompt".into()];
    record(&mut db, &batch).unwrap();
    append(
        &mut db,
        batch.conversation_id,
        9,
        EventPayload::TurnFinished {
            turn_id: batch.turn_id,
            outcome: TurnOutcome::Completed,
        },
    );
    db.execute(
        "insert into note_pages(id,title,created_at,updated_at) values ('note','Note','now','now')",
        [],
    )
    .unwrap();
    db.execute("insert into agent_conversation_bindings(conversation_id,resource_kind,resource_id) values (?1,'note','note')", [batch.conversation_id.to_string()]).unwrap();
    let db = Arc::new(Mutex::new(db));
    let catalog = SqliteConversationCatalog::new(db.clone());
    let child = Uuid::new_v4();
    let request = ConversationForkRequest {
        parent_id: ConversationId::from_uuid(batch.conversation_id),
        child_id: ConversationId::from_uuid(child),
        through_turn_id: batch.turn_id,
        binding: ConversationBinding {
            kind: BindingKind::Note,
            resource_id: "note".into(),
        },
    };
    if corrupt_identity {
        assert_eq!(
            catalog.fork_legacy_for_tests(request).await.unwrap_err(),
            CatalogError::Corrupt
        );
        let guard = db.lock().unwrap();
        let exists: bool = guard
            .query_row(
                "select exists(select 1 from agent_conversations where id=?1)",
                [child.to_string()],
                |row| row.get(0),
            )
            .unwrap();
        assert!(!exists);
        return;
    }
    {
        let guard = db.lock().unwrap();
        guard.execute_batch("create trigger fail_child_checkpoint before insert on agent_context_checkpoints begin select raise(abort, 'fixture checkpoint write failure'); end;").unwrap();
    }
    assert_eq!(
        catalog
            .fork_legacy_for_tests(request.clone())
            .await
            .unwrap_err(),
        CatalogError::Unavailable
    );
    {
        let guard = db.lock().unwrap();
        let exists: bool = guard
            .query_row(
                "select exists(select 1 from agent_conversations where id=?1)",
                [child.to_string()],
                |row| row.get(0),
            )
            .unwrap();
        assert!(!exists);
        guard
            .execute_batch("drop trigger fail_child_checkpoint")
            .unwrap();
    }
    catalog.fork_legacy_for_tests(request).await.unwrap();
    let mut db = db.lock().unwrap();
    let identity =
        context_windows::identity::read_or_create(&mut db, child, Uuid::now_v7()).unwrap();
    assert_eq!(identity.number, 1);
    assert_ne!(identity.first, root);
    assert_ne!(identity.current, checkpoint.window_id);
    let active = context_windows::read_active(&db, child).unwrap().unwrap();
    let restored = active.restore_reset_window(&|_| None).unwrap();
    let metadata = restored
        .iter()
        .find_map(|message| match message {
            research_agent_core::model::ModelMessage::Developer(body) => {
                research_agent_core::context::token_budget::TokenBudgetContext::from_body(body)
                    .unwrap()
            }
            _ => None,
        })
        .unwrap();
    assert_eq!(metadata.conversation_id, child);
    assert_eq!(metadata.first_window_id, identity.first);
    assert_eq!(metadata.window_id, identity.current);
    assert_eq!(metadata.previous_window_id, Some(identity.first));
    assert_eq!(metadata.thread_hint.as_deref(), Some("saved thread notes"));
    let current = read(&db, child, identity.current, active.source_watermark).unwrap();
    assert_eq!(current.len(), 1);
    assert_eq!(current[0].fragments, vec!["new window prompt"]);
    assert_eq!(read(&db, child, identity.first, 0).unwrap().len(), 1);
    assert!(
        read(&db, child, checkpoint.window_id, 0)
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn initial_window_prompts_fork_atomically_with_child_owned_identity_and_exact_replay() {
    let (mut db, batch) = setup();
    record(&mut db, &batch).unwrap();
    append(
        &mut db,
        batch.conversation_id,
        7,
        EventPayload::TurnFinished {
            turn_id: batch.turn_id,
            outcome: TurnOutcome::Completed,
        },
    );
    db.execute(
        "insert into note_pages(id,title,created_at,updated_at) values ('note','Note','now','now')",
        [],
    )
    .unwrap();
    db.execute("insert into agent_conversation_bindings(conversation_id,resource_kind,resource_id) values (?1,'note','note')", [batch.conversation_id.to_string()]).unwrap();
    let later_turn = Uuid::new_v4();
    let later_item = Uuid::new_v4();
    let model = context_windows::read_projection(&db, batch.conversation_id)
        .unwrap()
        .turn(batch.turn_id)
        .unwrap()
        .model
        .clone();
    append(
        &mut db,
        batch.conversation_id,
        8,
        EventPayload::TurnStarted {
            turn_id: later_turn,
            task_kind: TaskKind::Regular,
            context_snapshot_id: Uuid::new_v4(),
            model,
        },
    );
    append(
        &mut db,
        batch.conversation_id,
        9,
        EventPayload::ItemStarted {
            turn_id: later_turn,
            item_id: later_item,
            kind: ItemKind::AssistantMessage,
        },
    );
    append(
        &mut db,
        batch.conversation_id,
        10,
        EventPayload::ItemCompleted {
            turn_id: later_turn,
            item_id: later_item,
        },
    );
    let mut later = batch.clone();
    later.turn_id = later_turn;
    later.anchor_item_id = later_item;
    later.boundary_seq = 11;
    later.fragments = vec!["future prompt".into()];
    record(&mut db, &later).unwrap();
    append(
        &mut db,
        batch.conversation_id,
        11,
        EventPayload::TurnFinished {
            turn_id: later_turn,
            outcome: TurnOutcome::Completed,
        },
    );
    db.execute_batch("create trigger fail_fork_fragments before insert on agent_context_fragments begin select raise(abort, 'fixture fork write failure'); end;").unwrap();
    let db = Arc::new(Mutex::new(db));
    let catalog = SqliteConversationCatalog::new(db.clone());
    let child = Uuid::new_v4();
    let request = ConversationForkRequest {
        parent_id: ConversationId::from_uuid(batch.conversation_id),
        child_id: ConversationId::from_uuid(child),
        through_turn_id: batch.turn_id,
        binding: ConversationBinding {
            kind: BindingKind::Note,
            resource_id: "note".into(),
        },
    };
    assert_eq!(
        catalog
            .fork_legacy_for_tests(request.clone())
            .await
            .unwrap_err(),
        CatalogError::Unavailable
    );
    {
        let guard = db.lock().unwrap();
        let exists: bool = guard
            .query_row(
                "select exists(select 1 from agent_conversations where id=?1)",
                [child.to_string()],
                |row| row.get(0),
            )
            .unwrap();
        assert!(!exists);
        guard
            .execute_batch("drop trigger fail_fork_fragments")
            .unwrap();
    }
    catalog
        .fork_legacy_for_tests(request.clone())
        .await
        .unwrap();
    let changes = {
        let mut guard = db.lock().unwrap();
        let identity =
            context_windows::identity::read_or_create(&mut guard, child, Uuid::now_v7()).unwrap();
        assert_ne!(identity.first, batch.window_id);
        let inherited = read(&guard, child, identity.current, 0).unwrap();
        let mut expected = batch.clone();
        expected.conversation_id = child;
        expected.window_id = identity.current;
        assert!(inherited == vec![expected]);
        assert!(read(&guard, child, batch.window_id, 0).unwrap().is_empty());
        guard.total_changes()
    };
    catalog.fork_legacy_for_tests(request).await.unwrap();
    assert_eq!(db.lock().unwrap().total_changes(), changes);
}
