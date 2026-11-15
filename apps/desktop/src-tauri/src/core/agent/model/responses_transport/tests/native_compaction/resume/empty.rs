//! Empty legacy replacement crosses the same SQLite/process boundary as a
//! nonempty window. Only provider replies and account identities are synthetic.
use super::*;

#[tokio::test]
async fn native_empty_full_window_replaces_old_receipts_and_resumes_in_a_fresh_process() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("empty-window.sqlite3");
    let id = Uuid::new_v4();
    let fixture = fixture_at(&path, id);
    fixture
        .service
        .create(ConversationId::from_uuid(id))
        .await
        .unwrap();
    let old = legacy::full_window("superseded encrypted window");
    let (url, peer) = sequential_server(vec![
        ok(&body()),
        legacy::json_response(&old),
        ok(&body()),
        legacy::json_response(&[]),
        ok(&body()),
        ok(&body()),
        legacy::json_response(&[]),
        ok(&body()),
    ])
    .await;
    let mut previous = None;
    let mut empty = None;
    for (text, compact) in [
        ("original history", false),
        ("/compact", true),
        ("superseded tail", false),
        ("/compact", true),
        ("durable empty tail", false),
    ] {
        let record = fixture.record(text).await;
        assert_eq!(
            fixture
                .start(&record, execution(&fixture, &record, &url, true), compact)
                .await
                .task
                .wait()
                .await,
            TaskCompletion::Completed
        );
        if compact {
            let checkpoint = fixture
                .service
                .active_context_checkpoint(id)
                .await
                .unwrap()
                .unwrap();
            if let Some(previous) = previous {
                assert_eq!(checkpoint.previous_window_id, Some(previous));
                assert!(checkpoint.output == CompactionOutput::RemoteWindow(vec![]));
                assert!(checkpoint.retained.is_empty());
                empty = Some(checkpoint.window_id);
            }
            previous = Some(checkpoint.window_id);
        }
    }
    let empty = empty.unwrap();
    let connection = Arc::downgrade(&fixture.app.shared_agent_connection());
    drop(fixture);
    assert!(connection.upgrade().is_none());
    let mut child = tokio::process::Command::new(std::env::current_exe().unwrap());
    child
        .args(["--exact", CHILD, "--ignored", "--nocapture"])
        .env_clear()
        .env("RA_COMPACTION_TEST_DB", &path)
        .env("RA_COMPACTION_TEST_ID", id.to_string())
        .env("RA_COMPACTION_TEST_URL", &url)
        .env("RA_COMPACTION_TEST_LEGACY", "1")
        .env("RA_COMPACTION_TEST_WINDOW", empty.to_string())
        .kill_on_drop(true);
    let output = timeout(Duration::from_secs(30), child.output())
        .await
        .unwrap()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
    let sent = peer.await.unwrap();
    assert_eq!(sent.len(), 8);
    assert!(sent[3].to_string().contains("superseded encrypted window"));
    for request in &sent[4..] {
        let text = request.to_string();
        for removed in [
            "original history",
            "superseded encrypted window",
            "superseded tail",
            "/compact",
        ] {
            assert!(
                !text.contains(removed),
                "replayed replaced context: {removed}"
            );
        }
        assert!(!text.contains("\"encrypted_content\""));
    }
    for request in &sent[5..7] {
        let text = request.to_string();
        assert!(text.contains("durable empty tail"));
        assert!(text.contains("resumed current research authority"));
    }
    assert!(!sent[7].to_string().contains("durable empty tail"));
    assert!(sent[7].to_string().contains("after resumed compact"));
    let reopened = fixture_at(&path, id);
    let installed = reopened
        .service
        .active_context_checkpoint(id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(installed.previous_window_id, Some(empty));
    assert!(installed.output == CompactionOutput::RemoteWindow(vec![]));
}
