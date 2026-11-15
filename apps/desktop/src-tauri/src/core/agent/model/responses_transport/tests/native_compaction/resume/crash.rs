//! Durable installation fault boundaries; abrupt child exit bypasses Drop cleanup.
use super::*;
use research_agent_core::protocol::{TaskKind, TurnOutcome};

const CRASH_CHILD: &str =
    "agent::model::responses_transport::tests::native_compaction::resume::crash::crash_child";

#[tokio::test]
#[ignore = "Internal fault-injection subprocess, invoked by the crash-boundary tests"]
async fn crash_child() {
    let path = std::env::var("RA_COMPACTION_TEST_DB").unwrap();
    let id = Uuid::parse_str(&std::env::var("RA_COMPACTION_TEST_ID").unwrap()).unwrap();
    let url = std::env::var("RA_COMPACTION_TEST_URL").unwrap();
    let legacy = std::env::var("RA_COMPACTION_TEST_LEGACY").unwrap() == "1";
    let before = std::env::var("RA_COMPACTION_TEST_BEFORE").unwrap() == "1";
    let fixture = fixture_at(Path::new(&path), id);
    let record = fixture.record("/compact").await;
    {
        let db = fixture.app.shared_agent_connection();
        let db = db.lock().unwrap();
        db.execute_batch(
            "create trigger fail_compact_terminal before insert on agent_journal_records
            when json_extract(new.payload_json,'$.payload.type')='turnFinished'
            begin select raise(abort,'synthetic terminal write failure'); end;",
        )
        .unwrap();
        if before {
            db.execute_batch(
                "create trigger fail_compact_pointer before update on agent_active_context_windows
                begin select raise(abort,'synthetic window publication failure'); end;",
            )
            .unwrap();
        }
    }
    assert_eq!(
        fixture
            .start(&record, execution(&fixture, &record, &url, legacy), true)
            .await
            .task
            .wait()
            .await,
        TaskCompletion::RecoveryRequired
    );
    std::fs::write(
        Path::new(&path).with_extension("crash-turn"),
        record.turn_id.to_string(),
    )
    .unwrap();
    // Do not drop the service, registry, connection or runtime gracefully.
    std::process::exit(73);
}

#[tokio::test]
async fn v2_window_publication_and_terminal_failure_survive_abrupt_process_exit() {
    crash(false, true).await;
    crash(false, false).await;
}

#[tokio::test]
async fn full_window_publication_and_terminal_failure_survive_abrupt_process_exit() {
    crash(true, true).await;
    crash(true, false).await;
}

async fn crash(legacy: bool, before: bool) {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("crash.sqlite3");
    let id = Uuid::new_v4();
    let fixture = fixture_at(&path, id);
    fixture
        .service
        .create(ConversationId::from_uuid(id))
        .await
        .unwrap();
    let old = if legacy {
        legacy::full_window("old window")
    } else {
        vec![opaque()]
    };
    let new = if legacy {
        legacy::full_window("committed window")
    } else {
        vec![json!({"type":"compaction","encrypted_content":"committed opaque"})]
    };
    let compact_response = |items: &[Value]| {
        if legacy {
            legacy::json_response(items)
        } else {
            ok(&events(items, true))
        }
    };
    let (url, peer) = sequential_server(vec![
        ok(&body()),
        compact_response(&old),
        ok(&body()),
        compact_response(&new),
        ok(&body()),
    ])
    .await;
    for (text, compact) in [
        ("initial history", false),
        ("/compact", true),
        ("durable evidence tail", false),
    ] {
        let record = fixture.record(text).await;
        assert_eq!(
            fixture
                .start(&record, execution(&fixture, &record, &url, legacy), compact)
                .await
                .task
                .wait()
                .await,
            TaskCompletion::Completed
        );
    }
    let original = fixture
        .service
        .active_context_checkpoint(id)
        .await
        .unwrap()
        .unwrap()
        .window_id;
    let weak = Arc::downgrade(&fixture.app.shared_agent_connection());
    drop(fixture);
    assert!(weak.upgrade().is_none());
    let mut command = tokio::process::Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", CRASH_CHILD, "--ignored", "--nocapture"])
        .env_clear()
        .env("RA_COMPACTION_TEST_DB", &path)
        .env("RA_COMPACTION_TEST_ID", id.to_string())
        .env("RA_COMPACTION_TEST_URL", &url)
        .env("RA_COMPACTION_TEST_LEGACY", if legacy { "1" } else { "0" })
        .env("RA_COMPACTION_TEST_BEFORE", if before { "1" } else { "0" })
        .kill_on_drop(true);
    let output = timeout(Duration::from_secs(30), command.output())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(73),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let turn =
        Uuid::parse_str(&std::fs::read_to_string(path.with_extension("crash-turn")).unwrap())
            .unwrap();
    let fixture = fixture_at(&path, id);
    let checkpoint = fixture
        .service
        .active_context_checkpoint(id)
        .await
        .unwrap()
        .unwrap();
    let connection = fixture.app.shared_agent_connection();
    {
        let db = connection.lock().unwrap();
        db.execute_batch(
            "drop trigger fail_compact_terminal; drop trigger if exists fail_compact_pointer;",
        )
        .unwrap();
        assert_eq!(
            db.query_row(
                "select count(*) from agent_context_checkpoints where conversation_id=?1",
                [id.to_string()],
                |row| row.get::<_, usize>(0)
            )
            .unwrap(),
            if before { 1 } else { 2 }
        );
    }
    if before {
        assert_eq!(checkpoint.window_id, original);
    } else {
        assert_ne!(checkpoint.window_id, original);
        assert_eq!(checkpoint.previous_window_id, Some(original));
    }
    let projection = fixture
        .service
        .open(ConversationId::from_uuid(id))
        .await
        .unwrap();
    let interrupted = projection.turn(turn).unwrap();
    assert_eq!(interrupted.task_kind, TaskKind::Compact);
    assert_eq!(interrupted.outcome, Some(TurnOutcome::Interrupted));
    assert_eq!(interrupted.items.len(), 1);
    let item = interrupted.items.values().next().unwrap();
    assert_eq!(
        item.kind,
        research_agent_core::protocol::ItemKind::ContextCompaction
    );
    assert_eq!(item.completed, !before);
    assert_eq!(item.interrupted, before);
    assert!(projection.active_turn().is_none());
    let repeated = fixture
        .service
        .open(ConversationId::from_uuid(id))
        .await
        .unwrap();
    assert_eq!(repeated.seq(), projection.seq());
    let record = fixture.record("continue after crash").await;
    assert_eq!(
        fixture
            .start(&record, execution(&fixture, &record, &url, legacy), false)
            .await
            .task
            .wait()
            .await,
        TaskCompletion::Completed
    );
    let sent = peer.await.unwrap();
    assert_eq!(sent.len(), 5);
    let selected = if before { &old } else { &new };
    let excluded = if before { &new } else { &old };
    let input = sent[4]["input"].as_array().unwrap();
    for item in selected {
        assert_eq!(input.iter().filter(|value| *value == item).count(), 1);
    }
    for item in excluded {
        assert!(!input.contains(item));
    }
    if legacy {
        assert_eq!(&input[1..1 + selected.len()], selected.as_slice());
    }
    assert!(sent[4].to_string().contains("continue after crash"));
    assert!(
        sent.iter()
            .all(|request| !request.to_string().contains("/compact"))
    );
    assert_eq!(
        fixture
            .service
            .active_context_checkpoint(id)
            .await
            .unwrap()
            .unwrap()
            .window_id,
        checkpoint.window_id
    );
}
