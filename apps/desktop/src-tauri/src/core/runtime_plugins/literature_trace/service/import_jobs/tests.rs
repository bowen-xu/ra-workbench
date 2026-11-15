#![cfg(unix)]
use super::*;
use crate::runtime_plugins::{
    literature_catalog::save::SaveOutcome, literature_duplicates::tests::fixture::PAGE,
};
use research_workbench_native_sdk::import_jobs::ImportJobsRequestV1;
use rusqlite::functions::FunctionFlags;
use serde_json::{Value, json};
use std::sync::{atomic::AtomicUsize, mpsc};
use tokio::time::timeout;

mod fixture;
use fixture::{Fixture, SAVED_ID};

const WAIT: Duration = Duration::from_secs(8);

#[tokio::test]
async fn held_real_prepare_allows_the_original_core_note_writer_before_batch_dispatch() {
    let fixture = Fixture::new(true).await;
    let service = Arc::clone(&fixture.service);
    let database = Arc::clone(&fixture.data.db);
    let key = fixture.key.clone();
    let draft = fixture.draft.clone();
    let client = tokio::spawn(async move {
        service
            .apply_import_jobs(
                database,
                key,
                ImportJobsRequestV1::Bibtex {
                    drafts: vec![draft],
                },
            )
            .await
    });
    fixture.prepare_seen().await;
    let database = Arc::clone(&fixture.data.db);
    let mut writer = tokio::task::spawn_blocking(move || {
        let mut db = database.lock().unwrap();
        let revision = db
            .query_row(
                "select revision from note_pages where id=?1",
                [PAGE],
                |row| row.get(0),
            )
            .unwrap();
        let tx = db.transaction().unwrap();
        let mut document = crate::notes::load_document_value(&tx, PAGE).unwrap();
        document.blocks[0].content = json!({"kind":"inline","items":[{
            "type":"text","text":"Core note saved during synthetic native preparation","styles":[]
        }]});
        crate::notes::save_document_in(
            &tx,
            crate::notes::NoteDocumentSaveRequest {
                document,
                expected_revision: revision,
                save_reason: "manual".into(),
            },
        )
        .unwrap();
        tx.commit().unwrap();
    });
    let access = timeout(Duration::from_secs(2), &mut writer).await;
    fixture.release_prepare();
    let was_available = match access {
        Ok(result) => {
            result.unwrap();
            true
        }
        Err(_) => {
            timeout(WAIT, writer).await.unwrap().unwrap();
            false
        }
    };
    let error = timeout(WAIT, client).await.unwrap().unwrap().unwrap_err();
    assert_eq!(error.code, "plugin_host_exited");
    assert_eq!(error.exit_code, Some(41));
    assert!(
        error.cleanup_error.is_none(),
        "safe cleanup diagnostic: {error:?}"
    );
    let progress = error.import_jobs.as_ref().unwrap();
    assert_eq!(progress.header_committed, Some(true));
    assert_eq!(progress.completed_result_updates, Some(1));
    assert!(progress.job_id.is_some());
    assert!(
        matches!(&progress.last_save, Some(SaveOutcome::Committed { item_id }) if item_id == SAVED_ID)
    );
    assert_eq!(error.package_key.as_deref(), Some(fixture.key.as_str()));
    assert!(error.instance_id.is_some() && error.request_id.is_some());
    assert!(fixture.service.host.lock().await.is_none());
    assert!(!fixture.directory.exists() && !fixture.executable.exists());
    timeout(WAIT, fixture.service.shutdown())
        .await
        .unwrap()
        .unwrap();
    assert!(
        was_available,
        "Core DB/admission guard crossed native preparation await"
    );
    assert!(
        fixture
            .data
            .note()
            .to_string()
            .contains("Core note saved during synthetic native preparation")
    );
    let _ = committed_state(&fixture);
    fixture.data.db.shutdown().unwrap();
}

