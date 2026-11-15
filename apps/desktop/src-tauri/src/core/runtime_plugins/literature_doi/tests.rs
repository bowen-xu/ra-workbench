//! Actual Core SQL/log tests, with canned Native preview DTOs rather than a host.
use super::*;
use crate::runtime_plugins::{
    literature_catalog::data::{CatalogResources, capture_items},
    literature_import_jobs::{ImportJobsProgress, ImportJobsResources, SqlOutcome},
};
use research_workbench_native_sdk::{
    doi_metadata::MetadataPreviewV1, import_jobs::ImportFinishStatesV1,
};
use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
use serde_json::{Value, json};

mod fixture;
use fixture::Fixture;

fn states() -> ImportFinishStatesV1 {
    ImportFinishStatesV1 {
        no_failed: "completed".into(),
        no_imported: "failed".into(),
        mixed: "completed_with_errors".into(),
    }
}

#[test]
fn exact_resources_preserve_raw_header_candidate_and_related_rows_after_cold_reopen() {
    let f = Fixture::new();
    let note = f.data.note();
    let before = capture_items(&f.data.db.lock().unwrap(), &f.reads, &["survivor".into()]).unwrap();
    let (job, result) = f.create();
    let preview = f.preview(&job);
    assert!(f.data.db.lock().unwrap().is_autocommit());
    assert_eq!(
        f.scalar::<String>("select raw_input from import_results"),
        " Raw DOI input "
    );
    assert_eq!(
        f.scalar::<String>("select input_label from import_results"),
        "10.1000/normalized"
    );
    storage::insert_candidate(
        &f.data.db.lock().unwrap(),
        &f.doi,
        &preview,
        " raw audit payload ",
        &result,
    )
    .unwrap();
    assert_eq!(
        f.scalar::<String>("select state from import_jobs"),
        "queued"
    );
    assert_eq!(
        f.scalar::<String>("select state from import_results"),
        "pending"
    );
    assert_eq!(
        f.scalar::<String>("select raw_payload from metadata_candidates"),
        " raw audit payload "
    );
    assert_eq!(
        serde_json::from_str::<Value>(
            &f.scalar::<String>("select draft_json from metadata_candidates")
        )
        .unwrap(),
        serde_json::to_value(&preview.draft).unwrap()
    );
    assert_eq!(
        f.scalar::<String>("select import_result_id from metadata_candidates"),
        result
    );
    assert_eq!(f.scalar::<String>("select job_id from import_results"), job);
    assert_eq!(
        capture_items(&f.data.db.lock().unwrap(), &f.reads, &["survivor".into()]).unwrap(),
        before
    );
    assert_eq!(f.data.note(), note);
    let after = f.state();
    f.assert_cold(&after);
}

#[test]
fn shared_header_nested_begin_preserves_the_callers_transaction() {
    let f = Fixture::new();
    let before = f.state();
    let mut db = f.data.db.lock().unwrap();
    db.execute_batch("BEGIN").unwrap();
    let error = storage::create_job(&mut db, &f.jobs, "doi", "10.1000/x", "raw").unwrap_err();
    assert_eq!(error.code, "storage_error");
    assert!(!db.is_autocommit());
    assert!(matches!(
        error.import_jobs.as_ref().unwrap().outcome,
        SqlOutcome::NotStarted
    ));
    db.execute_batch("ROLLBACK").unwrap();
    drop(db);
    assert_eq!(f.state(), before);
    f.assert_cold(&before);
}

#[test]
fn shared_header_late_result_failure_rolls_back_both_inserts() {
    let f = Fixture::new();
    let before = f.state();
    f.sql("create temp trigger reject_doi_pending before insert on import_results begin select raise(abort,'synthetic pending rejection'); end;");
    let error = storage::create_job(
        &mut f.data.db.lock().unwrap(),
        &f.jobs,
        "doi",
        "10.1000/x",
        "raw",
    )
    .unwrap_err();
    let progress = error.import_jobs.as_ref().unwrap();
    assert_eq!(progress.header_committed, Some(false));
    assert!(matches!(progress.outcome, SqlOutcome::RolledBack));
    assert_eq!(
        error.data_error.as_ref().unwrap().sqlite_extended_code,
        Some(1811)
    );
    assert_eq!(f.state(), before);
    f.assert_cold(&before);
}

#[test]
fn failed_result_does_not_touch_item_id_or_fire_its_update_trigger() {
    let f = Fixture::new();
    let (job, result) = f.create();
    f.sql("update import_results set item_id='survivor'; create temp trigger reject_item_id before update of item_id on import_results begin select raise(abort,'item_id must not be updated'); end;");
    storage::fail_job(
        &f.data.db.lock().unwrap(),
        &f.jobs,
        &f.doi,
        &job,
        &result,
        "original fetch failure",
        &states(),
    )
    .unwrap();
    assert_eq!(
        f.scalar::<String>("select item_id from import_results"),
        "survivor"
    );
    assert_eq!(
        f.scalar::<String>("select error_message from import_results"),
        "original fetch failure"
    );
    assert_eq!(
        f.scalar::<String>("select state from import_jobs"),
        "failed"
    );
    assert_eq!(
        f.scalar::<i64>("select completed_count from import_jobs"),
        1
    );
    let after = f.state();
    f.assert_cold(&after);
}

