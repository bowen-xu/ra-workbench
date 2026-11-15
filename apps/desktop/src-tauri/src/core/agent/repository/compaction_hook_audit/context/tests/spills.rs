use super::*;
use crate::agent::repository::hook_spills as spills;
use research_agent_core::workspace::FileVersion;

fn receipt(db: &Connection, request: &CompactionHookRequest) -> (Uuid, String, FileVersion) {
    let id = Uuid::new_v4();
    let version = FileVersion {
        sha256: "a".repeat(64),
        byte_size: 4096,
        executable: false,
    };
    db.execute("insert into content_objects(sha256,relative_path,mime_type,byte_size,created_at) values(?1,?2,'text/plain',4096,'now') on conflict(sha256) do nothing",params![version.sha256,format!("aa/aa/{}",version.sha256)]).unwrap();
    spills::save_receipt(db, request, id, &version).unwrap();
    let preview = format!(
        "head … tail\n\nFull hook output saved to: {}",
        spills::locator(id)
    );
    spills::bind_preview(db, request, id, &preview).unwrap();
    (id, preview, version)
}
fn completed(preview: String) -> CompactionHookResult {
    CompactionHookResult {
        outcome: CompactionHookOutcome::Continue,
        additional_contexts: vec![preview],
    }
}
fn granted(db: &Connection, request: &CompactionHookRequest, id: Uuid, through: u64) -> bool {
    spills::resolve(db, request.conversation_id, id, &request.model, through).is_ok()
}

