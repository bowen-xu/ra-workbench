//! Pinned compact_resume_fork resume contract across an actual process boundary.
//! Reuse the native task owner and SQLite; only HTTP/provider data are synthetic.
use super::*;
use std::path::Path;
#[path = "resume/crash.rs"]
mod crash;
#[path = "resume/empty.rs"]
mod empty;
#[path = "resume/hooks.rs"]
mod hooks;

const CHILD: &str =
    "agent::model::responses_transport::tests::native_compaction::resume::resume_child";

pub(super) fn fixture_at(path: &Path, id: Uuid) -> Fixture {
    let mut db = rusqlite::Connection::open(path).unwrap();
    crate::storage::migrate(&mut db).unwrap();
    let app = crate::storage::AppStore::for_test(db, path.parent().unwrap().join("objects"));
    Fixture {
        _root: tempfile::tempdir().unwrap(),
        service: Arc::new(AgentSessionService::from_app_store(&app).unwrap()),
        store: Arc::new(SqliteSubmissionStore::new(app.shared_agent_connection())),
        app,
        id,
    }
}

fn execution(
    fixture: &Fixture,
    record: &SubmissionRecord,
    url: &str,
    legacy: bool,
) -> AgentExecution {
    if legacy {
        legacy::legacy_execution(fixture, record, url)
    } else {
        fixture.execution(record, fixture.transport(record, url))
    }
}

#[tokio::test]
#[ignore = "Internal subprocess entry; invoked with synthetic fixture paths by the two resume tests"]
async fn resume_child() {
    let path = std::env::var("RA_COMPACTION_TEST_DB").unwrap();
    let id = Uuid::parse_str(&std::env::var("RA_COMPACTION_TEST_ID").unwrap()).unwrap();
    let url = std::env::var("RA_COMPACTION_TEST_URL").unwrap();
    let legacy = std::env::var("RA_COMPACTION_TEST_LEGACY").unwrap() == "1";
    let original = Uuid::parse_str(&std::env::var("RA_COMPACTION_TEST_WINDOW").unwrap()).unwrap();
    let fixture = fixture_at(Path::new(&path), id);
    assert_eq!(
        fixture
            .service
            .active_context_checkpoint(id)
            .await
            .unwrap()
            .unwrap()
            .window_id,
        original
    );
    for (text, compact) in [
        ("resumed fresh", false),
        ("/compact", true),
        ("after resumed compact", false),
    ] {
        let record = fixture.record(text).await;
        let mut execution = execution(&fixture, &record, &url, legacy);
        execution.system_instructions = "resumed current research authority".into();
        assert_eq!(
            fixture
                .start(&record, execution, compact)
                .await
                .task
                .wait()
                .await,
            TaskCompletion::Completed
        );
    }
    let installed = fixture
        .service
        .active_context_checkpoint(id)
        .await
        .unwrap()
        .unwrap();
    assert_ne!(installed.window_id, original);
    assert_eq!(installed.previous_window_id, Some(original));
}

#[tokio::test]
async fn native_v2_resumes_and_recompacts_in_a_fresh_process() {
    resume(false).await;
}

#[tokio::test]
async fn native_full_window_resumes_and_recompacts_in_a_fresh_process() {
    resume(true).await;
}

async fn resume(legacy: bool) {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("resume.sqlite3");
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
    let replacement = if legacy {
        legacy::full_window("new window")
    } else {
        vec![json!({"type":"compaction","encrypted_content":"replacement opaque","extension":2})]
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
        ok(&body()),
        compact_response(&replacement),
        ok(&body()),
    ])
    .await;
    for (text, compact) in [
        ("original question", false),
        ("/compact", true),
        ("durable tail", false),
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
    let connection = Arc::downgrade(&fixture.app.shared_agent_connection());
    drop(fixture);
    assert!(
        connection.upgrade().is_none(),
        "parent connection must close before resume"
    );
    run_child(&path, id, &url, original, legacy).await;
    let sent = peer.await.unwrap();
    assert_eq!(sent.len(), 6);
    for index in [3, 4] {
        let input = sent[index]["input"].as_array().unwrap();
        for item in &old {
            assert_eq!(input.iter().filter(|value| *value == item).count(), 1);
        }
        assert!(sent[index].to_string().contains("durable tail"));
        assert!(
            sent[index]
                .to_string()
                .contains("resumed current research authority")
        );
        if legacy {
            assert_eq!(&input[1..1 + old.len()], old.as_slice());
        }
    }
    let input = sent[5]["input"].as_array().unwrap();
    for item in &replacement {
        assert_eq!(input.iter().filter(|value| *value == item).count(), 1);
    }
    for item in &old {
        assert!(!input.contains(item));
    }
    if legacy {
        assert_eq!(&input[1..1 + replacement.len()], replacement.as_slice());
    }
    assert!(
        sent.iter()
            .all(|request| !request.to_string().contains("/compact"))
    );
    let reopened = fixture_at(&path, id);
    let installed = reopened
        .service
        .active_context_checkpoint(id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(installed.previous_window_id, Some(original));
}

pub(super) async fn run_child(path: &Path, id: Uuid, url: &str, original: Uuid, legacy: bool) {
    let mut child = tokio::process::Command::new(std::env::current_exe().unwrap());
    child
        .args(["--exact", CHILD, "--ignored", "--nocapture"])
        .env_clear()
        .env("RA_COMPACTION_TEST_DB", path)
        .env("RA_COMPACTION_TEST_ID", id.to_string())
        .env("RA_COMPACTION_TEST_URL", url)
        .env("RA_COMPACTION_TEST_LEGACY", if legacy { "1" } else { "0" })
        .env("RA_COMPACTION_TEST_WINDOW", original.to_string())
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
}
