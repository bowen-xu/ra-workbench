use super::*;
use crate::agent::{compaction_hook_commands::HookConsentService, repository};
use research_agent_core::{
    context::{checkpoint::ContextCheckpoint, compaction_strategy::*},
    conversation::ConversationId,
    protocol::{AgentEvent, EventPayload, TaskKind},
    submission::SubmissionRecord,
};
use rusqlite::Connection;

pub(crate) fn fixture() -> (
    tempfile::TempDir,
    AppStore,
    SubmissionRecord,
    CompactionHookRequest,
    std::path::PathBuf,
) {
    let (root, old, record) = crate::agent::host_execution::tests::fixture("note");
    let path = root.path().canonicalize().unwrap().join("spill.sqlite");
    old.connection()
        .unwrap()
        .execute("vacuum main into ?1", [path.to_str().unwrap()])
        .unwrap();
    let content = old.content_directory().to_path_buf();
    drop(old);
    std::fs::create_dir_all(&content).unwrap();
    let mut db = Connection::open(&path).unwrap();
    crate::storage::migrate(&mut db).unwrap();
    let event = AgentEvent::new(
        ConversationId::from_uuid(record.conversation_id),
        1,
        1,
        record.request_id,
        EventPayload::TurnStarted {
            turn_id: record.turn_id,
            task_kind: TaskKind::Regular,
            context_snapshot_id: record.context_snapshot_id,
            model: record.input.model.clone(),
        },
    )
    .unwrap();
    repository::append_batch(
        &mut db,
        event.conversation_id(),
        record.request_id,
        0,
        &[event],
    )
    .unwrap();
    let checkpoint = ContextCheckpoint {
        schema_version: 1,
        window_id: Uuid::new_v4(),
        conversation_id: record.conversation_id,
        turn_id: record.turn_id,
        source_watermark: 1,
        previous_window_id: None,
        binding: CompactionBinding {
            model: record.input.model.clone(),
            strategy_id: "fixture.text".into(),
            state_schema: 1,
            kind: CompactionKind::TextSummary,
        },
        credential_scope_id: None,
        phase: CompactionPhase::Manual,
        output: CompactionOutput::TextSummary("summary".into()),
        retained: vec![],
        canonical_context: None,
    };
    repository::context_windows::install(&mut db, &checkpoint).unwrap();
    let request = CompactionHookRequest {
        operation_id: checkpoint.window_id,
        conversation_id: record.conversation_id,
        turn_id: record.turn_id,
        stage: CompactionHookStage::SessionStartCompact,
        phase: CompactionPhase::Manual,
        model: record.input.model.clone(),
    };
    repository::compaction_hook_audit::record(
        &db,
        &request,
        CompactionHookNotification::Started,
        1,
    )
    .unwrap();
    (root, AppStore::for_test(db, content), record, request, path)
}
struct Inner;
impl CompactionHooks for Inner {
    fn notify(&self, _: &CompactionHookRequest, _: CompactionHookNotification) {}
    fn run(&self, _: CompactionHookRequest) -> CompactionHookFuture<'_> {
        Box::pin(async { CompactionHookOutcome::Continue })
    }
}
pub(crate) async fn prepare(
    store: &AppStore,
    request: &CompactionHookRequest,
    text: &str,
) -> (Uuid, CompactionHookResult) {
    let settings = HookConsentService::from_app_store(store);
    let hooks = SpillingCompactionHooks::new(
        Arc::new(Inner),
        settings.output_store(),
        request.model.clone(),
        512,
    )
    .unwrap();
    let output = hooks
        .prepare_context(
            request.clone(),
            CompactionHookResult {
                outcome: CompactionHookOutcome::Continue,
                additional_contexts: vec![text.into()],
            },
        )
        .await
        .unwrap();
    let locator = output.additional_contexts[0]
        .lines()
        .last()
        .unwrap()
        .strip_prefix("Full hook output saved to: ")
        .unwrap();
    (parse_locator(locator).unwrap(), output)
}
fn read_request(
    record: &SubmissionRecord,
    id: Uuid,
    offset: usize,
    limit: usize,
) -> HookArtifactRead {
    HookArtifactRead {
        submission: Arc::new(record.clone()),
        through: 1,
        artifact_id: id,
        offset,
        limit,
    }
}
fn allow() -> Arc<HookArtifactAuthority> {
    Arc::new(|_| Ok(()))
}
pub(crate) fn complete(
    store: &AppStore,
    request: &CompactionHookRequest,
    output: &CompactionHookResult,
) {
    repository::compaction_hook_audit::context::record_result(
        &mut store.connection().unwrap(),
        request,
        output,
        2,
    )
    .unwrap();
}

