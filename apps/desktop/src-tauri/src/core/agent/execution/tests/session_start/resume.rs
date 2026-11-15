//! Fresh-process recovery through the production SQLite owner and native task.
//! Model responses are injected; no network or external hooks run here.
use super::*;
use crate::agent::repository::SqliteConversationCatalog;
use research_agent_core::conversation::{
    BindingKind, ConversationBinding, ConversationForkRequest,
};
use std::path::Path;

const CHILD: &str =
    "agent::execution::tests::manual_compaction::session_start::resume::hook_resume_child";

struct NoReplay;
impl CompactionHooks for NoReplay {
    fn notify(&self, _: &CompactionHookRequest, _: CompactionHookNotification) {}
    fn run(&self, _: CompactionHookRequest) -> CompactionHookFuture<'_> {
        panic!("persisted or inherited SessionStart must not execute again")
    }
}

async fn fixture(
    path: &Path,
    id: Uuid,
    create: bool,
    reset: bool,
) -> (ExecutionFixture, crate::data_interface::SharedDatabase) {
    let (_, _, transport, tools, mut execution, _) = setup().await;
    if reset {
        execution.compaction_strategy = Some(Arc::new(
            research_agent_core::context::token_budget::reset::TokenBudgetResetStrategy::new(
                model(),
            )
            .unwrap(),
        ));
    }
    let mut db = Connection::open(path).unwrap();
    crate::storage::migrate(&mut db).unwrap();
    if create {
        db.execute("insert into note_pages(id,title,created_at,updated_at) values('hook-resume-note','Note','now','now')", []).unwrap();
    }
    let app = crate::storage::AppStore::for_test(db, path.parent().unwrap().join("objects"));
    let db = app.shared_agent_connection();
    let service = Arc::new(AgentSessionService::from_app_store(&app).unwrap());
    let store = Arc::new(SqliteSubmissionStore::new(db.clone()));
    if create {
        SqliteConversationCatalog::new(db.clone())
            .create_bound_legacy_for_tests(ConversationId::from_uuid(id), binding())
            .await
            .unwrap();
    } else {
        service.open(ConversationId::from_uuid(id)).await.unwrap();
    }
    ((service, store, transport, tools, execution, id), db)
}

fn binding() -> ConversationBinding {
    ConversationBinding {
        kind: BindingKind::Note,
        resource_id: "hook-resume-note".into(),
    }
}

