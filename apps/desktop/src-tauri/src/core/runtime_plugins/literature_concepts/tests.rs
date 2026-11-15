//! Actual Core adapter tests; canned Native replies are not real-host evidence.
use super::*;
use research_workbench_native_sdk::concepts::{Concept, ConceptPreparationV1, ConceptRejectionV1};
use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

mod fixture;
mod ordering;
mod transactions;
use fixture::{Fixture, draft, parts, plan, search};

#[test]
fn actual_create_update_revisions_terms_and_disabled_embedding_survive_cold_reopen() {
    let f = Fixture::new();
    let note = f.note();
    let first = f.save(plan(draft())).unwrap();
    assert_eq!(first.revision, 1);
    assert_eq!(
        first.terms.as_slice(),
        std::slice::from_ref(&first.preferred_term)
    );
    assert_eq!(first.provenance_kind, "human_original");
    let mut update = draft();
    update.id = Some(first.id.clone());
    update.expected_revision = Some(first.revision);
    update.label = "Synthetic revised concept".into();
    update.preferred_term = "Synthetic revised term".into();
    let mut prepared = plan(update);
    let (term, rules) = parts(&mut prepared);
    term.normalized_text = "synthetic revised term".into();
    rules.concept_reason = "manual_edit".into();
    let actual = f.save(prepared).unwrap();
    assert_eq!(actual.id, first.id);
    assert_eq!(actual.revision, 2);
    assert_eq!(actual.preferred_term.text, "Synthetic revised term");
    assert_eq!(actual.terms.len(), 2);
    assert!(
        actual
            .terms
            .iter()
            .any(|term| term.id == first.preferred_term.id)
    );
    assert_eq!(f.count("terms"), 2);
    assert_eq!(f.count("research_object_revisions"), 4);
    assert_eq!(f.count("semantic_representations"), 2);
    assert_eq!(f.count("embedding_records"), 0);
    let db = f.db.lock().unwrap();
    let snapshots = db.prepare("select snapshot_json from research_object_revisions where object_kind='concept' order by revision").unwrap().query_map([], |r| r.get::<_, String>(0)).unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap();
    assert_eq!(
        serde_json::from_str::<Concept>(&snapshots[0]).unwrap(),
        first
    );
    assert_eq!(
        serde_json::from_str::<Concept>(&snapshots[1]).unwrap(),
        actual
    );
    drop(db);
    let before = f.state();
    assert_eq!(f.note(), note);
    let f = f.reopen();
    assert_eq!(f.state(), before);
    assert_eq!(f.note(), note);
    let ConceptResult::Concepts(rows) = f.apply(search("", "", "%%")).unwrap() else {
        panic!("list")
    };
    assert_eq!(rows, [actual]);
}

#[test]
fn search_retains_like_wildcards_and_decodes_every_row_instead_of_returning_drafts() {
    let f = Fixture::new();
    let first = f.save(plan(draft())).unwrap();
    let mut second = draft();
    second.label = "Other label".into();
    let second = f.save(plan(second)).unwrap();
    assert_eq!(first.preferred_term.id, second.preferred_term.id);
    f.db.lock()
        .unwrap()
        .execute(
            "update concepts set updated_at=case id when ?1 then 'z' else 'a' end",
            [&second.id],
        )
        .unwrap();
    let ConceptResult::Concepts(rows) = f.apply(search("%", "%", "%%%")).unwrap() else {
        panic!("list")
    };
    assert_eq!(
        rows.iter().map(|v| &v.id).collect::<Vec<_>>(),
        [&second.id, &first.id]
    );
    f.db.lock()
        .unwrap()
        .execute(
            "delete from concept_terms where concept_id=?1",
            [&second.id],
        )
        .unwrap();
    let error = match f.apply(search("", "", "%%")) {
        Err(e) => e,
        _ => panic!("bad row must not be skipped"),
    };
    assert_eq!(error.phase, "preferred-term");
    assert!(matches!(
        error.problem.as_ref(),
        ConceptProblem::Domain(ConceptRejectionV1::MissingPreferredTerm)
    ));
}

#[test]
fn configured_semantic_records_and_old_job_statuses_keep_original_queue_behavior() {
    let f = Fixture::new();
    f.db.lock().unwrap().execute("update ai_provider_settings set enabled=1,region='cn-beijing',model_id='text-embedding-v4' where provider_id='qwen-embedding'", []).unwrap();
    let first = f.save(plan(draft())).unwrap();
    assert_eq!(f.count("embedding_spaces"), 1);
    assert_eq!(f.count("embedding_index_generations"), 1);
    assert_eq!(f.count("embedding_records"), 1);
    let db = f.db.lock().unwrap();
    let (record, key): (String,String) = db.query_row("select r.id,j.idempotency_key from embedding_records r join embedding_outbox_jobs j on j.embedding_record_id=r.id", [], |r| Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(key, format!("embedding:{record}:1"));
    db.execute("update embedding_outbox_jobs set status='failed'", [])
        .unwrap();
    drop(db);
    let before = f.state();
    {
        let mut db = f.db.lock().unwrap();
        let tx = db.transaction().unwrap();
        super::semantic::register(
            &tx,
            super::semantic::SemanticResources::from_concepts(&f.resources),
            &first,
        )
        .unwrap_or_else(|e| panic!("semantic phase {:?}", e.phase));
        tx.commit().unwrap();
    }
    assert_eq!(f.state(), before); // failed latest job is reused, not retried.
    let mut update = draft();
    update.id = Some(first.id.clone());
    update.expected_revision = Some(1);
    f.db.lock()
        .unwrap()
        .execute("update embedding_outbox_jobs set status='running',lease_owner='synthetic-concept-worker',lease_expires_at='2999-01-01T00:00:00.000Z'", [])
        .unwrap();
    f.save(plan(update)).unwrap();
    let db = f.db.lock().unwrap();
    assert_eq!(
        db.query_row(
            "select status from embedding_records where id=?1",
            [&record],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "tombstoned"
    );
    assert_eq!(
        db.query_row(
            "select count(*) from embedding_outbox_jobs where status='canceled'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
}

#[test]
fn postcommit_read_failure_is_committed_and_never_replaced_by_input() {
    let f = Fixture::new();
    let note = f.note();
    let committed = Arc::new(AtomicBool::new(false));
    let mut db = f.db.lock().unwrap();
    let marker = Arc::clone(&committed);
    db.commit_hook(Some(move || {
        marker.store(true, Ordering::SeqCst);
        false
    }));
    db.authorizer(Some(move |context: AuthContext<'_>| match context.action {
        AuthAction::Read {
            table_name: "concepts",
            ..
        } if committed.load(Ordering::SeqCst) => Authorization::Deny,
        _ => Authorization::Allow,
    }));
    let error = match apply(&mut db, &f.resources, plan(draft())) {
        Err(e) => e,
        _ => panic!("postcommit read must fail"),
    };
    assert_eq!(error.outcome, ConceptOutcome::Committed);
    assert_eq!(error.phase, "postcommit-read");
    assert!(error.concept_id.is_some());
    assert!(db.is_autocommit());
    db.authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
    db.commit_hook(None::<fn() -> bool>);
    drop(db);
    assert_eq!(f.count("concepts"), 1);
    let state = f.state();
    let f = f.reopen();
    assert_eq!(f.state(), state);
    assert_eq!(f.note(), note);
}
