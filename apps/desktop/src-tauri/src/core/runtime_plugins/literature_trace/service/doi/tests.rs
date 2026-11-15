//! Completion mechanism tests with a real managed child, not Native policy tests.
#![cfg(unix)]
use super::*;
use crate::runtime_plugins::native::{NativeProfile, transport::NativeHost};
use research_workbench_native_sdk::{catalog::CatalogInput, data::DataValue};
use serde_json::Value;
use std::{sync::Arc, time::Duration};
use tokio::time::timeout;

mod fixture;
use fixture::Fixture;

const WAIT: Duration = Duration::from_secs(12);

#[tokio::test]
async fn managed_candidate_wait_releases_guard_for_actual_core_note_save() {
    let fixture = Fixture::new(false).await;
    let original_note = fixture.data.note();
    let task = fixture.start();
    let (_, input) = fixture.entered(1).await;
    assert!(!task.is_finished());
    let saved = fixture.save_note().await;
    assert_ne!(saved, original_note);
    assert!(!task.is_finished());
    assert_eq!(fixture.counts(), (0, 0, 0));
    let expected = fixture.release(1, &input, false);
    let preview = timeout(WAIT, task).await.unwrap().unwrap().unwrap();
    assert_eq!(preview, expected);
    assert_eq!(fixture.counts(), (1, 1, 1));
    assert_stored(&fixture.data.db, &preview, &fixture.result_id, "queued");
    timeout(WAIT, fixture.service.shutdown())
        .await
        .unwrap()
        .unwrap();
    fixture.assert_cleaned().await;
    assert_eq!(fixture.steps(), ["initialize", "candidate", "shutdown"]);
    let result_id = fixture.result_id.clone();
    let cold = fixture.cold();
    assert_eq!(cold.note(), saved);
    // Original migration recovery interrupts queued work; it does not replay it.
    assert_stored(&cold.db, &preview, &result_id, "interrupted");
}

