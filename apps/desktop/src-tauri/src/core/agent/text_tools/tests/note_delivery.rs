//! Original note transactions supply immutable facts; retries only notify readers.
use super::*;
use crate::{
    runtime_events::RuntimeEvents,
    storage::{AppStorePaths, AppStorePorts},
};
use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};
#[path = "note_delivery/transaction.rs"]
mod transaction;

#[derive(Default)]
struct Events {
    release_failed: AtomicBool,
    receipt_failed: AtomicBool,
    delivered: Mutex<Vec<(String, Value)>>,
}
fn with_events() -> (Fixture, Arc<Events>) {
    let events = Arc::new(Events::default());
    let control = events.clone();
    let f = fixture_with_store(true, move |root| open_store(root, control));
    (f, events)
}
fn open_store(root: &std::path::Path, control: Arc<Events>) -> AppStore {
    let events = RuntimeEvents::new(move |name, payload| {
        if (name == "research-note-text-release" && control.release_failed.load(Ordering::SeqCst))
            || (name == "research-note-text-receipt"
                && control.receipt_failed.load(Ordering::SeqCst))
        {
            return Err("private synthetic delivery transport cause".into());
        }
        control
            .delivered
            .lock()
            .unwrap()
            .push((name.into(), payload));
        Ok(())
    });
    AppStore::open_with_paths(
        AppStorePaths {
            data_directory: root.join("runtime"),
            agent_configuration_directory: root.join("configuration"),
        },
        AppStorePorts {
            native_note_trace: None,
            writing_context_source: None,
            writing_resource_source: Some(crate::agent::writing_resources::test_sources::source()),
            paper_metadata_source: None,
            retrieval_trace_source: None,
            retrieval_preview_source: None,
            semantic_index_source: None,
            paper_portrait_source: None,
            context_catalog_source: crate::context_catalog_app::source(),
            generation_policy_source: None,
            paper_generation_source: None,
            paper_reading_source: None,
            paper_analysis_source: None,
            candidate_recovery_source: None,
            writing_compile_feedback_source: None,
            #[cfg(target_os = "macos")]
            writing_workspace_compile_source: None,
            #[cfg(unix)]
            writing_workspace_source: None,
            text_editors: Arc::new(editor_barrier::EditorBarrier::for_events(events.clone())),
            writing_delivery_tools: Arc::new(
                crate::agent::writing_delivery_tool::DeliveryTools::for_events(events.clone()),
            ),
            events,
        },
    )
    .unwrap()
}
fn call(f: &Fixture, id: &str) -> ToolCall {
    ToolCall {
        namespace: None,
        raw_arguments_json: None,
        call_id: id.into(),
        name: PATCH.into(),
        arguments: json!({"patch":patch(&path(f.page),"old","new")}),
    }
}
fn key(f: &Fixture, id: &str) -> String {
    format!("{}:{id}", f.tools.record.request_id)
}
fn source(f: &Fixture) -> Value {
    serde_json::to_value(
        note::load(&f.tools.store.connection().unwrap(), &f.page.to_string()).unwrap(),
    )
    .unwrap()
}
fn later_input(f: &Fixture) -> Value {
    let mut db = f.tools.store.connection().unwrap();
    let current = note::load(&db, &f.page.to_string()).unwrap();
    let mut document = current.document.clone();
    document.blocks[0].content["items"][0]["text"] = json!("later manual input");
    let tx = db.transaction().unwrap();
    crate::notes::save_document_in(
        &tx,
        crate::notes::NoteDocumentSaveRequest {
            document,
            expected_revision: current.revision,
            save_reason: "manual".into(),
        },
    )
    .unwrap();
    tx.commit().unwrap();
    serde_json::to_value(note::load(&db, &f.page.to_string()).unwrap()).unwrap()
}
async fn unknown(f: &Fixture, request: ToolCall, phase: &str) -> Value {
    let result = f.tools.authorize_and_execute(request).await;
    let Err(RunError::ToolOutcomeUnknownDiagnostic(detail)) = result else {
        panic!("original note delivery must retain its recovery diagnostic");
    };
    let detail = serde_json::to_value(detail).unwrap();
    assert_eq!(detail["code"], "TEXT_PATCH_DELIVERY_UNCONFIRMED");
    assert_eq!(detail["phase"], phase);
    assert_eq!(
        detail["conversationId"],
        json!(f.tools.record.conversation_id)
    );
    assert_eq!(detail["requestId"], json!(f.tools.record.request_id));
    assert!(!detail.to_string().contains("private synthetic"));
    assert!(!detail.to_string().contains("later manual input"));
    detail
}
fn forbid_rewrite(f: &Fixture) {
    f.tools.store.connection().unwrap().execute_batch(
        "create temp trigger no_note_rewrite before update on main.note_pages begin select raise(abort,'must not rewrite source'); end;
         create temp trigger no_note_candidate_rewrite before update on main.agent_note_patch_candidates begin select raise(abort,'must not rewrite candidate'); end;"
    ).unwrap();
}

