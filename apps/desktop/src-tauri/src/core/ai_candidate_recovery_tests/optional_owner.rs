//! A missing domain owner must not consume a recoverable accepted operation.
use crate::{
    agent::{repository::projection::read_turn_outcome, service::AgentSessionService},
    runtime_events::RuntimeEvents,
    storage::{AppStore, AppStorePaths, AppStorePorts, AppStoreSources},
};
use research_agent_core::conversation::ConversationId;
use research_agent_core::protocol::TurnOutcome;
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use std::path::Path;

#[cfg(test)]
#[path = "optional_owner/fixture.rs"]
mod fixture;
use fixture::pending_completion;
use uuid::Uuid;

fn open(directory: &Path, sources: AppStoreSources) -> AppStore {
    AppStore::open_with_paths(
        AppStorePaths {
            data_directory: directory.to_owned(),
            agent_configuration_directory: directory.join("configuration"),
        },
        AppStorePorts::with_events(RuntimeEvents::new(|_, _| Ok(())), sources),
    )
    .expect("production store startup")
}

fn state(store: &AppStore, run: &str) -> (String, Option<String>, Option<String>, String) {
    store
        .connection()
        .unwrap()
        .query_row(
            "select status,error_code,error_message,updated_at from ai_runs where id=?1",
            [run],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap()
}

fn assert_retained(store: &AppStore, conversation: ConversationId, turn: Uuid, recovered: bool) {
    let db = store.connection().unwrap();
    assert_eq!(
        read_turn_outcome(&db, conversation, turn).unwrap(),
        Some(TurnOutcome::Completed)
    );
    assert_eq!(
        db.query_row(
            "select materialized_position from data_runtime_state",
            [],
            |row| row.get::<_, u64>(0)
        )
        .unwrap(),
        2
    );
    let counts = db.query_row(
        "select (select count(*) from ai_candidates),
             (select count(*) from ai_candidate_revisions),
             (select count(*) from ai_candidate_recovery_audits),
             (select count(*) from paper_analysis_turn_attempts where turn_id='stranded-concept-turn')",
        [], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?, row.get::<_, i64>(2)?, row.get::<_, i64>(3)?)),
    ).unwrap();
    assert_eq!(counts, (1, 1, i64::from(recovered), 0));
    let workflow: String = db
        .query_row(
            "select status from paper_analysis_workflows where id='stranded-workflow'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        workflow,
        if recovered {
            "overview_ready"
        } else {
            "overview_running"
        }
    );
}

fn save_core_note(store: &AppStore, page: &str, operation: &str) -> Value {
    let mut db = store.connection().unwrap();
    let document = serde_json::from_value(json!({
        "schemaVersion":1,"pageId":page,"blocks":[{
            "id":"optional-owner-note","type":"paragraph","props":{},
            "content":{"kind":"inline","items":[{"type":"text","text":"initial","styles":[]}]},"children":[]
        }]
    })).unwrap();
    let initial = crate::notes::hbs::seed(&mut db, page, document).unwrap();
    let mut saved = initial["saved"].clone();
    saved["blocks"][0]["content"]["items"][0]["text"] = json!("Core note remains available");
    let result = crate::hbs_storage::commit(
        store.generation_policy_source(),
        store.paper_reading_source(),
        &mut db,
        "note",
        page,
        crate::hbs_storage::Commit {
            id: operation.into(),
            expected_version: initial["version"].as_i64().unwrap(),
            saved: saved.clone(),
            baseline: initial["baseline"].clone(),
        },
    )
    .unwrap();
    assert_eq!(result["snapshot"]["saved"], saved);
    assert_eq!(result["snapshot"]["baseline"], initial["baseline"]);
    result["snapshot"].clone()
}

async fn close(store: AppStore, service: AgentSessionService) {
    service.shutdown().await.unwrap();
    drop(service);
    store.shared_agent_connection().shutdown().unwrap();
    drop(store);
}