#[tokio::test]
async fn aborted_wait_keeps_one_batch_and_shutdown_waits_for_sql_then_real_cold_reopen() {
    let fixture = Fixture::new(true).await;
    let note = fixture.data.note();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let attempts = Arc::new(AtomicUsize::new(0));
    {
        let db = fixture.data.db.lock().unwrap();
        let observed = Arc::clone(&attempts);
        db.create_scalar_function(
            "test_import_batch_barrier",
            0,
            FunctionFlags::SQLITE_UTF8,
            move |_| {
                observed.fetch_add(1, Ordering::SeqCst);
                entered_tx
                    .send(())
                    .map_err(|e| rusqlite::Error::UserFunctionError(Box::new(e)))?;
                release_rx
                    .recv()
                    .map_err(|e| rusqlite::Error::UserFunctionError(Box::new(e)))?;
                Ok(0_i64)
            },
        )
        .unwrap();
        db.execute_batch(
            "create temp trigger test_import_batch_pause before insert on bibliographic_items
            when new.id='service-import-job-item' begin select test_import_batch_barrier(); end;",
        )
        .unwrap();
    }
    let service = Arc::clone(&fixture.service);
    let database = Arc::clone(&fixture.data.db);
    let key = fixture.key.clone();
    let draft = fixture.draft.clone();
    let client = tokio::spawn(async move {
        service
            .apply_import_jobs(
                database,
                key,
                ImportJobsRequestV1::Bibtex {
                    drafts: vec![draft],
                },
            )
            .await
    });
    fixture.prepare_seen().await;
    fixture.release_prepare();
    timeout(
        WAIT,
        tokio::task::spawn_blocking(move || entered_rx.recv_timeout(WAIT)),
    )
    .await
    .unwrap()
    .unwrap()
    .unwrap();
    client.abort();
    let cancelled = client.await;
    let retained_owner = fixture.service.host.try_lock().is_err();
    let service = Arc::clone(&fixture.service);
    let mut shutdown = tokio::spawn(async move { service.shutdown().await });
    timeout(WAIT, async {
        while !fixture.service.stopping.load(Ordering::Acquire) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    let probe = timeout(Duration::from_millis(100), &mut shutdown).await;
    release_tx.send(()).unwrap();
    let (waited, result) = match probe {
        Err(_) => (true, timeout(WAIT, shutdown).await.unwrap()),
        Ok(result) => (false, result),
    };
    assert!(cancelled.is_err_and(|e| e.is_cancelled()));
    assert!(
        retained_owner,
        "client abort released the accepted host/SQL owner"
    );
    assert!(
        waited,
        "shutdown completed before actual batch SQL was released"
    );
    result.unwrap().unwrap();
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
    assert!(fixture.service.host.lock().await.is_none());
    assert!(!fixture.directory.exists() && !fixture.executable.exists());
    assert_eq!(fixture.data.note(), note);
    let state = committed_state(&fixture);
    let Fixture { data, service, .. } = fixture;
    drop(service);
    let reopened = data.reopen();
    assert_eq!(reopened.note(), note);
    assert_eq!(sql_state(&reopened.db.lock().unwrap()), state);
    reopened.db.shutdown().unwrap();
}

#[tokio::test]
async fn cancel_zero_not_found_is_domain_but_nonzero_false_not_found_retires_real_host() {
    for (id, exists, expected) in [
        ("missing-job", false, "not_found"),
        ("cancel-target", true, "plugin_host_response_mismatch"),
    ] {
        let fixture = Fixture::new(false).await;
        let note = fixture.data.note();
        if exists {
            fixture
                .data
                .db
                .lock()
                .unwrap()
                .execute(
                    "insert into import_jobs
                (id,source_kind,state,total_count,created_at,updated_at)
                values(?1,'doi','running',1,'raw-created','raw-updated')",
                    [id],
                )
                .unwrap();
        }
        let error = timeout(
            WAIT,
            fixture.service.apply_import_jobs(
                Arc::clone(&fixture.data.db),
                fixture.key.clone(),
                ImportJobsRequestV1::Cancel { id: id.into() },
            ),
        )
        .await
        .unwrap()
        .unwrap_err();
        assert_eq!(error.code, expected);
        assert_eq!(error.package_key.as_deref(), Some(fixture.key.as_str()));
        assert!(error.instance_id.is_some() && error.request_id.is_some());
        assert!(
            error.cleanup_error.is_none(),
            "safe cleanup diagnostic: {error:?}"
        );
        assert_eq!(
            error.import_jobs.as_ref().unwrap().job_id.as_deref(),
            Some(id)
        );
        if exists {
            assert!(fixture.service.host.lock().await.is_none());
            assert!(!fixture.directory.exists() && !fixture.executable.exists());
            assert!(
                fixture
                    .data
                    .db
                    .lock()
                    .unwrap()
                    .query_row(
                        "select cancellation_requested from import_jobs where id=?1",
                        [id],
                        |row| row.get::<_, bool>(0),
                    )
                    .unwrap()
            );
        } else {
            assert!(
                fixture.service.host.lock().await.is_some(),
                "valid domain NotFound retired a healthy host"
            );
        }
        assert_eq!(fixture.data.note(), note);
        timeout(WAIT, fixture.service.shutdown())
            .await
            .unwrap()
            .unwrap();
        fixture.data.db.shutdown().unwrap();
    }
}

fn committed_state(fixture: &Fixture) -> Value {
    let db = fixture.data.db.lock().unwrap();
    let state = sql_state(&db);
    assert_eq!(state["jobs"][0][1], "completed");
    assert_eq!(state["jobs"][0][2], "bibtex");
    assert_eq!(state["jobs"][0][3], 1);
    assert_eq!(state["jobs"][0][4], 1);
    assert_eq!(state["results"].as_array().unwrap().len(), 1);
    assert_eq!(state["results"][0][4], "imported");
    assert_eq!(state["results"][0][5], SAVED_ID);
    assert_eq!(state["items"].as_array().unwrap().len(), 1);
    assert_eq!(state["items"][0][0], SAVED_ID);
    assert_eq!(state["fts"], 1);
    let raw = state["results"][0][3].as_str().unwrap();
    let raw: Value = serde_json::from_str(raw).unwrap();
    assert_eq!(raw["sourceKind"], "original-raw-source");
    state
}

fn sql_state(db: &rusqlite::Connection) -> Value {
    let jobs = db
        .prepare(
            "select id,state,source_kind,total_count,completed_count,
        cancellation_requested,created_at,updated_at from import_jobs order by rowid",
        )
        .unwrap()
        .query_map([], |r| {
            Ok(json!([
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, bool>(5)?,
                r.get::<_, String>(6)?,
                r.get::<_, String>(7)?
            ]))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let results = db
        .prepare(
            "select id,job_id,input_label,raw_input,state,item_id,error_message,
        created_at,updated_at from import_results order by rowid",
        )
        .unwrap()
        .query_map([], |r| {
            Ok(json!([
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, Option<String>>(5)?,
                r.get::<_, Option<String>>(6)?,
                r.get::<_, String>(7)?,
                r.get::<_, String>(8)?
            ]))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let items = db
        .prepare(
            "select id,item_type,title,created_at,updated_at from bibliographic_items
        where id='service-import-job-item'",
        )
        .unwrap()
        .query_map([], |r| {
            Ok(json!([
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?
            ]))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let fts: i64 = db
        .query_row(
            "select count(*) from literature_search where item_id=?1",
            [SAVED_ID],
            |r| r.get(0),
        )
        .unwrap();
    json!({"jobs":jobs,"results":results,"items":items,"fts":fts})
}
