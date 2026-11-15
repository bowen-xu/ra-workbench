//! Real Core ownership and transaction reentry with finite canned Native decisions.
use super::*;
use crate::runtime_plugins::literature_claims::{
    self, ClaimsError, ClaimsPolicy, ClaimsRequest, ClaimsResult, ClaimsSql,
};
use research_workbench_native_sdk::{catalog::CatalogOutput, evidence_write::*};
use std::{fs, sync::Arc};
struct Static;
impl ClaimsPolicy for Static {
    fn validate(&mut self, input: EvidenceWriteInputV1) -> Result<(), ClaimsError> {
        match research_literature_library_native::evidence_write::decide(&input).rejection {
            None => Ok(()),
            Some(reason) => Err(reason.into()),
        }
    }
}
fn sql() -> ClaimsSql {
    ClaimsSql::for_test(
        &research_literature_library_native::evidence_write::storage::static_sql(),
        crate::runtime_plugins::literature_evidence::EvidenceReadSql::for_test(
            research_literature_library_native::evidence_read::static_sql(),
        ),
    )
}
fn draft() -> ClaimDraft {
    serde_json::from_value(json!({"projectId":"matrix-project","statement":" Manual original Ω "}))
        .unwrap()
}
fn ready() -> CatalogOutput {
    CatalogOutput::EvidenceWriteDecision {
        decision: EvidenceWriteDecisionV1 { rejection: None },
    }
}
fn setup(update: bool, outputs: &[CatalogOutput], gate: Option<usize>) -> (Fixture, ClaimDraft) {
    let mut f = Fixture::new();
    matrix_fixture::seed(&f);
    let mut claim = draft();
    if update {
        let original = literature_claims::save_claim_from(
            &sql(),
            &mut Static,
            false,
            &mut f.store.connection().unwrap(),
            claim.clone(),
        )
        .unwrap();
        claim.id = Some(original.id);
        claim.expected_revision = Some(1);
        claim.statement = "First requested edit".into();
    }
    let directory = f.package.path().join("writing");
    let path = directory.join("manifest.json");
    let mut manifest: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    manifest["provides"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id":"literature.evidence-write","version":1}));
    fs::write(path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../plugins/literature_library/native/data");
    for suffix in [
        "claim",
        "revision",
        "update",
        "insert",
        "history",
        "link-upsert",
        "link-lookup",
    ] {
        let file = format!("evidence-write-{suffix}.v1.sql");
        fs::copy(source.join(&file), directory.join("data").join(file)).unwrap();
    }
    for suffix in ["claims", "source", "sources", "links"] {
        let file = format!("evidence-read-{suffix}.v1.sql");
        fs::copy(source.join(&file), directory.join("data").join(file)).unwrap();
    }
    f.configure(outputs, gate, false);
    (f, claim)
}
fn start(
    f: &Fixture,
    claim: ClaimDraft,
) -> tokio::task::JoinHandle<crate::runtime_plugins::native::NativeResult<ClaimsResult>> {
    let service = Arc::clone(&f.service);
    let db = f.store.shared_agent_connection();
    let key = f.key.clone();
    tokio::spawn(async move {
        service
            .evidence_write(
                db,
                key,
                ClaimsRequest::Create {
                    claim: Box::new(claim),
                    evidence: NewEvidenceDraft {
                        anchor_id: "matrix-first-anchor".into(),
                        stance: "context".into(),
                        rationale: "Chosen source".into(),
                    },
                },
            )
            .await
    })
}
fn counts(f: &Fixture) -> (i64, i64, i64) {
    f.store.connection().unwrap().query_row("select (select count(*) from claims),(select count(*) from research_object_revisions where object_kind='claim'),(select count(*) from evidence_links)",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap()
}
async fn finish(f: Fixture) {
    f.service.shutdown().await.unwrap();
    f.store.shared_agent_connection().shutdown().unwrap();
}
#[tokio::test]
async fn evidence_write_stance_wait_rolls_back_claim_prefix_releases_guard_and_rechecks_cas() {
    let (f, claim) = setup(true, &[ready(), ready()], Some(1));
    let id = claim.id.clone().unwrap();
    let task = start(&f, claim);
    assert!(f.entered(1).await);
    assert_eq!(counts(&f), (1, 1, 0));
    let store = f.store.clone();
    tokio::time::timeout(
        WAIT,
        tokio::task::spawn_blocking(move || {
            let mut db = store.connection().unwrap();
            assert!(db.is_autocommit());
            let mut changed = draft();
            changed.id = Some(id);
            changed.expected_revision = Some(1);
            changed.statement = "Concurrent accepted edit".into();
            literature_claims::save_claim_from(&sql(), &mut Static, false, &mut db, changed)
                .unwrap();
        }),
    )
    .await
    .unwrap()
    .unwrap();
    f.release();
    let error = task.await.unwrap().unwrap_err();
    assert_eq!(error.code, "revision_conflict");
    assert_eq!(error.current_revision, Some(2));
    assert_eq!(counts(&f), (1, 2, 0));
    let statement: String = f
        .store
        .connection()
        .unwrap()
        .query_row("select statement from claims", [], |r| r.get(0))
        .unwrap();
    assert_eq!(statement, "Concurrent accepted edit");
    finish(f).await;
}
#[tokio::test]
async fn evidence_write_caller_loss_keeps_owned_atomic_save_after_stance_wait() {
    let (f, claim) = setup(false, &[ready(), ready()], Some(1));
    let task = start(&f, claim);
    assert!(f.entered(1).await);
    assert_eq!(counts(&f), (0, 0, 0));
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    f.release();
    tokio::time::timeout(WAIT, async {
        loop {
            if counts(&f) == (1, 1, 1) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    finish(f).await;
}
#[tokio::test]
async fn evidence_write_shutdown_during_stance_wait_leaves_no_partial_claim_or_history() {
    let (f, claim) = setup(false, &[ready(), ready()], Some(1));
    let task = start(&f, claim);
    assert!(f.entered(1).await);
    assert_eq!(counts(&f), (0, 0, 0));
    let stopping = f.service.stopped.notified();
    tokio::pin!(stopping);
    stopping.as_mut().enable();
    let service = Arc::clone(&f.service);
    let shutdown = tokio::spawn(async move { service.shutdown().await });
    tokio::time::timeout(WAIT, &mut stopping).await.unwrap();
    f.release();
    assert_eq!(
        task.await.unwrap().unwrap_err().code,
        "plugin_host_stopping"
    );
    tokio::time::timeout(WAIT, shutdown)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(counts(&f), (0, 0, 0));
    f.store.shared_agent_connection().shutdown().unwrap();
}
#[tokio::test]
async fn evidence_write_wrong_rejection_family_cannot_publish_claim() {
    let output = CatalogOutput::EvidenceWriteDecision {
        decision: EvidenceWriteDecisionV1 {
            rejection: Some(EvidenceWriteRejectionV1::InvalidStance("context".into())),
        },
    };
    let (f, claim) = setup(false, &[output], None);
    let error = start(&f, claim).await.unwrap().unwrap_err();
    assert_eq!(error.code, "plugin_host_response_mismatch");
    assert_eq!(counts(&f), (0, 0, 0));
    finish(f).await;
}
#[tokio::test]
async fn evidence_write_failure_after_reentry_retains_allocated_id_and_unknown_rollback_fence() {
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization, TransactionOperation};
    let (f, claim) = setup(false, &[ready(), ready()], Some(1));
    let task = start(&f, claim);
    assert!(f.entered(1).await);
    assert_eq!(counts(&f), (0, 0, 0));
    {
        let db = f.store.connection().unwrap();
        assert!(db.is_autocommit());
        db.execute_batch("create temp trigger synthetic_link_failure before insert on evidence_links begin select raise(abort,'private-evidence'); end;").unwrap();
        db.authorizer(Some(|context: AuthContext<'_>| match context.action {
            AuthAction::Transaction {
                operation: TransactionOperation::Rollback,
            } => Authorization::Deny,
            _ => Authorization::Allow,
        }));
    }
    f.release();
    let error = task.await.unwrap().unwrap_err();
    let value = serde_json::to_value(&error).unwrap();
    assert_eq!(value["evidenceWrite"]["outcome"], "unknown");
    uuid::Uuid::parse_str(value["evidenceWrite"]["objectId"].as_str().unwrap()).unwrap();
    assert!(error.instance_id.is_some() && error.request_id.is_some());
    assert!(f.store.connection().is_err());
    assert!(!value.to_string().contains("private-evidence"));
    f.service.shutdown().await.unwrap();
    assert!(
        f.store
            .shared_agent_connection()
            .shutdown()
            .unwrap_err()
            .to_string()
            .contains("data_recovery_required")
    );
}
