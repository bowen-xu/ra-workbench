//! Actual process-boundary recovery, using the native owner and disk SQLite.
//! Only model responses and the approved notes MCP service are synthetic.
use super::*;
use std::path::Path;

const CHILD: &str =
    "agent::execution::tests::accounting::token_budget::guidance::resume::resume_child";

async fn fixture_at(
    path: &Path,
    id: Uuid,
    create: bool,
) -> (ExecutionFixture, crate::data_interface::SharedDatabase) {
    // Reuse the common execution configuration and deterministic provider ports.
    let (_, _, transport, tools, mut execution, _) = setup().await;
    let mut db = Connection::open(path).unwrap();
    crate::storage::migrate(&mut db).unwrap();
    let app = crate::storage::AppStore::for_test(db, path.parent().unwrap().join("objects"));
    let service = Arc::new(AgentSessionService::from_app_store(&app).unwrap());
    let db = app.shared_agent_connection();
    let store = Arc::new(SqliteSubmissionStore::new(db.clone()));
    if create {
        service.create(ConversationId::from_uuid(id)).await.unwrap();
    } else {
        service.open(ConversationId::from_uuid(id)).await.unwrap();
    }
    buffered(&mut execution, transport.clone(), 120, true);
    ((service, store, transport, tools, execution, id), db)
}

#[tokio::test]
#[ignore = "Internal subprocess entry; invoked by the fresh-process recovery test"]
async fn resume_child() {
    let path = std::env::var("RA_BUDGET_RESUME_DB").unwrap();
    let expected_path = std::env::var("RA_BUDGET_RESUME_METADATA").unwrap();
    let id = Uuid::parse_str(&std::env::var("RA_BUDGET_RESUME_ID").unwrap()).unwrap();
    let ((service, store, transport, tools, execution, _), _) =
        fixture_at(Path::new(&path), id, false).await;
    // A persisted snapshot must bypass fresh external reads, even if the new
    // runtime's notes server would hang. Approval is still the normal fixture.
    let notes = install_hint_runtime(&service, "hang").await;
    *transport.responses.lock().unwrap() = VecDeque::from(vec![response(false)]);
    let mut submission = input("resumed");
    submission.text = "fresh-process input".into();
    let record = store.put(id, Uuid::new_v4(), submission).await.unwrap();
    assert_eq!(
        run(
            Arc::new(execution),
            service.clone(),
            store,
            id,
            record.request_id
        )
        .await,
        TaskCompletion::Completed
    );
    assert!(
        service
            .active_context_checkpoint(id)
            .await
            .unwrap()
            .is_none()
    );
    let requests = transport.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(
        metadata(&request.messages),
        vec![std::fs::read_to_string(expected_path).unwrap()]
    );
    assert_eq!(fragments(request), vec!["prepare for reset"]);
    let prompt = request.messages.iter().position(|message| {
        matches!(message, ModelMessage::Developer(body) if body == "prepare for reset")
    }).unwrap();
    assert!(matches!(
        request.messages[prompt - 1],
        ModelMessage::ToolResult { .. }
    ));
    assert!(
        request.messages[prompt + 1..]
            .iter()
            .any(|message| matches!(
                message,
                ModelMessage::Assistant { .. } | ModelMessage::CanonicalOutput(_)
            ))
    );
    assert!(request.messages[prompt + 1..].iter().any(|message| {
        matches!(message, ModelMessage::User { content: parts, .. } if parts == &vec![ContentPart::Text("fresh-process input".into())])
    }));
    assert_eq!(
        request
            .messages
            .iter()
            .filter(|message| matches!(message, ModelMessage::ToolResult { .. }))
            .count(),
        1
    );
    assert_eq!(
        tools.0.load(Ordering::SeqCst),
        0,
        "durable tools must not execute again"
    );
    assert!(
        !notes.path().join("calls").exists(),
        "saved hints must not be refreshed on restart"
    );
}

#[tokio::test]
async fn token_budget_metadata_and_prompts_resume_in_a_fresh_process() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("resume.sqlite3");
    let expected = root.path().join("expected-metadata.txt");
    let id = Uuid::new_v4();
    let ((service, store, transport, tools, execution, _), db) = fixture_at(&path, id, true).await;
    let notes = install_hint_runtime(&service, "text").await;
    *transport.responses.lock().unwrap() = VecDeque::from(vec![response(true), response(false)]);
    let record = store
        .put(id, Uuid::new_v4(), input("original"))
        .await
        .unwrap();
    assert_eq!(
        run(
            Arc::new(execution),
            service.clone(),
            store.clone(),
            id,
            record.request_id
        )
        .await,
        TaskCompletion::Completed
    );
    {
        let requests = transport.requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        let bodies = metadata(&requests[0].messages);
        assert_eq!(bodies.len(), 1);
        assert!(bodies[0].contains(" saved notes "));
        std::fs::write(&expected, &bodies[0]).unwrap();
        assert_eq!(fragments(&requests[1]), vec!["prepare for reset"]);
    }
    assert_eq!(tools.0.load(Ordering::SeqCst), 1);
    let connection = Arc::downgrade(&db);
    drop(db);
    drop(service);
    drop(store);
    drop(transport);
    drop(tools);
    drop(notes);
    assert!(
        connection.upgrade().is_none(),
        "parent DB owner must close before restart"
    );
    let mut child = tokio::process::Command::new(std::env::current_exe().unwrap());
    child
        .args(["--exact", CHILD, "--ignored", "--nocapture"])
        .env_clear()
        .env("RA_BUDGET_RESUME_DB", &path)
        .env("RA_BUDGET_RESUME_ID", id.to_string())
        .env("RA_BUDGET_RESUME_METADATA", &expected)
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

fn fragments(request: &ModelRequest) -> Vec<&str> {
    request
        .messages
        .iter()
        .filter_map(|message| match message {
            ModelMessage::Developer(body) if !body.starts_with("<context_window>\n") => {
                Some(body.as_str())
            }
            _ => None,
        })
        .collect()
}