#[test]
fn missing_owner_preserves_pending_completion_and_real_service_recovery_until_restored() {
    for original_cause in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let (run, conversation, turn) = pending_completion(directory.path(), original_cause);
        let store = open(directory.path(), AppStoreSources::default());
        let service = AgentSessionService::from_app_store(&store).unwrap();
        let first = state(&store, &run);
        assert_eq!(first.0, "validating");
        assert_eq!(
            first.1.as_deref(),
            Some(if original_cause {
                "storage_error"
            } else {
                "capability_unavailable"
            })
        );
        if original_cause {
            assert_eq!(first.2.as_deref(), Some("original local failure"));
        }
        assert_retained(&store, conversation, turn, false);
        let page = Uuid::new_v4().to_string();
        let operation = Uuid::new_v4().to_string();
        let note = save_core_note(&store, &page, &operation);
        store
            .connection()
            .unwrap()
            .execute(
                "update ai_runs set updated_at='first-missing-owner-startup' where id=?1",
                [&run],
            )
            .unwrap();
        tauri::async_runtime::block_on(close(store, service));

        let store = open(directory.path(), AppStoreSources::default());
        let service = AgentSessionService::from_app_store(&store).unwrap();
        let second = state(&store, &run);
        assert_eq!(
            (&second.0, &second.1, &second.2),
            (&first.0, &first.1, &first.2)
        );
        assert_eq!(second.3, "first-missing-owner-startup");
        assert_retained(&store, conversation, turn, false);
        assert_eq!(
            crate::notes::hbs::read(&store.connection().unwrap(), &page).unwrap(),
            note
        );
        tauri::async_runtime::block_on(close(store, service));

        let (sources, native) = native_sources();
        let binding = sources.native_note_trace;
        for _ in 0..2 {
            let store = open(
                directory.path(),
                AppStoreSources {
                    native_note_trace: binding.clone(),
                    ..Default::default()
                },
            );
            let service = AgentSessionService::from_app_store(&store).unwrap();
            let recovered = state(&store, &run);
            assert_eq!(
                (recovered.0.as_str(), recovered.1, recovered.2),
                ("needs_review", None, None)
            );
            assert_retained(&store, conversation, turn, true);
            let db = store.connection().unwrap();
            assert_eq!(crate::notes::hbs::read(&db, &page).unwrap(), note);
            assert_eq!(db.query_row("select count(*) from hbs_commit_receipts where kind='note' and resource_id=?1 and request_id=?2", params![page,operation], |row| row.get::<_, i64>(0)).unwrap(), 1);
            drop(db);
            tauri::async_runtime::block_on(close(store, service));
        }
        tauri::async_runtime::block_on(native.shutdown()).unwrap();
    }
}

/// Reuse the explicitly selected compiled host and original recovery owner.
fn native_sources() -> (
    AppStoreSources,
    std::sync::Arc<crate::runtime_plugins::literature_trace::service::LiteratureTraceService>,
) {
    use crate::runtime_plugins::{
        RuntimePluginCatalog,
        literature_trace::{binding::NoteTraceBinding, service::LiteratureTraceService},
    };
    let (root, selected_key) =
        match std::env::var_os("RESEARCH_ASSISTANT_CONTEXT_METADATA_PACKAGE_ROOT") {
            Some(root) => (
                std::path::PathBuf::from(root),
                Some(
                    std::env::var("RESEARCH_ASSISTANT_CONTEXT_METADATA_PACKAGE_KEY")
                        .expect("exact native package key"),
                ),
            ),
            None => {
                let package = std::path::PathBuf::from(
                    std::env::var_os("RESEARCH_ASSISTANT_TEST_LITERATURE_PACKAGE")
                        .expect("explicit independent literature package"),
                );
                (package.parent().unwrap().to_owned(), None)
            }
        };
    let repository = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .unwrap();
    assert!(
        std::fs::symlink_metadata(&root)
            .unwrap()
            .file_type()
            .is_dir()
    );
    let root = root.canonicalize().unwrap();
    assert!(root.starts_with(repository.join(".tmp")) && root != repository.join(".tmp"));
    let catalog = std::sync::Arc::new(
        RuntimePluginCatalog::new(Some(root)).with_native_writing_preview(true),
    );
    let listing = catalog.discover().unwrap();
    let package = listing
        .packages
        .iter()
        .find(|package| {
            selected_key
                .as_ref()
                .map_or(package.manifest["id"] == "literature_library", |key| {
                    package.package_key == *key
                })
        })
        .expect("exact independent literature package");
    assert_eq!(package.manifest["id"], "literature_library");
    let key = package.package_key.clone();
    let entry = package.manifest["native"]["entry"].as_str().unwrap();
    assert!(
        !catalog
            .resource(&key, entry)
            .unwrap()
            .bytes
            .starts_with(b"#!"),
        "compiled production host required"
    );
    let service = std::sync::Arc::new(LiteratureTraceService::new(catalog));
    let sources = AppStoreSources {
        native_note_trace: Some(NoteTraceBinding {
            service: std::sync::Arc::clone(&service),
            package_key: key,
        }),
        ..Default::default()
    };
    (sources, service)
}

