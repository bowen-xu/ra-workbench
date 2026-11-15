use super::*;
use crate::agent::repository::SqliteConversationCatalog;
use research_agent_core::conversation::{
    BindingKind, CatalogError, ConversationBinding, ConversationForkRequest,
};
use research_agent_core::protocol::{ItemKind, TurnOutcome};
use std::sync::{Arc, Mutex};

fn finish(db: &mut Connection, request: &CompactionHookRequest) {
    let user = Uuid::new_v4();
    let seq = context_windows::read_projection(db, request.conversation_id)
        .unwrap()
        .seq();
    append(
        db,
        request.conversation_id,
        seq,
        EventPayload::ItemStarted {
            turn_id: request.turn_id,
            item_id: user,
            kind: ItemKind::UserMessage,
        },
    );
    append(
        db,
        request.conversation_id,
        seq + 1,
        EventPayload::ItemCompleted {
            turn_id: request.turn_id,
            item_id: user,
        },
    );
    record_result(db, request, &result(), 2).unwrap();
    append(
        db,
        request.conversation_id,
        seq + 2,
        EventPayload::TurnFinished {
            turn_id: request.turn_id,
            outcome: TurnOutcome::Canceled,
        },
    );
}

async fn verify(fault: bool) {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("fork.sqlite");
    let (mut db, first) = setup(&path);
    finish(&mut db, &first);
    let mut late = first.clone();
    late.turn_id = Uuid::new_v4();
    append(
        &mut db,
        first.conversation_id,
        4,
        EventPayload::TurnStarted {
            turn_id: late.turn_id,
            task_kind: TaskKind::Regular,
            context_snapshot_id: Uuid::new_v4(),
            model: first.model.clone(),
        },
    );
    super::super::super::record(&db, &late, CompactionHookNotification::Started, 3).unwrap();
    finish(&mut db, &late);
    db.execute("insert into note_pages(id,title,created_at,updated_at) values('hook-note','Note','now','now')", []).unwrap();
    db.execute("insert into agent_conversation_bindings(conversation_id,resource_kind,resource_id) values(?1,'note','hook-note')", [first.conversation_id.to_string()]).unwrap();
    if fault {
        db.execute_batch("create trigger fail_inherited_hook before insert on agent_inherited_hook_contexts begin select raise(abort,'fixture'); end;").unwrap();
    }
    let db = Arc::new(Mutex::new(db));
    let catalog = SqliteConversationCatalog::new(db.clone());
    let child = Uuid::new_v4();
    let request = ConversationForkRequest {
        child_id: ConversationId::from_uuid(child),
        parent_id: ConversationId::from_uuid(first.conversation_id),
        through_turn_id: first.turn_id,
        binding: ConversationBinding {
            kind: BindingKind::Note,
            resource_id: "hook-note".into(),
        },
    };
    let outcome = catalog.fork_legacy_for_tests(request.clone()).await;
    if fault {
        assert!(matches!(outcome, Err(CatalogError::Unavailable)));
        let db = db.lock().unwrap();
        let count: u64 = db
            .query_row(
                "select count(*) from agent_conversations where id=?1",
                [child.to_string()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);
        assert_eq!(read_current(&db, first.conversation_id).unwrap().len(), 2);
        return;
    }
    outcome.unwrap();
    catalog.fork_legacy_for_tests(request).await.unwrap();
    let grandchild = Uuid::new_v4();
    catalog
        .fork_legacy_for_tests(ConversationForkRequest {
            child_id: ConversationId::from_uuid(grandchild),
            parent_id: ConversationId::from_uuid(child),
            through_turn_id: first.turn_id,
            binding: ConversationBinding {
                kind: BindingKind::Note,
                resource_id: "hook-note".into(),
            },
        })
        .await
        .unwrap();
    {
        let db = db.lock().unwrap();
        for id in [child, grandchild] {
            let batches = read_current(&db, id).unwrap();
            assert_eq!(batches.len(), 1, "late parent hook is outside fork cutoff");
            assert_eq!(batches[0].contexts, result().additional_contexts);
            assert_eq!(batches[0].turn_id, first.turn_id);
            assert_ne!(
                context_windows::read_active(&db, id)
                    .unwrap()
                    .unwrap()
                    .window_id,
                first.operation_id
            );
            let audits: u64 = db
                .query_row(
                    "select count(*) from agent_compaction_hook_audit where conversation_id=?1",
                    [id.to_string()],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(audits, 0, "inheritance does not claim hook execution");
        }
        db.execute(
            "delete from agent_compact_hook_contexts where conversation_id=?1",
            [first.conversation_id.to_string()],
        )
        .unwrap();
    }
    drop(catalog);
    drop(db);
    let reopened = Connection::open(path).unwrap();
    assert_eq!(
        read_current(&reopened, grandchild).unwrap()[0].contexts,
        result().additional_contexts
    );
}

#[tokio::test]
async fn fork_hook_context_is_independent_cutoff_bound_and_nested() {
    verify(false).await;
}

#[tokio::test]
async fn failed_hook_inheritance_rolls_back_the_entire_child() {
    verify(true).await;
}
