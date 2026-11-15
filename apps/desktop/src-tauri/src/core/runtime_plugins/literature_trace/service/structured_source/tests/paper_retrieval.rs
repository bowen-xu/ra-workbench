//! Paper trace consumption stays inside the original submission transaction.
use super::{
    paper_submission::{counts, prepare, spawn},
    *,
};
use crate::{agent::retrieval_context::RetrievalSelection, data_interface::PaperPrepareIntent};
use research_workbench_native_sdk::{
    catalog::CatalogOutput, paper_retrieval::PaperRetrievalInputV1,
};
use sha2::{Digest, Sha256};

fn with_trace(f: &Fixture, expires: &str) -> (PaperPrepareIntent, Vec<CatalogOutput>) {
    let (mut intent, mut outputs) = prepare(f);
    let body = "[CN-S1] Synthetic paper evidence Ω";
    let hash = |value: &str| format!("{:x}", Sha256::digest(value.as_bytes()));
    let db = f.store.connection().unwrap();
    db.execute(
        "insert into concept_retrieval_traces(id,paper_id,query_sha256,status,config_json,context_text,context_sha256,seed_count,edge_count,source_count,external_paper_count,created_at,expires_at) values('paper-trace','paper',?1,'previewed','{}',?2,?3,0,0,0,0,'now',?4)",
        rusqlite::params![hash(intent.request.text.trim()), body, hash(body), expires],
    ).unwrap();
    intent.request.retrieval = Some(RetrievalSelection {
        trace_id: "paper-trace".into(),
        query: intent.request.text.clone(),
    });
    let row = research_workbench_native_sdk::retrieval_trace::RetrievalTraceRow {
        paper_id: Some("paper".into()),
        query_sha256: hash(intent.request.text.trim()),
        status: "previewed".into(),
        context_text: body.into(),
        context_sha256: hash(body),
        expired: expires.starts_with("2000"),
    };
    let outcome =
        research_literature_library_native::paper_retrieval::decide(PaperRetrievalInputV1 {
            paper_id: "paper".into(),
            query: intent.request.text.clone(),
            row: Some(row),
        });
    outputs.push(CatalogOutput::PaperRetrievalTrace {
        outcome: Box::new(outcome),
    });
    (intent, outputs)
}
fn state(f: &Fixture) -> String {
    f.store
        .connection()
        .unwrap()
        .query_row(
            "select status from concept_retrieval_traces where id='paper-trace'",
            [],
            |row| row.get(0),
        )
        .unwrap()
}

#[tokio::test]
async fn changed_trace_during_native_wait_rejects_original_readset_without_consumption() {
    let mut f = Fixture::new();
    let (intent, outputs) = with_trace(&f, "2999-01-01T00:00:00Z");
    f.configure(&outputs, Some(3), false);
    let caller = spawn(&f, intent);
    assert!(f.entered(3).await);
    assert_eq!(counts(&f), (0, 0, 0));
    f.store.connection().unwrap().execute(
        "update concept_retrieval_traces set context_text='changed while waiting' where id='paper-trace'", [],
    ).unwrap();
    f.release();
    let error = serde_json::to_value(caller.await.unwrap().unwrap_err()).unwrap();
    assert_eq!(
        error["nativeFailure"]["dataError"]["code"], "plugin_data_snapshot_changed",
        "{error}"
    );
    assert_eq!(error["nativeFailure"]["outcome"], "rolledBack");
    assert!(error["nativeFailure"].get("nativeRequestId").is_none());
    assert_eq!(counts(&f), (0, 0, 0));
    assert_eq!(state(&f), "previewed");
    f.service.shutdown().await.unwrap();
}

#[tokio::test]
async fn revoked_binding_wins_over_cached_trace_decision() {
    let mut f = Fixture::new();
    let (intent, outputs) = with_trace(&f, "2999-01-01T00:00:00Z");
    let conversation = intent.request.conversation_id;
    f.configure(&outputs, Some(3), false);
    let caller = spawn(&f, intent);
    assert!(f.entered(3).await);
    assert_eq!(counts(&f), (0, 0, 0));
    f.store
        .connection()
        .unwrap()
        .execute(
            "delete from agent_conversation_bindings where conversation_id=?1",
            [conversation.to_string()],
        )
        .unwrap();
    f.release();
    let error = serde_json::to_value(caller.await.unwrap().unwrap_err()).unwrap();
    assert_eq!(
        error["reason"]["reason"]["safeCause"], "paper_preparation_unbound",
        "{error}"
    );
    assert!(error.get("nativeFailure").is_none());
    assert_eq!(counts(&f), (0, 0, 0));
    assert_eq!(state(&f), "previewed");
    f.service.shutdown().await.unwrap();
}

#[tokio::test]
async fn original_expiry_sql_failure_precedes_native_expired_and_rolls_back() {
    for fail_sql in [false, true] {
        let mut f = Fixture::new();
        let (intent, outputs) = with_trace(&f, "2000-01-01T00:00:00Z");
        f.configure(&outputs, None, false);
        if fail_sql {
            f.store.connection().unwrap().execute_batch(
                "create trigger deny_expiry before update of status on concept_retrieval_traces when new.status='expired' begin select raise(abort,'PRIVATE_TRACE_EXPIRY'); end;",
            ).unwrap();
        }
        let error = serde_json::to_value(spawn(&f, intent).await.unwrap().unwrap_err()).unwrap();
        assert_eq!(error["nativeFailure"]["outcome"], "rolledBack", "{error}");
        if fail_sql {
            assert_eq!(
                error["nativeFailure"]["dataError"]["code"],
                "plugin_data_sql_failed"
            );
            assert_eq!(
                error["nativeFailure"]["dataError"]["resourceId"],
                "literature.retrieval.trace-expire"
            );
            assert!(error["nativeFailure"].get("nativeRequestId").is_none());
        } else {
            assert!(error["nativeFailure"]["nativeRequestId"].as_str().is_some());
        }
        assert!(!error.to_string().contains("PRIVATE_TRACE_EXPIRY"));
        assert_eq!(counts(&f), (0, 0, 0));
        assert_eq!(state(&f), "previewed");
        f.service.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn original_mark_cas_failure_has_no_candidate_native_identity() {
    let mut f = Fixture::new();
    let (intent, outputs) = with_trace(&f, "2999-01-01T00:00:00Z");
    f.configure(&outputs, None, false);
    f.store.connection().unwrap().execute_batch(
        "create trigger ignore_mark before update of status on concept_retrieval_traces when new.status='sent' begin select raise(ignore); end;",
    ).unwrap();
    let error = serde_json::to_value(spawn(&f, intent).await.unwrap().unwrap_err()).unwrap();
    assert!(error.get("nativeFailure").is_none(), "{error}");
    assert_eq!(counts(&f), (0, 0, 0));
    assert_eq!(state(&f), "previewed");
    f.service.shutdown().await.unwrap();
}

#[tokio::test]
async fn detached_receiver_keeps_original_core_owner_through_trace_consumption() {
    let mut f = Fixture::new();
    let (intent, outputs) = with_trace(&f, "2999-01-01T00:00:00Z");
    f.configure(&outputs, Some(3), false);
    let caller = spawn(&f, intent);
    assert!(f.entered(3).await);
    assert_eq!(counts(&f), (0, 0, 0));
    caller.abort();
    assert!(caller.await.unwrap_err().is_cancelled());
    f.release();
    tokio::time::timeout(WAIT, async {
        while counts(&f) != (1, 1, 1) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(state(&f), "sent");
    f.service.shutdown().await.unwrap();
}
