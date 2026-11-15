//! Ordinary chat preparation reuses the original request-owned writing source loop.
use super::*;
use crate::{
    agent::context_session::prepare_status::Error,
    data_interface::{ContextPrepareIntent, ContextPrepareSource, SqlImmediateReceipt},
};

impl Binding {
    pub(crate) async fn prepare_independent(
        &self,
        database: SharedDatabase,
        command: ContextPrepareIntent,
        source: ContextPrepareSource,
    ) -> Result<(crate::agent::context_session::Preview, SqlImmediateReceipt), Error> {
        let conversation = command.request.conversation_id;
        let operation = command.request.request_id;
        let scope_required = needs_scope(&command.request.resources);
        self.run(
            database,
            operation,
            WorkerRetention::default(),
            scope_required,
            move |db, reader| db.prepare_context_with_reader(&command, source.clone(), reader),
        )
        .await
        .map_err(|error| Error::runtime(error, conversation, operation))?
        .map_err(|error| Error::scope(error, conversation, operation))
    }
}

#[cfg(all(test, target_os = "macos"))]
#[tokio::test]
#[ignore = "requires actual independent writing package; owned ordinary preparation pause and re-entry"]
async fn actual_package_ordinary_prepare_rechecks_sources_after_caller_drop() {
    use crate::{
        agent::{
            context_session::sources::{Access, Resource},
            writing_submission::{writing_test_fixture, writing_test_request},
        },
        runtime_plugins::RuntimePluginCatalog,
    };
    use research_agent_core::protocol::ModelIdentity;
    use serde_json::json;
    use std::{sync::Mutex, time::Duration};
    use uuid::Uuid;

    let root = std::fs::canonicalize(
        std::env::var_os("RESEARCH_ASSISTANT_TEST_WRITING_V2_PACKAGES").unwrap(),
    )
    .unwrap();
    assert!(
        root.starts_with(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .ancestors()
                .nth(3)
                .unwrap()
                .join(".tmp")
        )
    );
    let catalog = Arc::new(RuntimePluginCatalog::new(Some(root)));
    let packages = catalog.discover().unwrap();
    assert_eq!(packages.packages.len(), 1);
    let binding = Binding {
        service: Arc::new(NativePreviewService::new(catalog.clone())),
        catalog,
        package_key: packages.packages[0].package_key.clone(),
    };
    let f = writing_test_fixture(false);
    let store = f
        .store
        .clone()
        .without_static_writing_context_sources()
        .with_native_writing_context(binding.clone());
    let conversation = writing_test_request(&f, Uuid::new_v4()).conversation_id;
    let operation = Uuid::new_v4();
    let command = ContextPrepareIntent {
        request: serde_json::from_value(json!({
            "conversationId":conversation, "requestId":operation,
            "text":"Read the synthetic manuscript", "resources":[{
                "kind":"manuscript", "resourceId":f.manuscript.to_string(),
                "newMemberAccess":"readOnly", "groupAccess":"readOnly", "parts":[]
            }],
            "userInput":[{"type":"text", "text":"Read the synthetic manuscript", "textElements":[]},
                {"type":"mention", "name":"main.tex", "target":{
                    "kind":"file", "resourceId":f.manuscript.to_string(),
                    "sourceId":format!("context:{}", serde_json::to_string(&Resource::ManuscriptFile {
                        resource_id:f.manuscript, path:"main.tex".into(),
                    }).unwrap())
                }}]
        })).unwrap(),
        expected_epoch: None,
    };
    assert!(matches!(
        command.request.resources[0],
        Resource::Manuscript {
            new_member_access: Some(Access::ReadOnly),
            ..
        }
    ));
    let source = ContextPrepareSource {
        store: store.clone(),
        model: (
            ModelIdentity {
                provider_id: "fixture".into(),
                model_id: "fixture".into(),
                revision: None,
            },
            None,
            None,
        ),
    };
    let (entered, waiting) = tokio::sync::oneshot::channel();
    let entered = Mutex::new(Some(entered));
    let (release, gate) = std::sync::mpsc::channel();
    let gate = Mutex::new(gate);
    let (finished, completion) = tokio::sync::oneshot::channel();
    let finished = Mutex::new(Some(finished));
    let worker = binding.clone();
    let database = store.shared_agent_connection();
    let intent = command.clone();
    let caller = tokio::spawn(async move {
        worker
            .run(
                database,
                operation,
                WorkerRetention::default(),
                true,
                move |db, reader| {
                    let result = db.prepare_context_with_reader(&intent, source.clone(), reader);
                    if matches!(result, Err(Flow::Need(_))) {
                        if let Some(entered) = entered.lock().unwrap().take() {
                            // The real SQL owner has settled before this first Native wait.
                            assert!(db.lock().unwrap().is_autocommit());
                            entered.send(()).unwrap();
                            gate.lock().unwrap().recv().unwrap();
                        }
                    } else if let Some(finished) = finished.lock().unwrap().take() {
                        finished.send(result.is_ok()).unwrap();
                    }
                    result
                },
            )
            .await
    });
    tokio::time::timeout(Duration::from_secs(30), waiting)
        .await
        .unwrap()
        .unwrap();
    caller.abort();
    match caller.await {
        Err(error) => assert!(error.is_cancelled()),
        Ok(_) => panic!("the caller must be dropped while its owned worker waits"),
    }
    {
        let db = store.connection().unwrap();
        assert!(db.is_autocommit());
        let count: u64 = db
            .query_row("select count(*) from agent_submissions", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 0);
        db.execute(
            "update manuscripts set status='archived' where id=?1",
            [f.manuscript.to_string()],
        )
        .unwrap();
    }
    release.send(()).unwrap();
    assert!(
        tokio::time::timeout(Duration::from_secs(30), completion)
            .await
            .unwrap()
            .unwrap()
    );
    // The existing owned worker survives a dropped caller. Re-entry follows the
    // original unavailable-source rule and grants no authority to the archived object.
    let (frozen, receipt) = store
        .shared_agent_connection()
        .replay_context_prepare(&command)
        .unwrap()
        .unwrap();
    assert!(frozen.input.context.is_empty());
    assert_eq!(frozen.input.user_input, command.request.user_input);
    assert_eq!(receipt.operation_id, operation);
    assert!(receipt.replayed);
    assert!(store.connection().unwrap().is_autocommit());
    binding.service.shutdown().await.unwrap();
    store.shared_agent_connection().shutdown().unwrap();
}