#[tokio::test]
#[ignore = "Internal fresh-process entry invoked by hook context recovery test"]
async fn hook_resume_child() {
    let path = std::env::var("RA_HOOK_RESUME_DB").unwrap();
    let id = Uuid::parse_str(&std::env::var("RA_HOOK_RESUME_ID").unwrap()).unwrap();
    let parent = Uuid::parse_str(&std::env::var("RA_HOOK_RESUME_PARENT").unwrap()).unwrap();
    let window = Uuid::parse_str(&std::env::var("RA_HOOK_RESUME_WINDOW").unwrap()).unwrap();
    let expect_body = std::env::var("RA_HOOK_RESUME_BODY").unwrap() == "yes";
    let reset = std::env::var("RA_HOOK_RESUME_RESET").unwrap() == "yes";
    let ((service, store, transport, tools, mut execution, _), db) =
        fixture(Path::new(&path), id, false, reset).await;
    execution.compaction_hooks = Some(Arc::new(NoReplay));
    *transport.responses.lock().unwrap() = VecDeque::from(vec![response(false)]);
    let before = service
        .active_context_checkpoint(id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(before.window_id == window, id == parent);
    assert_eq!(
        matches!(
            before.output,
            research_agent_core::context::compaction_strategy::CompactionOutput::ResetWindow
        ),
        reset
    );
    let mut submission = input("fresh hook continuation");
    submission.text = "fresh-process hook input".into();
    let next = store.put(id, Uuid::new_v4(), submission).await.unwrap();
    assert_eq!(
        run(
            Arc::new(execution),
            service.clone(),
            store,
            id,
            next.request_id
        )
        .await,
        TaskCompletion::Completed
    );
    assert_eq!(
        service
            .active_context_checkpoint(id)
            .await
            .unwrap()
            .unwrap()
            .window_id,
        before.window_id
    );
    let requests = transport.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    let messages = &requests[0].messages;
    let bodies: Vec<_> = messages
        .iter()
        .enumerate()
        .filter_map(|(index, message)| {
            matches!(message, ModelMessage::Developer(body) if body == "persist before stopping")
                .then_some(index)
        })
        .collect();
    assert_eq!(bodies.len(), usize::from(expect_body));
    if expect_body {
        let input = messages.iter().position(|message| matches!(message,
            ModelMessage::User { content: parts, .. } if parts == &vec![ContentPart::Text("fresh-process hook input".into())])).unwrap();
        assert!(bodies[0] < input);
    }
    assert!(!messages.iter().any(|message| matches!(message,
        ModelMessage::User { content: parts, .. } if parts.iter().any(|part| matches!(part,
            ContentPart::Text(text) if text.contains("persist before stopping"))))));
    assert_eq!(tools.0.load(Ordering::SeqCst), 0);
    if reset {
        assert!(
            !messages.iter().any(|message| matches!(
                message,
                ModelMessage::Assistant { .. } | ModelMessage::ToolResult { .. }
            )),
            "reset must not restore the replaced answer or tool result"
        );
    }
    let audit_count: u64 = db.lock().unwrap().query_row(
        "select count(*) from agent_compaction_hook_audit where conversation_id=?1 and stage='session_start_compact'",
        [id.to_string()], |row| row.get(0),
    ).unwrap();
    assert_eq!(audit_count, if id == parent { 2 } else { 0 });
}

#[tokio::test]
async fn compact_hook_context_resumes_parent_and_nested_fork_in_fresh_processes() {
    verify_fresh_processes(false).await;
}

#[tokio::test]
async fn reset_hook_context_resumes_parent_and_nested_fork_in_fresh_processes() {
    verify_fresh_processes(true).await;
}

async fn verify_fresh_processes(reset: bool) {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("hook-resume.sqlite");
    let parent = Uuid::new_v4();
    let ((service, store, transport, tools, execution, _), db) =
        fixture(&path, parent, true, reset).await;
    *transport.responses.lock().unwrap() =
        VecDeque::from(vec![response(true), response(false), response(false)]);
    let mut execution = Arc::new(execution);
    let first = store
        .put(parent, Uuid::new_v4(), input("original"))
        .await
        .unwrap();
    assert_eq!(
        run(
            execution.clone(),
            service.clone(),
            store.clone(),
            parent,
            first.request_id
        )
        .await,
        TaskCompletion::Completed
    );
    assert_eq!(tools.0.load(Ordering::SeqCst), 1);
    let hooks = Arc::new(Hooks {
        stop_at: Some(CompactionHookStage::SessionStartCompact),
        events: Mutex::new(vec![]),
    });
    Arc::get_mut(&mut execution).unwrap().compaction_hooks = Some(hooks.clone());
    let (command, admission) =
        compact(execution.clone(), service.clone(), store.clone(), parent).await;
    assert_eq!(completed(admission).await, TaskCompletion::Completed);
    let window = service
        .active_context_checkpoint(parent)
        .await
        .unwrap()
        .unwrap()
        .window_id;
    let stopped = store
        .put(parent, Uuid::new_v4(), input("stopped"))
        .await
        .unwrap();
    assert_eq!(
        run(
            execution.clone(),
            service.clone(),
            store.clone(),
            parent,
            stopped.request_id
        )
        .await,
        TaskCompletion::Canceled
    );
    assert_eq!(
        transport.requests.lock().unwrap().len(),
        if reset { 2 } else { 3 }
    );
    let catalog = SqliteConversationCatalog::new(db.clone());
    let before = Uuid::new_v4();
    let child = Uuid::new_v4();
    let nested = Uuid::new_v4();
    for (source, target, through) in [
        (parent, before, command.turn_id),
        (parent, child, stopped.turn_id),
        (child, nested, stopped.turn_id),
    ] {
        catalog
            .fork_legacy_for_tests(ConversationForkRequest {
                child_id: ConversationId::from_uuid(target),
                parent_id: ConversationId::from_uuid(source),
                through_turn_id: through,
                binding: binding(),
            })
            .await
            .unwrap();
    }
    let connection = Arc::downgrade(&db);
    drop(catalog);
    drop(db);
    drop(execution);
    drop(service);
    drop(store);
    drop(transport);
    drop(tools);
    drop(hooks);
    assert!(
        connection.upgrade().is_none(),
        "close every disk owner before fresh process recovery"
    );
    for (id, body) in [(parent, true), (before, false), (nested, true)] {
        let mut child = tokio::process::Command::new(std::env::current_exe().unwrap());
        child
            .args(["--exact", CHILD, "--ignored", "--nocapture"])
            .env_clear()
            .env("RA_HOOK_RESUME_DB", &path)
            .env("RA_HOOK_RESUME_ID", id.to_string())
            .env("RA_HOOK_RESUME_PARENT", parent.to_string())
            .env("RA_HOOK_RESUME_WINDOW", window.to_string())
            .env("RA_HOOK_RESUME_BODY", if body { "yes" } else { "no" })
            .env("RA_HOOK_RESUME_RESET", if reset { "yes" } else { "no" })
            .kill_on_drop(true);
        let output = tokio::time::timeout(std::time::Duration::from_secs(30), child.output())
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
}
