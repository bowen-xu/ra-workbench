//! Stopped hook context and supplier-owned state restored into actual HTTP input.
use super::*;
use research_agent_core::context::compaction_hooks::*;

const ENTRY: &str = "agent::model::responses_transport::tests::native_compaction::resume::hooks::remote_hook_resume_child";
const CRASH_ENTRY: &str = "agent::model::responses_transport::tests::native_compaction::resume::hooks::remote_hook_crash_child";
const CONTEXT: &str = "remote compact trusted hook context";
const REFRESHED_RULE: &str = "refreshed remote research authority";

struct Stop;
impl CompactionHooks for Stop {
    fn notify(&self, _: &CompactionHookRequest, _: CompactionHookNotification) {}
    fn run(&self, _: CompactionHookRequest) -> CompactionHookFuture<'_> {
        Box::pin(async { CompactionHookOutcome::Continue })
    }
    fn run_with_context(&self, request: CompactionHookRequest) -> CompactionHookResultFuture<'_> {
        Box::pin(async move {
            if request.stage == CompactionHookStage::SessionStartCompact {
                CompactionHookResult {
                    outcome: CompactionHookOutcome::Stop,
                    additional_contexts: vec![CONTEXT.into()],
                }
            } else {
                CompactionHookResult::without_context(CompactionHookOutcome::Continue)
            }
        })
    }
}
struct NoReplay;
impl CompactionHooks for NoReplay {
    fn notify(&self, _: &CompactionHookRequest, _: CompactionHookNotification) {}
    fn run(&self, _: CompactionHookRequest) -> CompactionHookFuture<'_> {
        panic!("restored remote hook context must not execute again")
    }
}