#[tokio::test]
async fn immutable_delivery_records_actual_saved_revision_and_keeps_original_baseline_after_later_input()
 {
    let f = fixture(true);
    read(&f, &path(f.page)).await;
    let request = call(&f, "immutable");
    let output = f
        .tools
        .authorize_and_execute(request.clone())
        .await
        .unwrap();
    assert!(!output.is_error);
    let target = format!("{}:notes", key(&f, "immutable"));
    let original = {
        let db = f.tools.store.connection().unwrap();
        let delivery = note::delivery::read(&db, &f.tools.record, &target)
            .unwrap()
            .unwrap();
        assert_eq!(delivery.pages.len(), 1);
        assert_eq!(
            serde_json::to_value(&delivery.pages[0]).unwrap(),
            serde_json::to_value(note::load(&db, &f.page.to_string()).unwrap()).unwrap()
        );
        assert!(
            serde_json::to_string(&delivery.reviews)
                .unwrap()
                .contains("old")
        );
        serde_json::to_value((&delivery.pages, &delivery.reviews)).unwrap()
    };
    let later = later_input(&f);
    forbid_rewrite(&f);
    let repeated = f.tools.authorize_and_execute(request).await.unwrap();
    assert_eq!(repeated.content, output.content);
    assert_eq!(source(&f), later);
    let reopened = Connection::open(f._root.path().join("fixture.sqlite3")).unwrap();
    let delivery = note::delivery::read(&reopened, &f.tools.record, &target)
        .unwrap()
        .unwrap();
    assert_eq!(
        serde_json::to_value((&delivery.pages, &delivery.reviews)).unwrap(),
        original
    );
}

#[tokio::test]
async fn subsequent_patch_delivers_original_review_baseline_independent_from_history_card() {
    let f = fixture(true);
    read(&f, &path(f.page)).await;
    f.tools
        .authorize_and_execute(call(&f, "first"))
        .await
        .unwrap();
    let second = ToolCall {
        namespace: None,
        raw_arguments_json: None,
        call_id: "second".into(),
        name: PATCH.into(),
        arguments: json!({"patch":patch(&path(f.page),"new","third")}),
    };
    f.tools.authorize_and_execute(second).await.unwrap();
    let db = f.tools.store.connection().unwrap();
    let delivery = note::delivery::read(
        &db,
        &f.tools.record,
        &format!("{}:notes", key(&f, "second")),
    )
    .unwrap()
    .unwrap();
    let baseline = &delivery.reviews[&f.page.to_string()].baseline;
    assert_eq!(baseline.blocks[0].content["items"][0]["text"], "old");
    assert_eq!(
        delivery.pages[0].document.blocks[0].content["items"][0]["text"],
        "third"
    );
    let frozen: String = db.query_row(
        "select notes_json from agent_note_patch_snapshots where conversation_id=?1 order by generation desc limit 1",
        [f.tools.record.conversation_id.to_string()], |row| row.get(0),
    ).unwrap();
    let notes: Vec<note::PendingNote> = serde_json::from_str(&frozen).unwrap();
    assert_eq!(
        note_review::baseline_document(&notes[0]).blocks[0].content["items"][0]["text"],
        "new"
    );
}

