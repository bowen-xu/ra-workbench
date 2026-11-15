//! Original proven entry with Core storage and an actual independent writing package.
use super::*;
use crate::{
    agent::{
        context_session::sources::{Access, Resource},
        writing_submission::{writing_test_fixture, writing_test_request},
    },
    runtime_plugins::{
        RuntimePluginCatalog, native::NativePreviewService, writing_projects::context::Binding,
    },
};
use serde_json::{Value, json};
use std::sync::Arc;

fn binding() -> Binding {
    let root = std::fs::canonicalize(
        std::env::var_os("RESEARCH_ASSISTANT_TEST_WRITING_V2_PACKAGES")
            .expect("actual independent writing package collection"),
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
    Binding {
        service: Arc::new(NativePreviewService::new(catalog.clone())),
        catalog,
        package_key: packages.packages[0].package_key.clone(),
    }
}
fn request(conversation: Uuid, manuscript: Uuid) -> PrepareRequest {
    PrepareRequest {
        conversation_id: conversation,
        request_id: Uuid::new_v4(),
        text: "Read the synthetic manuscript and Core note".into(),
        resources: vec![Resource::Manuscript {
            resource_id: manuscript.to_string(),
            new_member_access: Some(Access::ReadOnly),
            group_access: Some(Access::ReadOnly),
            parts: vec![],
        }],
        user_input: vec![],
        retrieval: None,
        model: None,
        queue: None,
    }
}
fn result(response: &Response) -> &Preview {
    let Status::Committed { result, .. } = &response.status else {
        panic!("expected exact committed preparation")
    };
    result
}
fn wire_error(error: Error) -> Value {
    serde_json::to_value(error).unwrap()
}

#[tokio::test]
#[ignore = "requires actual independent writing package; ordinary proven preparation"]
async fn actual_package_prepares_readonly_manuscript_with_core_note_and_original_reference_semantics()
 {
    let mut f = writing_test_fixture(false);
    crate::storage::migrate(&mut f.store.connection().unwrap()).unwrap();
    let conversation = writing_test_request(&f, Uuid::new_v4()).conversation_id;
    let native = binding();
    let static_store = f.store.clone();
    f.store = f
        .store
        .without_static_writing_context_sources()
        .without_static_writing_workspace_source()
        .without_static_writing_compile_sources()
        .with_native_writing_context(native.clone());
    let note = Uuid::new_v4();
    f.store.connection().unwrap().execute(
        "insert into note_pages(id,title,created_at,updated_at) values(?1,'Core source note','now','now')",
        [note.to_string()],
    ).unwrap();
    let mut input = request(conversation, f.manuscript);
    input.resources.push(Resource::Note { resource_id: note });
    let file = Resource::ManuscriptFile {
        resource_id: f.manuscript,
        path: "main.tex".into(),
    };
    input.user_input = serde_json::from_value(json!([{
        "type":"text","text":input.text,"textElements":[]
    }, {
        "type":"mention","name":"main.tex", "target":{
            "kind":"file","sourceId":format!("context:{}", serde_json::to_string(&file).unwrap()),
            "resourceId":f.manuscript.to_string()
        }
    }]))
    .unwrap();
    let first = prepare(&f.store, &f.models, input.clone(), None)
        .await
        .unwrap();
    let frozen = result(&first);
    assert_eq!(frozen.input.context.len(), 2);
    assert!(frozen.input.context.iter().all(|source| {
        let anchor: Value = serde_json::from_str(source.anchor.as_deref().unwrap()).unwrap();
        anchor["contextAccess"] == "readOnly"
    }));
    let mut comparison = input.clone();
    comparison.request_id = Uuid::new_v4();
    let original = prepare(&static_store, &f.models, comparison, None)
        .await
        .unwrap();
    assert_eq!(frozen.input, result(&original).input);
    let response = serde_json::to_value(&first).unwrap();
    assert_eq!(
        response["status"]["proof"]["operationId"],
        input.request_id.to_string()
    );
    let count: u64 = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "select count(*) from data_sql_operation_receipts where operation_id=?1",
            [input.request_id.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
    let missing = f.store.clone().with_native_writing_context(Binding {
        package_key: "0".repeat(64),
        ..native.clone()
    });
    let replay = prepare(&missing, &f.models, input.clone(), None)
        .await
        .unwrap();
    assert_eq!(result(&replay).turn_id, frozen.turn_id);
    assert_eq!(result(&replay).input, frozen.input);
    assert_eq!(
        serde_json::to_value(replay).unwrap()["status"]["proof"]["replayed"],
        true
    );
    let mut new_request = input;
    new_request.request_id = Uuid::new_v4();
    let failure = wire_error(
        prepare(&missing, &f.models, new_request.clone(), None)
            .await
            .err()
            .unwrap(),
    );
    assert_eq!(failure["code"], "storage");
    assert_eq!(
        failure["contextSource"]["requestId"],
        new_request.request_id.to_string()
    );
    assert_eq!(
        failure["contextSource"]["conversationId"],
        conversation.to_string()
    );
    assert_eq!(
        failure["contextSource"]["source"]["error"]["packageKey"],
        "0".repeat(64)
    );
    assert!(f.store.connection().unwrap().is_autocommit());
    assert!(
        f.store
            .shared_agent_connection()
            .replay_context_prepare(&ContextPrepareIntent {
                request: new_request,
                expected_epoch: None,
            })
            .unwrap()
            .is_none()
    );
    native.service.shutdown().await.unwrap();
    f.store.shared_agent_connection().shutdown().unwrap();
}

#[tokio::test]
#[ignore = "requires actual independent writing package; original epoch and Core-only paths"]
async fn actual_package_keeps_epoch_precedence_and_core_only_preparation_independent() {
    let mut f = writing_test_fixture(false);
    crate::storage::migrate(&mut f.store.connection().unwrap()).unwrap();
    let conversation = writing_test_request(&f, Uuid::new_v4()).conversation_id;
    let mut native = binding();
    native.package_key = "0".repeat(64);
    f.store = f
        .store
        .without_static_writing_context_sources()
        .with_native_writing_context(native.clone());
    let mut input = request(conversation, f.manuscript);
    let failure = wire_error(
        prepare(&f.store, &f.models, input.clone(), Some(Uuid::new_v4()))
            .await
            .err()
            .unwrap(),
    );
    assert_eq!(failure["reason"]["code"], "DATA_SQL_OPERATION_CONFLICT");
    assert!(failure.get("contextSource").is_none());
    input.resources.clear();
    let core = prepare(&f.store, &f.models, input, None).await.unwrap();
    assert!(result(&core).input.context.is_empty());
    native.service.shutdown().await.unwrap();
    f.store.shared_agent_connection().shutdown().unwrap();
}

#[path = "runtime_tests/mixed.rs"]
mod mixed;

#[tokio::test]
#[ignore = "requires actual independent writing package; original unresolved-file reference semantics"]
async fn actual_package_keeps_unavailable_file_mentions_as_original_text_without_new_authority() {
    let mut f = writing_test_fixture(false);
    let native = binding();
    let conversation = writing_test_request(&f, Uuid::new_v4()).conversation_id;
    let static_store = f.store.clone();
    f.store = f
        .store
        .without_static_writing_context_sources()
        .with_native_writing_context(native.clone());
    let resource = Resource::ManuscriptFile {
        resource_id: f.manuscript,
        path: "main.tex".into(),
    };
    let path = f
        .store
        .manuscript_directory()
        .join(f.manuscript.to_string())
        .join("source/main.tex");
    std::fs::remove_file(path).unwrap();
    let mut input = request(conversation, f.manuscript);
    input.user_input = serde_json::from_value(json!([
        {"type":"text", "text":input.text, "textElements":[]},
        {"type":"mention", "name":"main.tex", "target": {
            "kind":"file",
            "sourceId":format!("context:{}", serde_json::to_string(&resource).unwrap()),
            "resourceId":f.manuscript.to_string()
        }}
    ]))
    .unwrap();
    let original = prepare(&static_store, &f.models, input.clone(), None)
        .await
        .unwrap();
    input.request_id = Uuid::new_v4();
    let runtime = prepare(&f.store, &f.models, input.clone(), None)
        .await
        .unwrap();
    assert_eq!(result(&runtime).input, result(&original).input);
    assert_eq!(result(&runtime).input.user_input, input.user_input);
    assert_eq!(result(&runtime).input.context.len(), 1);
    native.service.shutdown().await.unwrap();
    f.store.shared_agent_connection().shutdown().unwrap();
}

#[tokio::test]
#[ignore = "requires actual independent writing package; original queue claim and exact replay"]
async fn actual_package_ordinary_queue_preserves_original_claim_and_replay() {
    use crate::agent::repository::input_queue;
    use research_agent_core::input_queue::{QueueEdit, QueuedInput};
    let mut f = writing_test_fixture(false);
    let native = binding();
    let conversation = writing_test_request(&f, Uuid::new_v4()).conversation_id;
    f.store = f
        .store
        .without_static_writing_context_sources()
        .with_native_writing_context(native.clone());
    let mut input = request(conversation, f.manuscript);
    let file = Resource::ManuscriptFile {
        resource_id: f.manuscript,
        path: "main.tex".into(),
    };
    input.user_input = serde_json::from_value(json!([
        {"type":"text", "text":input.text, "textElements":[]},
        {"type":"mention", "name":"main.tex", "target": {
            "kind":"file",
            "sourceId":format!("context:{}", serde_json::to_string(&file).unwrap()),
            "resourceId":f.manuscript.to_string()
        }}
    ]))
    .unwrap();
    let queued = Uuid::new_v4();
    input_queue::apply_checked(
        &mut f.store.connection().unwrap(),
        conversation,
        Uuid::new_v4(),
        0,
        QueueEdit::Add(QueuedInput {
            id: queued,
            text: input.text.clone(),
            user_input: input.user_input.clone(),
            origin: None,
        }),
        |_| Ok(()),
    )
    .unwrap();
    input.queue = Some(crate::agent::queue_contract::Confirmation {
        input_id: queued,
        expected_revision: 1,
    });
    let first = prepare(&f.store, &f.models, input.clone(), None)
        .await
        .unwrap();
    let frozen = result(&first);
    assert_eq!(frozen.queue.as_ref().unwrap().input_id, queued);
    assert_eq!(frozen.queue.as_ref().unwrap().expected_revision, 2);
    let missing = f.store.clone().with_native_writing_context(Binding {
        package_key: "0".repeat(64),
        ..native.clone()
    });
    let replay = prepare(&missing, &f.models, input.clone(), None)
        .await
        .unwrap();
    assert_eq!(result(&replay).turn_id, frozen.turn_id);
    assert_eq!(result(&replay).input, frozen.input);
    assert_eq!(result(&replay).queue.as_ref().unwrap().input_id, queued);
    assert_eq!(result(&replay).queue.as_ref().unwrap().expected_revision, 2);
    let mut changed = input;
    changed.queue.as_mut().unwrap().expected_revision = 2;
    let failure = wire_error(
        prepare(&missing, &f.models, changed, None)
            .await
            .err()
            .unwrap(),
    );
    assert_eq!(failure["reason"]["code"], "DATA_SQL_OPERATION_CONFLICT");
    assert!(failure.get("contextSource").is_none());
    native.service.shutdown().await.unwrap();
    f.store.shared_agent_connection().shutdown().unwrap();
}