#[tokio::test]
async fn changed_related_readset_reprepares_only_candidate_same_ids_and_writes_once() {
    let fixture = Fixture::new(false).await;
    let task = fixture.start();
    let (first_frame, first_input) = fixture.entered(1).await;
    let first_current = current(&first_input);
    assert_eq!(
        first_current
            .related
            .as_ref()
            .unwrap()
            .metadata_sources
            .rows,
        vec![vec![
            DataValue::Text("title".into()),
            DataValue::Text("doi.org:csl-json".into()),
            DataValue::Integer(0)
        ]]
    );
    let saved = fixture.save_note().await;
    // Change only columns in the actual related readset, leaving main unchanged.
    fixture
        .data
        .db
        .lock()
        .unwrap()
        .execute(
            "update metadata_provenance set source_kind='manual',confirmed_by_user=1
         where item_id='survivor' and field_name='title'",
            [],
        )
        .unwrap();
    let stale = fixture.release(1, &first_input, false);
    let (second_frame, second_input) = fixture.entered(2).await;
    assert!(!task.is_finished());
    assert_eq!(fixture.counts(), (0, 0, 0));
    assert_eq!(first_frame.instance_id, second_frame.instance_id);
    assert_ne!(first_frame.request_id, second_frame.request_id);
    let second_current = current(&second_input);
    assert_eq!(first_current.main, second_current.main);
    assert_eq!(
        first_current.related.as_ref().unwrap().creators,
        second_current.related.as_ref().unwrap().creators
    );
    assert_ne!(
        first_current.related.as_ref().unwrap().metadata_sources,
        second_current.related.as_ref().unwrap().metadata_sources
    );
    let mut first_echo = serde_json::to_value(&first_input).unwrap();
    let mut second_echo = serde_json::to_value(&second_input).unwrap();
    // All other candidate inputs, including ID/job/HTTP translation, are identical.
    first_echo["input"]["current"] = Value::Null;
    second_echo["input"]["current"] = Value::Null;
    assert_eq!(first_echo, second_echo);
    let expected = fixture.release(2, &second_input, false);
    assert_eq!(expected.id, stale.id);
    assert!(!stale.diffs[0].user_confirmed);
    assert!(expected.diffs[0].user_confirmed);
    let preview = timeout(WAIT, task).await.unwrap().unwrap().unwrap();
    assert_eq!(preview, expected);
    assert_eq!(fixture.counts(), (1, 1, 1));
    assert_eq!(fixture.data.note(), saved);
    assert_stored(&fixture.data.db, &preview, &fixture.result_id, "queued");
    timeout(WAIT, fixture.service.shutdown())
        .await
        .unwrap()
        .unwrap();
    fixture.assert_cleaned().await;
    // No Start/Translate/HTTP operation was dispatched by direct completion.
    assert_eq!(
        fixture.steps(),
        ["initialize", "candidate", "candidate", "shutdown"]
    );
    let result_id = fixture.result_id.clone();
    let cold = fixture.cold();
    assert_eq!(cold.note(), saved);
    assert_stored(&cold.db, &preview, &result_id, "interrupted");
    let confirmed: i64 = cold
        .db
        .lock()
        .unwrap()
        .query_row(
            "select confirmed_by_user from metadata_provenance
         where item_id='survivor' and field_name='title'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(confirmed, 1);
}

#[tokio::test]
async fn wrong_echo_writes_no_candidate_and_retires_actual_owned_host() {
    let fixture = Fixture::new(true).await;
    let note = fixture.data.note();
    let changes = fixture.data.db.lock().unwrap().total_changes();
    let task = fixture.start();
    let (request, input) = fixture.entered(1).await;
    fixture.release(1, &input, true);
    let error = timeout(WAIT, task).await.unwrap().unwrap().unwrap_err();
    assert_eq!(error.code, "plugin_host_response_mismatch");
    assert_eq!(error.package_key.as_deref(), Some(fixture.key.as_str()));
    assert_eq!(
        error.instance_id.as_deref(),
        Some(fixture.instance.as_str())
    );
    assert_eq!(
        error.request_id.as_deref(),
        Some(request.request_id.as_str())
    );
    assert_eq!(error.signal, Some(9));
    assert!(
        error.cleanup_error.is_none(),
        "safe cleanup diagnostic: {error:?}"
    );
    let progress = error.import_jobs.as_ref().unwrap();
    assert_eq!(progress.phase, "candidate-projection");
    assert_eq!(progress.header_committed, Some(true));
    assert_eq!(progress.job_id.as_deref(), Some(fixture.job_id.as_str()));
    assert_eq!(
        progress.result_id.as_deref(),
        Some(fixture.result_id.as_str())
    );
    assert!(progress.candidate_insert_returned.is_none());
    assert_eq!(fixture.counts(), (0, 0, 0));
    assert_eq!(fixture.data.db.lock().unwrap().total_changes(), changes);
    assert_eq!(fixture.data.note(), note);
    let diagnostic = serde_json::to_string(&error).unwrap();
    for private in [
        "wrong-candidate-echo",
        "Synthetic DOI candidate",
        "Preserved synthetic duplicate Core body",
        "{\"synthetic\":true}",
    ] {
        assert!(!diagnostic.contains(private));
    }
    fixture.assert_cleaned().await;
    timeout(WAIT, fixture.service.shutdown())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(fixture.steps(), ["initialize", "candidate"]);
    let cold = fixture.cold();
    assert_eq!(cold.note(), note);
    assert_eq!(
        cold.db
            .lock()
            .unwrap()
            .query_row("select count(*) from metadata_candidates", [], |row| row
                .get::<_, i64>(0),)
            .unwrap(),
        0
    );
}

fn current(input: &CatalogInput) -> &research_workbench_native_sdk::catalog::CatalogItemRows {
    let CatalogInput::DoiPreview { input } = input else {
        panic!("not DOI")
    };
    let DoiPreviewInputV1::Candidate { current, .. } = input.as_ref() else {
        panic!("not Candidate")
    };
    current.as_ref().unwrap()
}

fn assert_stored(
    database: &crate::data_interface::SharedDatabase,
    preview: &MetadataPreviewV1,
    result_id: &str,
    job_state: &str,
) {
    let db = database.lock().unwrap();
    let stored: (String, String, String, String, String) = db
        .query_row(
            "select item_id,source_kind,source_reference,draft_json,raw_payload
         from metadata_candidates where id=?1 and import_result_id=?2",
            rusqlite::params![preview.id, result_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(
        stored,
        (
            "survivor".into(),
            preview.source_kind.clone(),
            "10.1000/synthetic".into(),
            serde_json::to_string(&preview.draft).unwrap(),
            "{\"synthetic\":true}".into()
        )
    );
    let state: String = db
        .query_row(
            "select state from import_jobs where id=?1",
            [&preview.import_job_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(state, job_state);
    let result: (String, String, String) = db
        .query_row(
            "select state,input_label,raw_input from import_results where id=?1",
            [result_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        result,
        (
            "pending".into(),
            "10.1000/synthetic".into(),
            " Raw DOI input ".into()
        )
    );
    let writes: (i64, i64) = db
        .query_row(
            "select (select count(*) from synthetic_doi_calls where phase='candidate'),
         (select count(*) from synthetic_doi_calls where phase='queued')",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(writes, (1, 1));
}