#[test]
fn failed_result_statement_keeps_fail_side_effect_and_original_safe_error() {
    let f = Fixture::new();
    let (job, result) = f.create();
    let note = f.data.note();
    f.sql("create temp trigger reject_doi_failed before update of state on import_results when new.state='failed' begin insert into synthetic_doi_observations(phase) values('before failed result'); select raise(fail,'synthetic failed result'); end;");
    let error = storage::fail_job(
        &f.data.db.lock().unwrap(),
        &f.jobs,
        &f.doi,
        &job,
        &result,
        "fetch error must not win",
        &states(),
    )
    .unwrap_err();
    assert_eq!(error.code, "storage_error");
    assert_eq!(
        error.data_error.as_ref().unwrap().sqlite_extended_code,
        Some(1811)
    );
    let p = error.import_jobs.as_ref().unwrap();
    assert_eq!(p.phase, "result-failed");
    assert_eq!(p.completed_result_updates, Some(0));
    assert_eq!(p.header_committed, Some(true));
    assert!(matches!(p.outcome, SqlOutcome::StatementFailed));
    assert_eq!(
        f.scalar::<i64>("select count(*) from synthetic_doi_observations"),
        1
    );
    assert_eq!(
        f.scalar::<String>("select state from import_results"),
        "pending"
    );
    assert_eq!(
        f.scalar::<String>("select state from import_jobs"),
        "running"
    );
    assert_eq!(f.data.note(), note);
    let after = f.state();
    f.assert_cold(&after);
}

#[test]
fn aggregate_or_finish_errors_keep_the_failed_result_prefix() {
    for count_failure in [true, false] {
        let f = Fixture::new();
        let (job, result) = f.create();
        if count_failure {
            f.data
                .db
                .lock()
                .unwrap()
                .authorizer(Some(|c: AuthContext<'_>| match c.action {
                    AuthAction::Read {
                        table_name: "import_results",
                        column_name: "state",
                    } => Authorization::Deny,
                    _ => Authorization::Allow,
                }));
        } else {
            f.sql("create temp trigger reject_doi_finish before update of state on import_jobs when new.state='failed' begin select raise(abort,'synthetic finish failure'); end;");
        }
        let error = storage::fail_job(
            &f.data.db.lock().unwrap(),
            &f.jobs,
            &f.doi,
            &job,
            &result,
            "original fetch failure",
            &states(),
        )
        .unwrap_err();
        f.data
            .db
            .lock()
            .unwrap()
            .authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
        let p = error.import_jobs.as_ref().unwrap();
        assert_eq!(
            p.phase,
            if count_failure {
                "aggregate"
            } else {
                "finish-update"
            }
        );
        assert_eq!(p.completed_result_updates, Some(1));
        assert_eq!(
            f.scalar::<String>("select error_message from import_results"),
            "original fetch failure"
        );
        assert_eq!(
            f.scalar::<String>("select state from import_results"),
            "failed"
        );
        assert_eq!(
            f.scalar::<String>("select state from import_jobs"),
            "running"
        );
        let after = f.state();
        f.assert_cold(&after);
    }
}

#[test]
fn candidate_fail_is_not_reported_as_a_known_rollback_or_successful_insert() {
    let f = Fixture::new();
    let (job, result) = f.create();
    let preview = f.preview(&job);
    f.sql("create temp trigger reject_doi_candidate before insert on metadata_candidates begin insert into synthetic_doi_observations(phase) values('before candidate'); select raise(fail,'synthetic candidate failure'); end;");
    let error = storage::insert_candidate(
        &f.data.db.lock().unwrap(),
        &f.doi,
        &preview,
        "audit",
        &result,
    )
    .unwrap_err();
    let p = error.import_jobs.as_ref().unwrap();
    assert_eq!(p.phase, "candidate-insert");
    assert_eq!(p.candidate_id.as_deref(), Some(preview.id.as_str()));
    assert_eq!(p.candidate_insert_returned, None);
    assert!(matches!(p.outcome, SqlOutcome::StatementFailed));
    assert_eq!(
        f.scalar::<i64>("select count(*) from synthetic_doi_observations"),
        1
    );
    assert_eq!(
        f.scalar::<i64>("select count(*) from metadata_candidates"),
        0
    );
    assert_eq!(
        f.scalar::<String>("select state from import_jobs"),
        "running"
    );
    let after = f.state();
    f.assert_cold(&after);
}

#[test]
fn queued_update_failure_keeps_the_committed_candidate_and_exact_identity() {
    let f = Fixture::new();
    let (job, result) = f.create();
    let preview = f.preview(&job);
    let note = f.data.note();
    f.sql("create temp trigger reject_doi_queue before update of state on import_jobs when new.state='queued' begin select raise(abort,'synthetic queue failure'); end;");
    let error = storage::insert_candidate(
        &f.data.db.lock().unwrap(),
        &f.doi,
        &preview,
        "audit untouched",
        &result,
    )
    .unwrap_err();
    let p = error.import_jobs.as_ref().unwrap();
    assert_eq!(p.phase, "queued-update");
    assert_eq!(p.candidate_id.as_deref(), Some(preview.id.as_str()));
    assert_eq!(p.candidate_insert_returned, Some(true));
    assert_eq!(p.header_committed, Some(true));
    assert!(matches!(p.outcome, SqlOutcome::StatementFailed));
    assert_eq!(
        f.scalar::<String>("select id from metadata_candidates"),
        preview.id
    );
    assert_eq!(
        f.scalar::<String>("select raw_payload from metadata_candidates"),
        "audit untouched"
    );
    assert_eq!(
        f.scalar::<String>("select state from import_jobs"),
        "running"
    );
    assert_eq!(f.data.note(), note);
    let after = f.state();
    f.assert_cold(&after);
}

#[test]
fn existing_import_job_json_does_not_gain_doi_fields_by_default_or_on_unknown_join() {
    for progress in [
        ImportJobsProgress::new("header"),
        ImportJobsProgress::unknown("worker-join"),
    ] {
        let value = serde_json::to_value(progress).unwrap();
        assert!(value.get("candidateId").is_none());
        assert!(value.get("candidateInsertReturned").is_none());
    }
}