#[tokio::test]
async fn native_output_is_durable_before_preview_and_only_completed_context_can_read_it() {
    let (_root, store, record, request, path) = fixture();
    let text = "完整证据🙂".repeat(1000);
    let (id, output) = prepare(&store, &request, &text).await;
    assert!(output.additional_contexts[0].len() <= 512);
    assert!(!output.additional_contexts[0].contains(store.content_directory().to_str().unwrap()));
    let settings = HookConsentService::from_app_store(&store);
    assert!(
        settings
            .output_reader()
            .read(read_request(&record, id, 0, 64), allow())
            .await
            .is_err()
    );
    let (sha,relative):(String,String)=store.connection().unwrap().query_row("select c.sha256,c.relative_path from content_objects c join agent_hook_spill_artifacts a on a.content_sha256=c.sha256 where a.id=?1",[id.to_string()],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(
        std::fs::read_to_string(store.content_directory().join(relative)).unwrap(),
        text
    );
    complete(&store, &request, &output);
    let content = store.content_directory().to_path_buf();
    drop(settings);
    drop(store);
    let store = AppStore::for_test(Connection::open(path).unwrap(), content);
    let reader = HookConsentService::from_app_store(&store).output_reader();
    let mut offset = 0;
    let mut collected = String::new();
    loop {
        let slice = reader
            .read(read_request(&record, id, offset, 4096), allow())
            .await
            .unwrap();
        assert_eq!(slice.sha256, sha);
        assert_eq!(slice.total_bytes, text.len());
        assert!(slice.text.len() <= 4096);
        collected.push_str(&slice.text);
        match slice.next_offset {
            Some(next) => {
                assert!(next > offset);
                offset = next;
            }
            None => break,
        }
    }
    assert_eq!(collected, text);
    assert_eq!(
        store
            .connection()
            .unwrap()
            .query_row("select count(*) from agent_object_staging", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn native_reader_denies_wrong_models_cutoffs_offsets_and_revoked_host_permissions() {
    let (_root, store, record, request, _) = fixture();
    let (id, output) = prepare(&store, &request, &"🙂证据".repeat(1000)).await;
    complete(&store, &request, &output);
    let reader = HookConsentService::from_app_store(&store).output_reader();
    assert!(
        reader
            .read(
                read_request(&record, id, 0, 64),
                Arc::new(|_| Err(HookContextError))
            )
            .await
            .is_err()
    );
    for (offset, limit) in [(1, 64), (usize::MAX, 64), (0, 3), (0, 65537)] {
        assert!(
            reader
                .read(read_request(&record, id, offset, limit), allow())
                .await
                .is_err()
        );
    }
    let mut wrong = record.clone();
    wrong.input.model.revision = Some("wrong".into());
    assert!(
        reader
            .read(read_request(&wrong, id, 0, 64), allow())
            .await
            .is_err()
    );
    let mut wrong = read_request(&record, id, 0, 64);
    wrong.through = 0;
    assert!(reader.read(wrong, allow()).await.is_err());
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let authority: Arc<HookArtifactAuthority> = Arc::new(move |_| {
        if calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
            Ok(())
        } else {
            Err(HookContextError)
        }
    });
    assert!(
        reader
            .read(read_request(&record, id, 0, 64), authority)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn native_reader_rejects_corrupted_full_body_even_when_requested_prefix_is_unchanged() {
    let (_root, store, record, request, _) = fixture();
    let text = "evidence".repeat(1000);
    let (id, output) = prepare(&store, &request, &text).await;
    complete(&store, &request, &output);
    let relative:String=store.connection().unwrap().query_row("select c.relative_path from content_objects c join agent_hook_spill_artifacts a on a.content_sha256=c.sha256 where a.id=?1",[id.to_string()],|r|r.get(0)).unwrap();
    let mut changed = text.into_bytes();
    *changed.last_mut().unwrap() = b'!';
    std::fs::write(store.content_directory().join(relative), changed).unwrap();
    let reader = HookConsentService::from_app_store(&store).output_reader();
    assert!(
        reader
            .read(read_request(&record, id, 0, 64), allow())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn failed_native_receipt_never_publishes_preview_and_object_remains_recoverable() {
    let (_root, store, _record, request, _) = fixture();
    store.connection().unwrap().execute_batch("create trigger reject_receipt before insert on agent_hook_spill_artifacts begin select raise(abort,'fault'); end;").unwrap();
    let settings = HookConsentService::from_app_store(&store);
    let text = "evidence".repeat(1000);
    assert!(
        settings
            .output_store()
            .save(request.clone(), text.clone())
            .await
            .is_err()
    );
    {
        let db = store.connection().unwrap();
        assert_eq!(
            db.query_row("select count(*) from agent_hook_spill_artifacts", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            db.query_row(
                "select count(*) from content_object_gc_candidates",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
        db.execute_batch("drop trigger reject_receipt").unwrap();
    }
    let (id, output) = prepare(&store, &request, &text).await;
    complete(&store, &request, &output);
    assert!(!id.is_nil());
}

#[tokio::test]
async fn invalid_native_request_cannot_stage_objects_or_receipts() {
    let (_root, store, _record, request, _) = fixture();
    let writer = HookConsentService::from_app_store(&store).output_store();
    let mut wrong = request.clone();
    wrong.operation_id = Uuid::new_v4();
    assert!(writer.save(wrong, "evidence".repeat(1000)).await.is_err());
    let mut wrong = request;
    wrong.stage = CompactionHookStage::PostCompact;
    assert!(writer.save(wrong, "evidence".repeat(1000)).await.is_err());
    let db = store.connection().unwrap();
    for table in [
        "content_objects",
        "agent_object_staging",
        "agent_hook_spill_artifacts",
    ] {
        assert_eq!(
            db.query_row(&format!("select count(*) from {table}"), [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}

#[tokio::test]
async fn interrupted_object_staging_uses_shared_recovery_without_inventing_receipt_or_access() {
    let (_root, store, _record, request, _) = fixture();
    let text = "recoverable evidence".repeat(1000);
    let version = file_version(text.as_bytes(), false);
    store.connection().unwrap().execute_batch("create trigger reject_registration before insert on content_objects begin select raise(abort,'fault'); end;").unwrap();
    let writer = HookConsentService::from_app_store(&store).output_store();
    assert!(writer.save(request, text).await.is_err());
    {
        let db = store.connection().unwrap();
        assert_eq!(
            db.query_row("select count(*) from agent_object_staging", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            db.query_row("select count(*) from agent_hook_spill_artifacts", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        db.execute_batch("drop trigger reject_registration")
            .unwrap();
    }
    assert_eq!(object_staging::recover(&store).unwrap(), (1, 0));
    let db = store.connection().unwrap();
    assert_eq!(
        db.query_row("select count(*) from agent_object_staging", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        db.query_row("select sha256 from content_objects", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        version.sha256
    );
    assert_eq!(
        db.query_row("select count(*) from agent_hook_spill_grants", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}
