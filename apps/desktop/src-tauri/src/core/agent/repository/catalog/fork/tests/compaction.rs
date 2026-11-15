//! Fork around two installed windows; storage is atomic and chain order is authoritative.
use super::*;
#[path = "compaction/prefill.rs"]
mod prefill;
use crate::agent::repository::context_windows::{self, Installation};
use research_agent_core::context::{
    checkpoint::{CanonicalContext, ContextCheckpoint, StoredContent, StoredMessage},
    compaction_strategy::{CompactionBinding, CompactionKind, CompactionOutput, CompactionPhase},
};

fn checkpoint(n: u128, output: CompactionOutput, previous: Option<Uuid>) -> ContextCheckpoint {
    let kind = match &output {
        CompactionOutput::ResetWindow => CompactionKind::ResetWindow,
        CompactionOutput::TextSummary(_) => CompactionKind::TextSummary,
        CompactionOutput::RemoteOpaque(_) => CompactionKind::RemoteOpaque,
        CompactionOutput::RemoteWindow(_) => CompactionKind::RemoteWindow,
    };
    ContextCheckpoint {
        schema_version: 2,
        window_id: Uuid::from_u128(n),
        conversation_id: *id(1).as_uuid(),
        turn_id: Uuid::from_u128(if previous.is_none() { 20 } else { 30 }),
        source_watermark: if previous.is_none() { 6 } else { 8 },
        previous_window_id: previous,
        binding: CompactionBinding {
            model: ModelIdentity {
                provider_id: "fixture".into(),
                model_id: "fixture".into(),
                revision: None,
            },
            strategy_id: "fixture.compact".into(),
            state_schema: 1,
            kind,
        },
        credential_scope_id: (kind != CompactionKind::TextSummary).then_some(Uuid::from_u128(500)),
        phase: CompactionPhase::MidTurn,
        output,
        retained: vec![],
        canonical_context: Some(CanonicalContext {
            before_retained: 0,
            messages: vec![StoredMessage::User {
                source_item_id: None,
                content: vec![StoredContent::Text {
                    text: "frozen canonical evidence".into(),
                }],
            }],
        }),
    }
}

fn finish(db: &mut Connection, seq: u64, turn: u128) {
    let payload = P::TurnFinished {
        turn_id: Uuid::from_u128(turn),
        outcome: TurnOutcome::Completed,
    };
    let event = AgentEvent::new(id(1), seq, 123, Uuid::new_v4(), payload).unwrap();
    append_batch(db, id(1), Uuid::new_v4(), seq - 1, &[event]).unwrap();
}

fn seed_windows(
    output: CompactionOutput,
) -> (
    Arc<Mutex<Connection>>,
    SqliteConversationCatalog,
    ContextCheckpoint,
    ContextCheckpoint,
) {
    let (db, catalog) = setup();
    let first = checkpoint(100, output.clone(), None);
    let second = checkpoint(101, output, Some(first.window_id));
    {
        let mut guard = db.lock().unwrap();
        crate::storage::migrate(&mut guard).unwrap();
        assert_eq!(
            context_windows::install(&mut guard, &first).unwrap(),
            Installation::Installed
        );
        finish(&mut guard, 7, 20);
        let event = AgentEvent::new(
            id(1),
            8,
            123,
            Uuid::new_v4(),
            P::TurnStarted {
                task_kind: research_agent_core::protocol::TaskKind::Regular,
                turn_id: Uuid::from_u128(30),
                context_snapshot_id: Uuid::from_u128(31),
                model: second.binding.model.clone(),
            },
        )
        .unwrap();
        append_batch(&mut guard, id(1), Uuid::new_v4(), 7, &[event]).unwrap();
        assert_eq!(
            context_windows::install(&mut guard, &second).unwrap(),
            Installation::Installed
        );
        finish(&mut guard, 9, 30);
    }
    (db, catalog, first, second)
}

