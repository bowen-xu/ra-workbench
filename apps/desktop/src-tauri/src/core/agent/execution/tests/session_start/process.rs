use super::*;
use crate::agent::{compaction_hook_process::*, process::ProcessSpec};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};

struct Commands {
    root: PathBuf,
    script: &'static str,
    calls: AtomicUsize,
}
impl CompactionHookCommandSource for Commands {
    fn prepare(
        &self,
        request: &CompactionHookRequest,
    ) -> Result<Option<AuthorizedHookCommand>, HookCommandError> {
        if request.stage != CompactionHookStage::SessionStartCompact {
            return Ok(None);
        }
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(Some(AuthorizedHookCommand {
            process: ProcessSpec {
                program: "/bin/sh".into(),
                arguments: vec!["-c".into(), self.script.into()],
                directory: self.root.clone(),
                environment: Default::default(),
            },
            transcript_path: None,
            launcher: None,
        }))
    }
}

#[tokio::test]
async fn native_process_hook_stops_sampling_persists_body_and_resumes_without_reexecution() {
    let ((service, store, transport, _, mut execution, id), db) = setup_with_connection().await;
    let root = tempfile::tempdir().unwrap();
    let commands = Arc::new(Commands {
        root: root.path().into(),
        script: "cat > hook-input.json; printf '%s' '{\"continue\":false,\"hookSpecificOutput\":{\"hookEventName\":\"SessionStart\",\"additionalContext\":\"native hook body\"}}'",
        calls: AtomicUsize::new(0),
    });
    execution.compaction_hooks = Some(Arc::new(
        ProcessCompactionHooks::new(commands.clone(), Duration::from_secs(2), 1024).unwrap(),
    ));
    *transport.responses.lock().unwrap() =
        VecDeque::from(vec![response(false), response(false), response(false)]);
    let execution = Arc::new(execution);
    let initial = store
        .put(id, Uuid::new_v4(), input("original input"))
        .await
        .unwrap();
    assert_eq!(
        run(
            execution.clone(),
            service.clone(),
            store.clone(),
            id,
            initial.request_id
        )
        .await,
        TaskCompletion::Completed
    );
    let (_, admission) = compact(execution.clone(), service.clone(), store.clone(), id).await;
    assert_eq!(completed(admission).await, TaskCompletion::Completed);
    let stopped = store
        .put(id, Uuid::new_v4(), input("stopped input"))
        .await
        .unwrap();
    assert_eq!(
        run(
            execution.clone(),
            service.clone(),
            store.clone(),
            id,
            stopped.request_id
        )
        .await,
        TaskCompletion::Canceled
    );
    assert_eq!(transport.requests.lock().unwrap().len(), 2);
    let saved: String = db
        .lock()
        .unwrap()
        .query_row(
            "select contexts_json from agent_compact_hook_contexts",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Vec<String>>(&saved).unwrap(),
        ["native hook body"]
    );
    let mut fresh_input = input("fresh input");
    fresh_input.text = "fresh input".into();
    let fresh = store.put(id, Uuid::new_v4(), fresh_input).await.unwrap();
    assert_eq!(
        run(execution, service.clone(), store, id, fresh.request_id).await,
        TaskCompletion::Completed
    );
    assert_eq!(commands.calls.load(Ordering::SeqCst), 1);
    let requests = transport.requests.lock().unwrap();
    let messages = &requests.last().unwrap().messages;
    let positions: Vec<_> = messages
        .iter()
        .enumerate()
        .filter_map(|(index, message)| {
            matches!(message, ModelMessage::Developer(body) if body == "native hook body")
                .then_some(index)
        })
        .collect();
    assert_eq!(positions.len(), 1);
    let fresh_index = messages.iter().rposition(|message| matches!(message, ModelMessage::User { content: parts, .. } if parts.iter().any(|part| matches!(part, ContentPart::Text(text) if text == "fresh input")))).unwrap();
    assert!(positions[0] < fresh_index);
}

#[derive(Default)]
struct HandlerEvents(Mutex<Vec<(Uuid, CompactionHookNotification)>>);
impl HookHandlerObserver for HandlerEvents {
    fn notify(
        &self,
        id: Uuid,
        _: &CompactionHookRequest,
        notification: CompactionHookNotification,
    ) {
        self.0.lock().unwrap().push((id, notification));
    }
}