#[test]
#[ignore = "requires explicit independent native package root and exact key"]
fn actual_package_startup_recovers_replayed_candidate_before_interrupt_and_reopens() {
    for original_cause in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let (run, conversation, turn) = pending_completion(directory.path(), original_cause);
        let (sources, native) = native_sources();
        let binding = sources.native_note_trace.clone();
        let store = open(directory.path(), sources);
        assert!(store.paper_analysis_source().is_none());
        assert!(store.native_note_trace().is_some());
        assert_retained(&store, conversation, turn, true);
        let state = state(&store, &run);
        assert_eq!(
            (state.0.as_str(), state.1, state.2),
            ("needs_review", None, None)
        );
        {
            let db = store.connection().unwrap();
            let original: (String, Option<String>, Option<String>, String, i64) = db.query_row(
                "select original_run_status,original_run_error_code,original_run_error_message,original_attempt_status,recovery_version from ai_candidate_recovery_audits", [],
                |row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?)),
            ).unwrap();
            assert_eq!(original.0, "validating");
            assert_eq!(
                original.1.as_deref(),
                original_cause.then_some("storage_error")
            );
            assert_eq!(
                original.2.as_deref(),
                original_cause.then_some("original local failure")
            );
            assert_eq!((original.3.as_str(), original.4), ("running", 1));
        }
        let sessions = AgentSessionService::from_app_store(&store).unwrap();
        tauri::async_runtime::block_on(close(store, sessions));
        let store = open(
            directory.path(),
            AppStoreSources {
                native_note_trace: binding,
                ..Default::default()
            },
        );
        assert_retained(&store, conversation, turn, true);
        let sessions = AgentSessionService::from_app_store(&store).unwrap();
        tauri::async_runtime::block_on(close(store, sessions));
        tauri::async_runtime::block_on(native.shutdown()).unwrap();
    }
}