#[tokio::test]
#[ignore = "Internal fresh-process entry invoked by remote hook recovery tests"]
async fn remote_hook_resume_child() {
    let path = std::env::var("RA_REMOTE_HOOK_DB").unwrap();
    let id = Uuid::parse_str(&std::env::var("RA_REMOTE_HOOK_ID").unwrap()).unwrap();
    let url = std::env::var("RA_REMOTE_HOOK_URL").unwrap();
    let legacy = std::env::var("RA_REMOTE_HOOK_LEGACY").unwrap() == "yes";
    let window = Uuid::parse_str(&std::env::var("RA_REMOTE_HOOK_WINDOW").unwrap()).unwrap();
    let fixture = fixture_at(Path::new(&path), id);
    for (text, refresh) in [
        ("remote hook fresh input", false),
        ("remote hook second input", true),
    ] {
        let record = fixture.record(text).await;
        let mut execution = execution(&fixture, &record, &url, legacy);
        if refresh {
            execution.system_instructions = REFRESHED_RULE.into();
        }
        execution.compaction_hooks = Some(Arc::new(NoReplay));
        assert_eq!(
            fixture
                .start(&record, execution, false)
                .await
                .task
                .wait()
                .await,
            TaskCompletion::Completed
        );
        assert_eq!(
            fixture
                .service
                .active_context_checkpoint(id)
                .await
                .unwrap()
                .unwrap()
                .window_id,
            window
        );
        let db = fixture.app.shared_agent_connection();
        let guard = db.lock().unwrap();
        let legacy_rows: u64 = guard
            .query_row(
                "select count(*) from agent_responses_checkpoints where turn_id=?1",
                [record.turn_id.to_string()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            legacy_rows, 0,
            "new responses must not save cumulative prefixes"
        );
        let canonical:u64=guard.query_row("select count(*) from agent_items where conversation_id=?1 and json_extract(payload_json,'$.payload.turnId')=?2 and json_extract(payload_json,'$.payload.type')='itemAccepted' and json_extract(json_extract(payload_json,'$.payload.source.providerDataJson'),'$.schema_version')=1",rusqlite::params![id.to_string(),record.turn_id.to_string()],|row|row.get(0)).unwrap();
        assert_eq!(
            canonical, 1,
            "completed native output lives in the same accepted history fact"
        );
    }
    let db = fixture.app.shared_agent_connection();
    let audit: u64 = db.lock().unwrap().query_row(
        "select count(*) from agent_compaction_hook_audit where conversation_id=?1 and stage='session_start_compact'",
        [id.to_string()], |row| row.get(0),
    ).unwrap();
    assert_eq!(audit, 2, "recovery cannot add a second hook execution");
}

#[tokio::test]
async fn native_v2_restores_stopped_hook_context_in_fresh_process_http_input() {
    verify(false, false, false).await;
}

#[tokio::test]
async fn native_full_window_restores_stopped_hook_context_in_fresh_process_http_input() {
    verify(true, false, false).await;
}

#[tokio::test]
async fn native_empty_v2_restores_stopped_hook_context_in_fresh_process_http_input() {
    verify(false, true, false).await;
}

#[tokio::test]
async fn native_v2_stopped_hook_survives_terminal_failure_and_abrupt_exit() {
    verify(false, false, true).await;
}

#[tokio::test]
async fn native_full_window_stopped_hook_survives_terminal_failure_and_abrupt_exit() {
    verify(true, false, true).await;
}

#[tokio::test]
async fn native_empty_v2_stopped_hook_survives_terminal_failure_and_abrupt_exit() {
    verify(false, true, true).await;
}

#[tokio::test]
#[ignore = "Internal abrupt-exit producer invoked by remote hook recovery tests"]
async fn remote_hook_crash_child() {
    let path = std::env::var("RA_REMOTE_HOOK_DB").unwrap();
    let id = Uuid::parse_str(&std::env::var("RA_REMOTE_HOOK_ID").unwrap()).unwrap();
    let url = std::env::var("RA_REMOTE_HOOK_URL").unwrap();
    let legacy = std::env::var("RA_REMOTE_HOOK_LEGACY").unwrap() == "yes";
    let empty = std::env::var("RA_REMOTE_HOOK_EMPTY").unwrap() == "yes";
    let window = prepare_stopped(Path::new(&path), id, &url, legacy, empty, true).await;
    std::fs::write(
        Path::new(&path).with_extension("window"),
        window.to_string(),
    )
    .unwrap();
    // Exit without unwinding any runtime or database owner.
    std::process::exit(73);
}

async fn prepare_stopped(
    path: &Path,
    id: Uuid,
    url: &str,
    legacy: bool,
    empty: bool,
    crash: bool,
) -> Uuid {
    let fixture = fixture_at(path, id);
    for (text, compact, stopped) in [
        ("original remote hook question", false, false),
        ("/compact", true, false),
        ("stop before remote sampling", false, true),
    ] {
        if empty && text == "original remote hook question" {
            continue;
        }
        let record = fixture.record(text).await;
        if crash && stopped {
            fixture
                .app
                .shared_agent_connection()
                .lock()
                .unwrap()
                .execute_batch(
                    "create trigger fail_hook_terminal before insert on agent_journal_records
                when json_extract(new.payload_json,'$.payload.type')='turnFinished'
                begin select raise(abort,'synthetic hook terminal write failure'); end;",
                )
                .unwrap();
            std::fs::write(
                path.with_extension("stopped-turn"),
                record.turn_id.to_string(),
            )
            .unwrap();
        }
        let mut execution = execution(&fixture, &record, url, legacy);
        execution.compaction_hooks = Some(Arc::new(Stop));
        assert_eq!(
            fixture
                .start(&record, execution, compact)
                .await
                .task
                .wait()
                .await,
            if crash && stopped {
                TaskCompletion::RecoveryRequired
            } else if stopped {
                TaskCompletion::Canceled
            } else {
                TaskCompletion::Completed
            }
        );
    }
    let window = fixture
        .service
        .active_context_checkpoint(id)
        .await
        .unwrap()
        .unwrap()
        .window_id;
    if crash {
        // Keep all owners alive until process::exit; graceful close must not help recovery.
        std::mem::forget(fixture);
    } else {
        let connection = Arc::downgrade(&fixture.app.shared_agent_connection());
        drop(fixture);
        assert!(connection.upgrade().is_none());
    }
    window
}

async fn verify(legacy: bool, empty: bool, crash: bool) {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("remote-hook.sqlite");
    let id = Uuid::new_v4();
    let fixture = fixture_at(&path, id);
    fixture
        .service
        .create(ConversationId::from_uuid(id))
        .await
        .unwrap();
    let state = if legacy {
        legacy::full_window("hook window")
    } else {
        vec![opaque()]
    };
    let compaction_response = if legacy {
        legacy::json_response(&state)
    } else {
        ok(&events(&state, true))
    };
    let mut replies = vec![];
    if !empty {
        replies.push(ok(&body()));
    }
    replies.extend([compaction_response, ok(&body()), ok(&body())]);
    let (url, peer) = sequential_server(replies).await;
    let connection = Arc::downgrade(&fixture.app.shared_agent_connection());
    drop(fixture);
    assert!(connection.upgrade().is_none());
    let window = if crash {
        let mut producer = tokio::process::Command::new(std::env::current_exe().unwrap());
        producer
            .args(["--exact", CRASH_ENTRY, "--ignored", "--nocapture"])
            .env_clear()
            .env("RA_REMOTE_HOOK_DB", &path)
            .env("RA_REMOTE_HOOK_ID", id.to_string())
            .env("RA_REMOTE_HOOK_URL", &url)
            .env("RA_REMOTE_HOOK_LEGACY", if legacy { "yes" } else { "no" })
            .env("RA_REMOTE_HOOK_EMPTY", if empty { "yes" } else { "no" })
            .kill_on_drop(true);
        let output = timeout(Duration::from_secs(30), producer.output())
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
        let fixture = fixture_at(&path, id);
        fixture
            .app
            .shared_agent_connection()
            .lock()
            .unwrap()
            .execute_batch("drop trigger fail_hook_terminal;")
            .unwrap();
        let projection = fixture
            .service
            .open(ConversationId::from_uuid(id))
            .await
            .unwrap();
        assert!(projection.active_turn().is_none());
        let stopped_turn =
            Uuid::parse_str(&std::fs::read_to_string(path.with_extension("stopped-turn")).unwrap())
                .unwrap();
        assert_eq!(
            projection.turn(stopped_turn).unwrap().outcome,
            Some(research_agent_core::protocol::TurnOutcome::Interrupted)
        );
        let window =
            Uuid::parse_str(&std::fs::read_to_string(path.with_extension("window")).unwrap())
                .unwrap();
        let connection = Arc::downgrade(&fixture.app.shared_agent_connection());
        drop(fixture);
        assert!(connection.upgrade().is_none());
        window
    } else {
        prepare_stopped(&path, id, &url, legacy, empty, false).await
    };
    let mut child = tokio::process::Command::new(std::env::current_exe().unwrap());
    child
        .args(["--exact", ENTRY, "--ignored", "--nocapture"])
        .env_clear()
        .env("RA_REMOTE_HOOK_DB", &path)
        .env("RA_REMOTE_HOOK_ID", id.to_string())
        .env("RA_REMOTE_HOOK_URL", &url)
        .env("RA_REMOTE_HOOK_LEGACY", if legacy { "yes" } else { "no" })
        .env("RA_REMOTE_HOOK_WINDOW", window.to_string())
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
    let requests = peer.await.unwrap();
    assert_eq!(
        requests.len(),
        if empty { 3 } else { 4 },
        "stopping the hook must prevent sampling"
    );
    let latest = requests.last().unwrap()["input"].as_array().unwrap();
    assert_eq!(latest[0]["content"], REFRESHED_RULE);
    assert!(
        !latest
            .iter()
            .any(|item| item["content"] == "current research authority")
    );
    assert_eq!(
        latest
            .iter()
            .filter(|item| item["role"] == "user"
                && item["content"][0]["text"] == "remote hook second input")
            .count(),
        1
    );
    for request in requests.iter().rev().take(2) {
        let input = request["input"].as_array().unwrap();
        for item in &state {
            assert_eq!(input.iter().filter(|value| *value == item).count(), 1);
        }
        let hook: Vec<_> = input
            .iter()
            .enumerate()
            .filter_map(|(index, item)| {
                (item["role"] == "developer" && item["content"] == CONTEXT).then_some(index)
            })
            .collect();
        assert_eq!(hook.len(), 1);
        let fresh = input
            .iter()
            .position(|item| {
                item["role"] == "user" && item["content"][0]["text"] == "remote hook fresh input"
            })
            .unwrap();
        let opaque = input
            .iter()
            .position(|item| item["type"] == "compaction")
            .unwrap();
        assert!(opaque < hook[0] && hook[0] < fresh);
        assert_eq!(
            input
                .iter()
                .filter(|item| item["role"] == "user"
                    && item["content"][0]["text"] == "stop before remote sampling")
                .count(),
            1,
            "accepted input remains durable even when the hook prevents sampling"
        );
        assert!(
            !input
                .iter()
                .any(|item| item["role"] == "user" && item.to_string().contains(CONTEXT))
        );
    }
}