#[test]
fn ancestor_hook_spill_retains_producer_and_grants_active_window() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("ancestor-spill.sqlite");
    let (mut db, request) = setup(&path);
    let mut next = context_windows::read_active(&db, request.conversation_id)
        .unwrap()
        .unwrap();
    next.previous_window_id = Some(next.window_id);
    next.window_id = Uuid::new_v4();
    context_windows::install(&mut db, &next).unwrap();
    let (id, preview, version) = receipt(&db, &request);
    record_result(&mut db, &request, &completed(preview), 2).unwrap();
    let (source, owner): (String, String) = db.query_row(
        "select a.source_window_id,g.window_id from agent_hook_spill_artifacts a join agent_hook_spill_grants g on g.artifact_id=a.id where a.id=?1",
        [id.to_string()], |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap();
    assert_eq!(source, request.operation_id.to_string());
    assert_eq!(owner, next.window_id.to_string());
    drop(db);
    let db = Connection::open(&path).unwrap();
    assert_eq!(
        spills::resolve(&db, request.conversation_id, id, &request.model, 1)
            .unwrap()
            .0,
        version
    );
    assert_eq!(
        read(&db, request.conversation_id, next.window_id, 0, 1)
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn spill_receipt_requires_exact_completed_preview_and_survives_reopen() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("spill.sqlite");
    let (mut db, request) = setup(&path);
    let (id, preview, version) = receipt(&db, &request);
    assert!(!granted(&db, &request, id, 1));
    let mut output = completed(preview.clone());
    output.additional_contexts.push(preview);
    record_result(&mut db, &request, &output, 2).unwrap();
    assert!(!granted(&db, &request, id, 0));
    assert!(granted(&db, &request, id, 1));
    assert!(!granted(&db, &request, id, 2));
    let mut model = request.model.clone();
    model.revision = Some("other".into());
    assert!(spills::resolve(&db, request.conversation_id, id, &model, 1).is_err());
    assert!(spills::resolve(&db, Uuid::new_v4(), id, &request.model, 1).is_err());
    drop(db);
    let mut db = Connection::open(path).unwrap();
    assert_eq!(
        spills::resolve(&db, request.conversation_id, id, &request.model, 1)
            .unwrap()
            .0,
        version
    );
    let mut next = context_windows::read_active(&db, request.conversation_id)
        .unwrap()
        .unwrap();
    next.previous_window_id = Some(next.window_id);
    next.window_id = Uuid::new_v4();
    context_windows::install(&mut db, &next).unwrap();
    assert!(
        read_current(&db, request.conversation_id)
            .unwrap()
            .is_empty()
    );
    assert!(
        granted(&db, &request, id, 1),
        "window rollover keeps surviving evidence readable"
    );
    db.execute(
        "update agent_conversations set archived=1 where id=?1",
        [request.conversation_id.to_string()],
    )
    .unwrap();
    assert!(!granted(&db, &request, id, 1));
}

#[test]
fn changed_preview_and_forged_locators_do_not_activate_native_receipts() {
    let root = tempfile::tempdir().unwrap();
    let (mut db, request) = setup(&root.path().join("spill.sqlite"));
    let (id, preview, _) = receipt(&db, &request);
    assert!(spills::bind_preview(&db, &request, id, &format!("changed {preview}")).is_err());
    let mut other = request.clone();
    other.model.model_id = "other".into();
    assert!(spills::bind_preview(&db, &other, id, &preview).is_err());
    assert!(spills::bind_preview(&db, &request, Uuid::new_v4(), &preview).is_err());
    spills::bind_preview(&db, &request, id, &preview).unwrap();
    record_result(
        &mut db,
        &request,
        &completed(format!("forged {preview}")),
        2,
    )
    .unwrap();
    assert!(!granted(&db, &request, id, 1));
    assert!(
        db.execute(
            "update agent_hook_spill_previews set preview_sha256=?1",
            ["b".repeat(64)]
        )
        .is_err()
    );
    assert!(
        db.execute("update agent_hook_spill_artifacts set phase='mid_turn'", [])
            .is_err()
    );
}

#[test]
fn failed_grant_publication_rolls_back_hook_context_and_completion() {
    let root = tempfile::tempdir().unwrap();
    let (mut db, request) = setup(&root.path().join("spill.sqlite"));
    let (id, preview, _) = receipt(&db, &request);
    db.execute_batch("create trigger reject_grant before insert on agent_hook_spill_grants begin select raise(abort,'fault'); end;").unwrap();
    assert!(record_result(&mut db, &request, &completed(preview.clone()), 2).is_err());
    assert!(!granted(&db, &request, id, 1));
    assert!(
        read_current(&db, request.conversation_id)
            .unwrap()
            .is_empty()
    );
    let count: i64 = db
        .query_row(
            "select count(*) from agent_compaction_hook_audit where notification='completed'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
    db.execute_batch("drop trigger reject_grant").unwrap();
    record_result(&mut db, &request, &completed(preview), 3).unwrap();
    assert!(granted(&db, &request, id, 1));
}

#[test]
fn rollback_denies_spill_reads_but_retains_full_object_reference() {
    use research_agent_core::protocol::{ItemKind, TurnOutcome};
    let root = tempfile::tempdir().unwrap();
    let (mut db, request) = setup(&root.path().join("spill.sqlite"));
    let (id, preview, version) = receipt(&db, &request);
    record_result(&mut db, &request, &completed(preview), 2).unwrap();
    let item = Uuid::new_v4();
    for (i, payload) in [
        EventPayload::ItemStarted {
            turn_id: request.turn_id,
            item_id: item,
            kind: ItemKind::UserMessage,
        },
        EventPayload::ItemCompleted {
            turn_id: request.turn_id,
            item_id: item,
        },
        EventPayload::TurnFinished {
            turn_id: request.turn_id,
            outcome: TurnOutcome::Canceled,
        },
        EventPayload::ConversationRolledBack {
            turn_id: request.turn_id,
            user_turns: 1,
        },
    ]
    .into_iter()
    .enumerate()
    {
        append(&mut db, request.conversation_id, i as u64 + 1, payload);
    }
    assert!(!granted(&db, &request, id, 5));
    let references:i64=db.query_row("select count(*) from content_object_references where owner_kind='agent_hook_spill_artifact' and content_sha256=?1",[version.sha256],|r|r.get(0)).unwrap();
    assert_eq!(references, 1);
    assert_eq!(
        db.query_row("select count(*) from agent_hook_spill_grants", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn pending_artifact_retention_cancels_gc_and_removal_requeues_it() {
    let root = tempfile::tempdir().unwrap();
    let (db, request) = setup(&root.path().join("spill.sqlite"));
    let (id, _, version) = receipt(&db, &request);
    assert!(
        db.execute(
            "delete from content_objects where sha256=?1",
            [&version.sha256]
        )
        .is_err()
    );
    db.execute(
        "delete from agent_hook_spill_artifacts where id=?1",
        [id.to_string()],
    )
    .unwrap();
    assert_eq!(
        db.query_row(
            "select count(*) from content_object_gc_candidates where content_sha256=?1",
            [version.sha256],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
}

#[tokio::test]
async fn nested_fork_spill_grants_are_cutoff_bound_and_independent_of_parent() {
    use crate::agent::repository::SqliteConversationCatalog;
    use research_agent_core::conversation::{
        BindingKind, ConversationBinding, ConversationDeletionRequest, ConversationForkRequest,
    };
    use std::sync::{Arc, Mutex};
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("spill.sqlite");
    let (mut db, request) = setup(&path);
    let (id, preview, _) = receipt(&db, &request);
    // A user message makes this a real forkable journal turn.
    let mut output = result();
    output.additional_contexts = vec![preview];
    let user = Uuid::new_v4();
    append(
        &mut db,
        request.conversation_id,
        1,
        EventPayload::ItemStarted {
            turn_id: request.turn_id,
            item_id: user,
            kind: research_agent_core::protocol::ItemKind::UserMessage,
        },
    );
    append(
        &mut db,
        request.conversation_id,
        2,
        EventPayload::ItemCompleted {
            turn_id: request.turn_id,
            item_id: user,
        },
    );
    record_result(&mut db, &request, &output, 2).unwrap();
    append(
        &mut db,
        request.conversation_id,
        3,
        EventPayload::TurnFinished {
            turn_id: request.turn_id,
            outcome: research_agent_core::protocol::TurnOutcome::Canceled,
        },
    );
    db.execute("insert into note_pages(id,title,created_at,updated_at) values('spill-note','Note','now','now')",[]).unwrap();
    db.execute("insert into agent_conversation_bindings(conversation_id,resource_kind,resource_id) values(?1,'note','spill-note')",[request.conversation_id.to_string()]).unwrap();
    let mut late = request.clone();
    late.turn_id = Uuid::new_v4();
    append(
        &mut db,
        request.conversation_id,
        4,
        EventPayload::TurnStarted {
            turn_id: late.turn_id,
            task_kind: TaskKind::Regular,
            context_snapshot_id: Uuid::new_v4(),
            model: request.model.clone(),
        },
    );
    crate::agent::repository::compaction_hook_audit::record(
        &db,
        &late,
        CompactionHookNotification::Started,
        3,
    )
    .unwrap();
    let (late_id, late_preview, _) = receipt(&db, &late);
    record_result(&mut db, &late, &completed(late_preview), 4).unwrap();
    append(
        &mut db,
        request.conversation_id,
        5,
        EventPayload::TurnFinished {
            turn_id: late.turn_id,
            outcome: research_agent_core::protocol::TurnOutcome::Canceled,
        },
    );
    let db = Arc::new(Mutex::new(db));
    let catalog = SqliteConversationCatalog::new(db.clone());
    let child = Uuid::new_v4();
    let grandchild = Uuid::new_v4();
    for (parent, child) in [(request.conversation_id, child), (child, grandchild)] {
        let fork = ConversationForkRequest {
            child_id: ConversationId::from_uuid(child),
            parent_id: ConversationId::from_uuid(parent),
            through_turn_id: request.turn_id,
            binding: ConversationBinding {
                kind: BindingKind::Note,
                resource_id: "spill-note".into(),
            },
        };
        db.lock().unwrap().execute_batch("create trigger reject_inherited_spill before insert on agent_hook_spill_grants begin select raise(abort,'fork fault'); end;").unwrap();
        assert!(catalog.fork_legacy_for_tests(fork.clone()).await.is_err());
        {
            let guard = db.lock().unwrap();
            let count: i64 = guard
                .query_row(
                    "select count(*) from agent_conversations where id=?1",
                    [child.to_string()],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(count, 0, "failed spill grant must roll back the whole fork");
            guard
                .execute_batch("drop trigger reject_inherited_spill")
                .unwrap();
        }
        catalog.fork_legacy_for_tests(fork.clone()).await.unwrap();
        catalog.fork_legacy_for_tests(fork).await.unwrap();
        let guard = db.lock().unwrap();
        assert!(spills::resolve(&guard, child, id, &request.model, 4).is_ok());
        assert!(spills::resolve(&guard, child, id, &request.model, 2).is_err());
        assert!(spills::resolve(&guard, child, late_id, &request.model, 4).is_err());
    }
    db.lock()
        .unwrap()
        .execute(
            "update agent_conversations set archived=1 where id=?1",
            [request.conversation_id.to_string()],
        )
        .unwrap();
    catalog
        .delete_legacy_for_tests(ConversationDeletionRequest {
            request_id: Uuid::new_v4(),
            conversation_id: ConversationId::from_uuid(request.conversation_id),
            binding: Some(ConversationBinding {
                kind: BindingKind::Note,
                resource_id: "spill-note".into(),
            }),
            expected_title: "hook context".into(),
            expected_durable_count: 6,
            force: true,
        })
        .await
        .unwrap();
    let reopened = Connection::open(&path).unwrap();
    assert!(spills::resolve(&reopened, grandchild, id, &request.model, 4).is_ok());
    drop(reopened);
    for removed in [child, grandchild] {
        let title = {
            let guard = db.lock().unwrap();
            guard
                .execute(
                    "update agent_conversations set archived=1 where id=?1",
                    [removed.to_string()],
                )
                .unwrap();
            guard
                .query_row(
                    "select title from agent_conversations where id=?1",
                    [removed.to_string()],
                    |r| r.get::<_, String>(0),
                )
                .unwrap()
        };
        catalog
            .delete_legacy_for_tests(ConversationDeletionRequest {
                request_id: Uuid::new_v4(),
                conversation_id: ConversationId::from_uuid(removed),
                binding: Some(ConversationBinding {
                    kind: BindingKind::Note,
                    resource_id: "spill-note".into(),
                }),
                expected_title: title,
                expected_durable_count: 4,
                force: true,
            })
            .await
            .unwrap();
        if removed == child {
            assert!(
                spills::resolve(&db.lock().unwrap(), grandchild, id, &request.model, 4).is_ok()
            );
        }
    }
    assert_eq!(
        db.lock()
            .unwrap()
            .query_row("select count(*) from agent_hook_spill_artifacts", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}