#[test]
#[ignore = "requires explicit independent native package root and exact key"]
fn actual_package_startup_interrupts_unrecoverable_attempt_without_replaying_model() {
    let directory = tempfile::tempdir().unwrap();
    let (run, conversation, turn) = pending_completion(directory.path(), true);
    {
        let db = Connection::open(directory.path().join("research-assistant.sqlite3")).unwrap();
        db.execute(
            "update ai_candidates set original_payload_hash=?1 where id='stranded-candidate'",
            ["c".repeat(64)],
        )
        .unwrap();
        db.execute(
            "insert into agent_paper_analysis_submissions
             (attempt_id,conversation_id,request_id,turn_id,context_snapshot_id,paper_id,created_at,candidate_id)
             select 'stranded-attempt',conversation_id,request_id,turn_id,
                    'stranded-snapshot','stranded-paper','now','stranded-candidate'
             from agent_submissions where conversation_id=?1 and turn_id=?2",
            params![conversation.as_uuid().to_string(), turn.to_string()],
        ).unwrap();
    }
    let (sources, native) = native_sources();
    let store = open(directory.path(), sources);
    {
        let db = store.connection().unwrap();
        assert_eq!(
            read_turn_outcome(&db, conversation, turn).unwrap(),
            Some(TurnOutcome::Completed)
        );
        let terminal:(String,String,String,String,String,i64)=db.query_row(
            "select r.status,r.error_code,w.status,t.status,a.status,(select count(*) from ai_candidate_recovery_audits) from ai_runs r join paper_analysis_workflows w on w.id='stranded-workflow' join paper_analysis_turns t on t.id='stranded-overview-turn' join paper_analysis_turn_attempts a on a.id='stranded-attempt' where r.id=?1",[run],
            |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?)),
        ).unwrap();
        assert_eq!(
            terminal,
            (
                "failed".into(),
                "interrupted".into(),
                "failed".into(),
                "interrupted".into(),
                "interrupted".into(),
                0
            )
        );
        assert_eq!(db.query_row("select terminal_status from agent_paper_analysis_submissions where attempt_id='stranded-attempt'",[],|r|r.get::<_,String>(0)).unwrap(),"interrupted");
        assert_eq!(
            db.query_row("select count(*) from ai_candidates", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            db.query_row(
                "select count(*) from paper_analysis_turn_attempts",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
    }
    store.shared_agent_connection().shutdown().unwrap();
    drop(store);
    tauri::async_runtime::block_on(native.shutdown()).unwrap();
}

#[test]
fn selected_missing_native_recovery_preserves_candidate_and_never_falls_back_to_static() {
    use crate::runtime_plugins::{
        RuntimePluginCatalog,
        literature_trace::{binding::NoteTraceBinding, service::LiteratureTraceService},
    };
    let directory = tempfile::tempdir().unwrap();
    let (run, _, _) = pending_completion(directory.path(), true);
    let packages = tempfile::tempdir().unwrap();
    let native = std::sync::Arc::new(LiteratureTraceService::new(std::sync::Arc::new(
        RuntimePluginCatalog::new(Some(packages.path().to_owned()))
            .with_native_writing_preview(true),
    )));
    let sources = AppStoreSources {
        native_note_trace: Some(NoteTraceBinding {
            service: std::sync::Arc::clone(&native),
            package_key: "0".repeat(64),
        }),
        ..crate::core_sources::sources()
    };
    let result = AppStore::open_with_paths(
        AppStorePaths {
            data_directory: directory.path().to_owned(),
            agent_configuration_directory: directory.path().join("configuration"),
        },
        AppStorePorts::with_events(RuntimeEvents::new(|_, _| Ok(())), sources),
    );
    let Err(crate::storage::StorageError::NativeStartup(error)) = result else {
        panic!("selected missing package must remain Native");
    };
    assert!(!error.code.is_empty());
    assert_eq!(error.package_key.as_deref(), Some("0".repeat(64).as_str()));
    let db = Connection::open(directory.path().join("research-assistant.sqlite3")).unwrap();
    let state:(String,String,i64)=db.query_row("select status,error_code,(select count(*) from ai_candidate_recovery_audits) from ai_runs where id=?1",[run],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert_eq!(state, ("validating".into(), "storage_error".into(), 0));
    tauri::async_runtime::block_on(native.shutdown()).unwrap();
}

#[test]
#[ignore = "requires explicit independent native package root and exact key"]
fn actual_package_startup_recovery_rolls_back_original_candidate_transaction_and_retries() {
    let directory = tempfile::tempdir().unwrap();
    let (run, conversation, turn) = pending_completion(directory.path(), true);
    {
        let db = Connection::open(directory.path().join("research-assistant.sqlite3")).unwrap();
        db.execute_batch("create trigger reject_recovery_audit before insert on ai_candidate_recovery_audits begin select raise(abort,'synthetic audit fault'); end;").unwrap();
    }
    let (sources, native) = native_sources();
    let binding = sources.native_note_trace.clone();
    let result = AppStore::open_with_paths(
        AppStorePaths {
            data_directory: directory.path().to_owned(),
            agent_configuration_directory: directory.path().join("configuration"),
        },
        AppStorePorts::with_events(RuntimeEvents::new(|_, _| Ok(())), sources),
    );
    assert!(matches!(
        result,
        Err(crate::storage::StorageError::Database(_))
    ));
    {
        let db = Connection::open(directory.path().join("research-assistant.sqlite3")).unwrap();
        let state:(String,String,String,String,i64)=db.query_row("select r.status,r.error_code,w.status,a.status,(select count(*) from ai_candidate_recovery_audits) from ai_runs r join paper_analysis_workflows w on w.id='stranded-workflow' join paper_analysis_turn_attempts a on a.id='stranded-attempt' where r.id=?1",[run],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).unwrap();
        assert_eq!(
            state,
            (
                "validating".into(),
                "storage_error".into(),
                "overview_running".into(),
                "running".into(),
                0
            )
        );
        assert_eq!(
            db.query_row("select count(*) from ai_candidates", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            1
        );
        db.execute_batch("drop trigger reject_recovery_audit;")
            .unwrap();
    }
    let store = open(
        directory.path(),
        AppStoreSources {
            native_note_trace: binding,
            ..Default::default()
        },
    );
    assert_retained(&store, conversation, turn, true);
    let sessions = AgentSessionService::from_app_store(&store).unwrap();
    tauri::async_runtime::block_on(close(store, sessions));
    tauri::async_runtime::block_on(native.shutdown()).unwrap();
}