#[tokio::test]
async fn invalid_delivery_reference_after_save_is_unknown_then_recovers_exact_original_facts() {
    let (f, events) = with_events();
    read(&f, &path(f.page)).await;
    let call_key = key(&f, "unreadable");
    let target = format!("{call_key}:notes");
    {
        let db = f.tools.store.connection().unwrap();
        db.execute_batch("create temp table delivery_fault(call_id text,target_id text,original_json text);
            create temp trigger corrupt_saved_delivery after update on main.agent_text_patch_calls
            when new.state='complete' and exists(select 1 from delivery_fault where call_id=new.request_id)
            begin
              update delivery_fault set original_json=(select results_json from main.agent_text_patch_calls where request_id=delivery_fault.target_id);
              update agent_text_patch_calls set results_json=json_set(results_json,'$.noteDelivery.schemaVersion',999) where request_id in(select target_id from delivery_fault);
            end;").unwrap();
        db.execute(
            "insert into delivery_fault(call_id,target_id) values(?1,?2)",
            params![call_key, target],
        )
        .unwrap();
    }
    let request = call(&f, "unreadable");
    assert_eq!(
        unknown(&f, request.clone(), "note-delivery-read").await["safeCause"],
        "note_delivery_snapshot_invalid"
    );
    let saved = source(&f);
    assert_eq!(saved["revision"], 2);
    {
        let db = f.tools.store.connection().unwrap();
        db.execute_batch("drop trigger corrupt_saved_delivery;
            update agent_text_patch_calls set results_json=(select original_json from delivery_fault where target_id=agent_text_patch_calls.request_id) where request_id in(select target_id from delivery_fault);
            drop table delivery_fault;").unwrap();
    }
    let later = later_input(&f);
    forbid_rewrite(&f);
    let output = f.tools.authorize_and_execute(request).await.unwrap();
    assert!(!output.is_error);
    assert_eq!(source(&f), later);
    let events = events.delivered.lock().unwrap();
    let receipt = &events
        .iter()
        .find(|(name, _)| name == "research-note-text-receipt")
        .unwrap()
        .1;
    assert_eq!(receipt["pages"][0]["revision"], 2);
    assert_eq!(receipt["pages"][0]["document"], saved["document"]);
    assert!(
        receipt["pages"][0]["review"]["baseline"]
            .to_string()
            .contains("old")
    );
}

#[tokio::test]
async fn lost_native_publication_retries_original_receipt_without_reapplying_the_note() {
    let (f, events) = with_events();
    read(&f, &path(f.page)).await;
    events.release_failed.store(true, Ordering::SeqCst);
    let request = call(&f, "publish-lost");
    let detail = unknown(&f, request.clone(), "note-delivery-publish").await;
    assert_eq!(detail["safeCause"], "note_delivery_publish_failed");
    let saved = source(&f);
    forbid_rewrite(&f);
    events.release_failed.store(false, Ordering::SeqCst);
    events.receipt_failed.store(true, Ordering::SeqCst);
    unknown(&f, request.clone(), "note-delivery-publish").await;
    assert_eq!(source(&f), saved);
    events.receipt_failed.store(false, Ordering::SeqCst);
    let output = f
        .tools
        .authorize_and_execute(request.clone())
        .await
        .unwrap();
    assert!(!output.is_error);
    let again = f.tools.authorize_and_execute(request).await.unwrap();
    assert_eq!(again.content, output.content);
    assert_eq!(source(&f), saved);
    let delivered = events.delivered.lock().unwrap();
    let receipts = delivered
        .iter()
        .filter(|(name, _)| name == "research-note-text-receipt")
        .map(|(_, value)| value)
        .collect::<Vec<_>>();
    assert_eq!(receipts.len(), 2);
    assert_eq!(receipts[0], receipts[1]);
    assert_eq!(receipts[0]["pages"][0]["document"], saved["document"]);
}

#[tokio::test]
async fn reopening_original_sql_log_pair_recovers_lost_delivery_without_rewriting_later_input() {
    let (f, events) = with_events();
    read(&f, &path(f.page)).await;
    events.release_failed.store(true, Ordering::SeqCst);
    let request = call(&f, "cold-delivery");
    unknown(&f, request.clone(), "note-delivery-publish").await;
    let original = source(&f);
    let later = later_input(&f);
    let record = f.tools.record.clone();
    let Fixture {
        _root,
        tools,
        page,
        parent,
    } = f;
    drop(tools);
    events.release_failed.store(false, Ordering::SeqCst);
    let reopened = Fixture {
        tools: TextTools {
            store: open_store(_root.path(), events.clone()),
            record,
            previous: Arc::new(RejectingToolExecutor),
            previous_names: Default::default(),
            previous_qualified_names: Default::default(),
            delivery_attempted: Default::default(),
            delivery_sealed: Default::default(),
            delivery_lease: Default::default(),
            cancellation: Default::default(),
            #[cfg(target_os = "macos")]
            file_workspace: None,
        },
        _root,
        page,
        parent,
    };
    forbid_rewrite(&reopened);
    let output = reopened.tools.authorize_and_execute(request).await.unwrap();
    assert!(!output.is_error);
    assert_eq!(source(&reopened), later);
    let delivered = events.delivered.lock().unwrap();
    let receipts = delivered
        .iter()
        .filter(|(name, _)| name == "research-note-text-receipt")
        .collect::<Vec<_>>();
    assert_eq!(receipts.len(), 1);
    assert_eq!(receipts[0].1["pages"][0]["revision"], original["revision"]);
    assert_eq!(receipts[0].1["pages"][0]["document"], original["document"]);
}

#[tokio::test]
async fn legacy_completed_results_do_not_invent_new_delivery_proofs() {
    let (f, events) = with_events();
    read(&f, &path(f.page)).await;
    let request = call(&f, "legacy");
    f.tools
        .authorize_and_execute(request.clone())
        .await
        .unwrap();
    let call_key = key(&f, "legacy");
    let target = format!("{call_key}:notes");
    {
        let db = f.tools.store.connection().unwrap();
        db.execute("update agent_text_patch_calls set results_json=json_remove(results_json,'$.noteDelivery') where request_id=?1 or request_id=?2",params![call_key,target]).unwrap();
        assert!(
            note::delivery::read(&db, &f.tools.record, &target)
                .unwrap()
                .is_none()
        );
    }
    events.delivered.lock().unwrap().clear();
    let saved = source(&f);
    forbid_rewrite(&f);
    let output = f.tools.authorize_and_execute(request).await.unwrap();
    assert!(!output.is_error);
    assert_eq!(source(&f), saved);
    assert!(
        !events
            .delivered
            .lock()
            .unwrap()
            .iter()
            .any(|(name, _)| name == "research-note-text-receipt")
    );
}