#[tokio::test]
async fn historical_forks_choose_the_surviving_window_and_copy_complete_chain() {
    for output in [
        CompactionOutput::TextSummary("summary".into()),
        CompactionOutput::RemoteOpaque(
            serde_json::json!({"type":"compaction","encrypted_content":"opaque"}),
        ),
        CompactionOutput::RemoteWindow(vec![
            serde_json::json!({"type":"future_item","vendor_field":3}),
            serde_json::json!({"type":"compaction","encrypted_content":"opaque"}),
        ]),
    ] {
        let (db, catalog, first, second) = seed_windows(output);
        for (child, turn) in [(2, 10), (3, 20), (4, 30)] {
            let mut fork = request(child);
            fork.through_turn_id = Uuid::from_u128(turn);
            catalog.fork_legacy_for_tests(fork.clone()).await.unwrap();
            assert!(
                context_windows::session_start::take(&mut db.lock().unwrap(), *id(child).as_uuid())
                    .unwrap()
                    .is_none(),
                "fork must not replay the parent's pending compact hook"
            );
            let copied =
                context_windows::read_active(&db.lock().unwrap(), *id(child).as_uuid()).unwrap();
            if turn == 10 {
                assert!(copied.is_none());
                continue;
            }
            let copied = copied.unwrap();
            let expected = if turn == 20 { &first } else { &second };
            assert_ne!(copied.window_id, expected.window_id);
            assert_eq!(copied.conversation_id, *id(child).as_uuid());
            assert_eq!(copied.turn_id, expected.turn_id);
            assert_eq!(copied.source_watermark, expected.source_watermark);
            assert!(copied.output == expected.output);
            assert!(copied.canonical_context == expected.canonical_context);
            assert_eq!(copied.binding, expected.binding);
            assert_eq!(copied.credential_scope_id, expected.credential_scope_id);
            assert_eq!(copied.previous_window_id.is_some(), turn == 30);
            if let Some(previous) = copied.previous_window_id {
                assert_ne!(previous, first.window_id);
                let guard = db.lock().unwrap();
                let encoded:String=guard.query_row("select checkpoint_json from agent_context_checkpoints where conversation_id=?1 and id=?2",
                    params![id(child).as_uuid().to_string(),previous.to_string()],|row|row.get(0)).unwrap();
                let older: ContextCheckpoint = serde_json::from_str(&encoded).unwrap();
                assert!(older.output == first.output);
                assert!(older.previous_window_id.is_none());
            }
            let changes = db.lock().unwrap().total_changes();
            catalog.fork_legacy_for_tests(fork).await.unwrap();
            assert_eq!(changes, db.lock().unwrap().total_changes());
        }
        assert!(
            context_windows::read_active(&db.lock().unwrap(), *id(1).as_uuid())
                .unwrap()
                .unwrap()
                == second
        );
        let mut branch = request(5);
        branch.parent_id = id(4);
        branch.through_turn_id = Uuid::from_u128(20);
        catalog.fork_legacy_for_tests(branch).await.unwrap();
        let descendant = context_windows::read_active(&db.lock().unwrap(), *id(5).as_uuid())
            .unwrap()
            .unwrap();
        assert!(descendant.output == first.output);
        assert!(descendant.previous_window_id.is_none());
    }
}

#[tokio::test]
async fn failed_window_publication_rolls_back_child_and_corrupt_chain_is_rejected() {
    let (db, catalog, _, _) = seed_windows(CompactionOutput::TextSummary("summary".into()));
    db.lock().unwrap().execute_batch("create trigger reject_fork_window before insert on agent_active_context_windows when new.conversation_id != '00000000-0000-0000-0000-000000000001' begin select raise(abort,'injected'); end;").unwrap();
    let mut fork = request(2);
    fork.through_turn_id = Uuid::from_u128(30);
    assert!(catalog.fork_legacy_for_tests(fork.clone()).await.is_err());
    assert_eq!(
        catalog.read(id(2), None).await.unwrap_err(),
        CatalogError::NotFound
    );
    {
        let guard = db.lock().unwrap();
        let count: u64 = guard
            .query_row(
                "select count(*) from agent_context_checkpoints where conversation_id=?1",
                [id(2).as_uuid().to_string()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);
        guard.execute_batch("drop trigger reject_fork_window; drop trigger agent_context_checkpoints_immutable; update agent_context_checkpoints set checkpoint_json=json_set(checkpoint_json,'$.previous_window_id','00000000-0000-0000-0000-0000000003e7'),previous_window_id='00000000-0000-0000-0000-0000000003e7' where id='00000000-0000-0000-0000-000000000065';").unwrap();
    }
    assert_eq!(
        catalog.fork_legacy_for_tests(fork).await.unwrap_err(),
        CatalogError::Corrupt
    );
    assert_eq!(
        catalog.read(id(2), None).await.unwrap_err(),
        CatalogError::NotFound
    );
}