#[tokio::test]
async fn native_concurrent_hook_commands_preserve_successful_context_when_one_fails() {
    let ((service, store, transport, _, mut execution, id), db) = setup_with_connection().await;
    let root = tempfile::tempdir().unwrap();
    let observer = Arc::new(HandlerEvents::default());
    let scripts = [
        "cat >/dev/null; while [ ! -f second-ready ]; do /bin/sleep 0.01; done; printf 'first context'",
        "cat >/dev/null; touch second-ready; printf '%s' '{\"continue\":false,\"hookSpecificOutput\":{\"hookEventName\":\"SessionStart\",\"additionalContext\":\"second context\"}}'",
        "cat >/dev/null; printf 'failed context'; exit 1",
    ];
    let commands: Vec<_> = scripts
        .into_iter()
        .map(|script| {
            Arc::new(Commands {
                root: root.path().into(),
                script,
                calls: AtomicUsize::new(0),
            })
        })
        .collect();
    let handlers = commands
        .iter()
        .enumerate()
        .map(|(index, commands)| CompactionHookHandler {
            id: Uuid::from_u128(index as u128 + 10),
            stage: CompactionHookStage::SessionStartCompact,
            hooks: Arc::new(
                ProcessCompactionHooks::new(commands.clone(), Duration::from_secs(2), 1024)
                    .unwrap(),
            ),
        })
        .collect();
    execution.compaction_hooks = Some(Arc::new(
        ConcurrentCompactionHooks::new(handlers, observer.clone()).unwrap(),
    ));
    *transport.responses.lock().unwrap() =
        VecDeque::from(vec![response(false), response(false), response(false)]);
    let execution = Arc::new(execution);
    let initial = store
        .put(id, Uuid::new_v4(), input("multi original"))
        .await
        .unwrap();
    assert_eq!(
        run(
            execution.clone(),
            service.clone(),
            store.clone(),
            id,
            initial.request_id
        )
        .await,
        TaskCompletion::Completed
    );
    let (_, admission) = compact(execution.clone(), service.clone(), store.clone(), id).await;
    assert_eq!(completed(admission).await, TaskCompletion::Completed);
    let stopped = store
        .put(id, Uuid::new_v4(), input("multi stopped"))
        .await
        .unwrap();
    assert_eq!(
        run(
            execution.clone(),
            service.clone(),
            store.clone(),
            id,
            stopped.request_id
        )
        .await,
        TaskCompletion::Canceled
    );
    assert_eq!(transport.requests.lock().unwrap().len(), 2);
    let saved: String = db
        .lock()
        .unwrap()
        .query_row(
            "select contexts_json from agent_compact_hook_contexts",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Vec<String>>(&saved).unwrap(),
        ["first context", "second context"]
    );
    let durable: Vec<(String, Option<String>)> = db.lock().unwrap()
        .prepare("select a.notification,h.outcome from agent_compaction_hook_handlers h join agent_compaction_hook_audit a on a.seq=h.audit_seq order by a.seq,h.display_order")
        .unwrap().query_map([], |row| Ok((row.get(0)?,row.get(1)?))).unwrap()
        .collect::<Result<_,_>>().unwrap();
    assert_eq!(
        durable,
        vec![
            ("started".into(), None),
            ("started".into(), None),
            ("started".into(), None),
            ("completed".into(), Some("continue".into())),
            ("completed".into(), Some("stop".into())),
            ("completed".into(), Some("failed".into())),
        ]
    );
    let mut fresh_input = input("multi fresh");
    fresh_input.text = "multi fresh".into();
    let fresh = store.put(id, Uuid::new_v4(), fresh_input).await.unwrap();
    assert_eq!(
        run(execution, service, store, id, fresh.request_id).await,
        TaskCompletion::Completed
    );
    for commands in commands {
        assert_eq!(commands.calls.load(Ordering::SeqCst), 1);
    }
    let events = observer.0.lock().unwrap();
    assert_eq!(events.len(), 6);
    assert!(events.contains(&(
        Uuid::from_u128(12),
        CompactionHookNotification::Completed(CompactionHookOutcome::Failed)
    )));
    let requests = transport.requests.lock().unwrap();
    let messages = &requests.last().unwrap().messages;
    let bodies: Vec<_> = messages
        .iter()
        .filter_map(|message| match message {
            ModelMessage::Developer(body) if body.ends_with("context") => Some(body.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(bodies, ["first context", "second context"]);
}
