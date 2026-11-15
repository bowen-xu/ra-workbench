//! Original manual claim and evidence paths through real Core and the independent package.
use super::*;
use crate::runtime_plugins::{
    literature_claims::{ClaimsRequest, ClaimsResult},
    literature_evidence::{EvidenceReadRequest, EvidenceReadResult},
};
use research_workbench_native_sdk::evidence_write::*;
async fn setup() -> Fixture {
    evidence_matrix::fixture().await
}
fn draft() -> ClaimDraft {
    serde_json::from_value(json!({"projectId":"matrix-project","statement":" Human claim Ω ","changeReason":" manual_from_overview_source "})).unwrap()
}
fn source(stance: &str) -> NewEvidenceDraft {
    NewEvidenceDraft {
        anchor_id: "matrix-first-anchor".into(),
        stance: stance.into(),
        rationale: " Manually chosen Ω ".into(),
    }
}
async fn call(
    f: &Fixture,
    request: ClaimsRequest,
) -> crate::runtime_plugins::native::NativeResult<ClaimsResult> {
    f.service
        .evidence_write(f.store.shared_agent_connection(), f.key.clone(), request)
        .await
}
async fn save(
    f: &Fixture,
    draft: ClaimDraft,
) -> crate::runtime_plugins::native::NativeResult<Claim> {
    match call(
        f,
        ClaimsRequest::Save {
            draft: Box::new(draft),
        },
    )
    .await?
    {
        ClaimsResult::Claim(value) => Ok(*value),
        _ => panic!("claim result"),
    }
}
async fn create(
    f: &Fixture,
    claim: ClaimDraft,
    evidence: NewEvidenceDraft,
) -> crate::runtime_plugins::native::NativeResult<Claim> {
    match call(
        f,
        ClaimsRequest::Create {
            claim: Box::new(claim),
            evidence,
        },
    )
    .await?
    {
        ClaimsResult::Claim(value) => Ok(*value),
        _ => panic!("claim result"),
    }
}
async fn link(
    f: &Fixture,
    id: &str,
    stance: &str,
    rationale: &str,
) -> crate::runtime_plugins::native::NativeResult<EvidenceLink> {
    match call(
        f,
        ClaimsRequest::Link {
            draft: Box::new(EvidenceLinkDraft {
                claim_id: id.into(),
                anchor_id: "matrix-first-anchor".into(),
                stance: stance.into(),
                rationale: rationale.into(),
            }),
        },
    )
    .await?
    {
        ClaimsResult::Link(value) => Ok(*value),
        _ => panic!("link result"),
    }
}
async fn links(f: &Fixture, id: &str) -> Vec<EvidenceLink> {
    match f
        .service
        .evidence_read(
            f.store.shared_agent_connection(),
            f.key.clone(),
            EvidenceReadRequest::Links {
                claim_id: id.into(),
            },
        )
        .await
        .unwrap()
    {
        EvidenceReadResult::Links(value) => value,
        _ => panic!("link list"),
    }
}
async fn claims(f: &Fixture) -> Vec<Claim> {
    match f
        .service
        .evidence_read(
            f.store.shared_agent_connection(),
            f.key.clone(),
            EvidenceReadRequest::Claims {
                project_id: Some("matrix-project".into()),
            },
        )
        .await
        .unwrap()
    {
        EvidenceReadResult::Claims(value) => value,
        _ => panic!("claim list"),
    }
}
fn counts(f: &Fixture) -> (i64, i64, i64) {
    f.store.connection().unwrap().query_row("select (select count(*) from claims),(select count(*) from research_object_revisions where object_kind='claim'),(select count(*) from evidence_links)",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap()
}
fn outcome(error: &crate::runtime_plugins::native::NativeHostError) -> Value {
    serde_json::to_value(error).unwrap()["evidenceWrite"].clone()
}
#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_evidence_writes_create_edit_keep_provenance_history_and_reopen() {
    let f = setup().await;
    let created = create(&f, draft(), source("supports")).await.unwrap();
    assert_eq!(created.statement, "Human claim Ω");
    assert_eq!(created.epistemic_role, "assertion");
    assert_eq!(created.origin, "user");
    assert_eq!(created.provenance_kind, "human_original");
    assert_eq!(created.evidence_count, 1);
    assert_eq!(created.project_title.as_deref(), Some("Synthetic matrix"));
    let original = links(&f, &created.id).await;
    assert_eq!(original.len(), 1);
    assert_eq!(original[0].source.quote_exact, "Synthetic first result");
    assert_eq!(original[0].rationale, "Manually chosen Ω");
    let mut update = draft();
    update.id = Some(created.id.clone());
    update.expected_revision = Some(1);
    update.epistemic_role = "inference".into();
    update.statement = " Human revision Ω ".into();
    let updated = save(&f, update.clone()).await.unwrap();
    assert_eq!(updated.revision, 2);
    assert_eq!(updated.origin, created.origin);
    assert_eq!(updated.provenance_kind, created.provenance_kind);
    assert_eq!(updated.created_at, created.created_at);
    assert_eq!(updated.evidence_count, 1);
    let error = save(&f, update).await.unwrap_err();
    assert_eq!(error.code, "revision_conflict");
    assert_eq!(error.current_revision, Some(2));
    assert_eq!(counts(&f), (1, 2, 1));
    let snapshots:Vec<(i64,String,String)>=f.store.connection().unwrap().prepare("select revision,snapshot_json,change_reason from research_object_revisions where object_kind='claim' order by revision").unwrap().query_map([],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap().collect::<Result<_,_>>().unwrap();
    assert_eq!(snapshots[0].0, 1);
    assert_eq!(snapshots[0].2, " manual_from_overview_source ");
    assert_eq!(
        serde_json::from_str::<Value>(&snapshots[0].1).unwrap()["statement"],
        "Human claim Ω"
    );
    let f = reopen_actual_fixture(f, false).await;
    assert_eq!(claims(&f).await, vec![updated]);
    assert_eq!(links(&f, &created.id).await, original);
    evidence_matrix::close(f).await;
}
#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_evidence_writes_duplicate_links_preserve_id_time_and_three_stances() {
    let f = setup().await;
    let claim = save(&f, draft()).await.unwrap();
    assert_eq!(counts(&f), (1, 1, 0));
    for stance in ["supports", "opposes", "context"] {
        let first = link(&f, &claim.id, stance, " First Ω ").await.unwrap();
        let second = link(&f, &claim.id, stance, " Revised Ω ").await.unwrap();
        assert_eq!(first.id, second.id);
        assert_eq!(first.created_at, second.created_at);
        assert_eq!(second.rationale, "Revised Ω");
        assert_eq!(second.source, first.source);
    }
    assert_eq!(counts(&f), (1, 1, 3));
    let rows = links(&f, &claim.id).await;
    let f = reopen_actual_fixture(f, false).await;
    assert_eq!(links(&f, &claim.id).await, rows);
    assert_eq!(claims(&f).await[0].evidence_count, 3);
    evidence_matrix::close(f).await;
}
#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_evidence_writes_keep_validation_priority_and_atomic_failure() {
    let f = setup().await;
    for (statement, role, message) in [
        ("  ", "invalid", "观点内容不能为空"),
        ("Text", "invalid", "不支持的认识角色：invalid"),
    ] {
        let mut invalid = draft();
        invalid.statement = statement.into();
        invalid.epistemic_role = role.into();
        let error = create(&f, invalid, source("invalid")).await.unwrap_err();
        assert_eq!(error.code, "invalid_input");
        assert!(error.message.contains(message));
        assert_eq!(counts(&f), (0, 0, 0));
    }
    let error = create(&f, draft(), source("invalid")).await.unwrap_err();
    assert_eq!(error.code, "invalid_input");
    assert!(error.message.contains("不支持的证据关系：invalid"));
    assert_eq!(counts(&f), (0, 0, 0));
    let original = create(&f, draft(), source("context")).await.unwrap();
    let mut update = draft();
    update.id = Some(original.id.clone());
    update.expected_revision = Some(1);
    update.statement = "Must roll back".into();
    let mut missing = source("supports");
    missing.anchor_id = "missing".into();
    let error = create(&f, update, missing).await.unwrap_err();
    assert_eq!(error.code, "storage_error");
    assert_eq!(outcome(&error)["outcome"], "rolledBack");
    assert_eq!(counts(&f), (1, 1, 1));
    assert_eq!(claims(&f).await, vec![original]);
    evidence_matrix::close(f).await;
}
#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_evidence_writes_history_failure_rolls_back_original_claim_and_revision() {
    for updating in [false, true] {
        let f = setup().await;
        let mut request = draft();
        let before = if updating {
            let original = save(&f, request.clone()).await.unwrap();
            request.id = Some(original.id.clone());
            request.expected_revision = Some(1);
            request.statement = "Rejected update".into();
            vec![original]
        } else {
            vec![]
        };
        f.store.connection().unwrap().execute_batch("create temp trigger synthetic_claim_history_failure before insert on research_object_revisions when new.object_kind='claim' begin select raise(abort,'private-synthetic-claim'); end;").unwrap();
        let error = save(&f, request).await.unwrap_err();
        assert_eq!(outcome(&error)["outcome"], "rolledBack");
        assert_eq!(error.code, "storage_error");
        assert!(
            !serde_json::to_string(&error)
                .unwrap()
                .contains("private-synthetic-claim")
        );
        let f = reopen_actual_fixture(f, false).await;
        assert_eq!(claims(&f).await, before);
        assert_eq!(counts(&f), if updating { (1, 1, 0) } else { (0, 0, 0) });
        evidence_matrix::close(f).await;
    }
}
#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_evidence_writes_unknown_transaction_fences_and_reopens() {
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization, TransactionOperation};
    for rollback in [false, true] {
        let f = setup().await;
        {
            let db = f.store.connection().unwrap();
            if rollback {
                db.authorizer(Some(|context: AuthContext<'_>| match context.action {
                    AuthAction::Transaction {
                        operation: TransactionOperation::Rollback,
                    } => Authorization::Deny,
                    _ => Authorization::Allow,
                }));
            } else {
                db.commit_hook(Some(|| true));
            }
        }
        let error = create(&f, draft(), source("supports")).await.unwrap_err();
        let diagnostic = outcome(&error);
        assert_eq!(diagnostic["outcome"], "unknown");
        assert_eq!(!diagnostic["rollbackError"].is_null(), rollback);
        assert!(error.operation_id.is_some());
        assert!(error.message.contains("禁止自动重试"));
        assert!(f.store.connection().is_err());
        if rollback {
            assert!(error.instance_id.is_none() && error.request_id.is_none());
        } else {
            assert!(diagnostic["objectId"].as_str().is_some());
        }
        let f = reopen_actual_fixture(f, true).await;
        assert_eq!(counts(&f), (0, 0, 0));
        evidence_matrix::close(f).await;
    }
}
#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_evidence_writes_standalone_link_readback_failure_preserves_committed_id() {
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
    let f = setup().await;
    let claim = save(&f, draft()).await.unwrap();
    f.store
        .connection()
        .unwrap()
        .authorizer(Some(|context: AuthContext<'_>| match context.action {
            AuthAction::Read {
                table_name: "source_anchors",
                column_name: "quote_exact",
                ..
            } => Authorization::Deny,
            _ => Authorization::Allow,
        }));
    let error = link(&f, &claim.id, "supports", " Written once ")
        .await
        .unwrap_err();
    let diagnostic = outcome(&error);
    assert_eq!(diagnostic["outcome"], "committed");
    let id = diagnostic["objectId"].as_str().unwrap().to_owned();
    assert!(error.message.contains("已提交") && error.message.contains("禁止自动重放"));
    assert_eq!(counts(&f), (1, 1, 1));
    f.store
        .connection()
        .unwrap()
        .authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
    assert_eq!(links(&f, &claim.id).await[0].id, id);
    let f = reopen_actual_fixture(f, false).await;
    assert_eq!(counts(&f), (1, 1, 1));
    assert_eq!(links(&f, &claim.id).await[0].id, id);
    evidence_matrix::close(f).await;
}
#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_evidence_writes_standalone_sql_failure_preserves_usable_owner() {
    let f = setup().await;
    let claim = save(&f, draft()).await.unwrap();
    f.store.connection().unwrap().execute_batch("create temp trigger synthetic_link_failure before insert on evidence_links begin select raise(abort,'private-link'); end;").unwrap();
    let error = link(&f, &claim.id, "supports", "Not stored")
        .await
        .unwrap_err();
    assert_eq!(error.code, "storage_error");
    assert_eq!(outcome(&error)["outcome"], "statementFailed");
    assert!(f.store.connection().is_ok());
    assert_eq!(counts(&f), (1, 1, 0));
    assert!(
        !serde_json::to_string(&error)
            .unwrap()
            .contains("private-link")
    );
    let f = reopen_actual_fixture(f, false).await;
    assert_eq!(counts(&f), (1, 1, 0));
    assert!(links(&f, &claim.id).await.is_empty());
    evidence_matrix::close(f).await;
}

#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_evidence_writes_claim_postcommit_readback_preserves_committed_identity() {
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
    use std::sync::atomic::{AtomicBool, Ordering};
    let f = setup().await;
    let committed = Arc::new(AtomicBool::new(false));
    {
        let db = f.store.connection().unwrap();
        let observed = Arc::clone(&committed);
        db.commit_hook(Some(move || {
            observed.store(true, Ordering::SeqCst);
            false
        }));
        db.authorizer(Some(move |context: AuthContext<'_>| match context.action {
            AuthAction::Read {
                table_name: "claims",
                column_name: "statement",
                ..
            } if committed.load(Ordering::SeqCst) => Authorization::Deny,
            _ => Authorization::Allow,
        }));
    }
    let error = create(&f, draft(), source("supports")).await.unwrap_err();
    let diagnostic = outcome(&error);
    assert_eq!(diagnostic["outcome"], "committed");
    let id = diagnostic["objectId"].as_str().unwrap().to_owned();
    uuid::Uuid::parse_str(&id).unwrap();
    assert_eq!(counts(&f), (1, 1, 1));
    assert!(error.message.contains("已提交") && error.message.contains("禁止自动重放"));
    {
        let db = f.store.connection().unwrap();
        db.authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
        db.commit_hook(None::<fn() -> bool>);
    }
    let f = reopen_actual_fixture(f, false).await;
    assert_eq!(claims(&f).await[0].id, id);
    assert_eq!(links(&f, &id).await.len(), 1);
    assert_eq!(counts(&f), (1, 1, 1));
    evidence_matrix::close(f).await;
}

#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_evidence_writes_link_lookup_failure_keeps_unknown_identity_and_original_unique_key()
 {
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
    let f = setup().await;
    let claim = save(&f, draft()).await.unwrap();
    let original = link(&f, &claim.id, "context", "Original rationale")
        .await
        .unwrap();
    f.store
        .connection()
        .unwrap()
        .authorizer(Some(|context: AuthContext<'_>| match context.action {
            AuthAction::Read {
                table_name: "evidence_links",
                column_name: "id",
                ..
            } => Authorization::Deny,
            _ => Authorization::Allow,
        }));
    let error = link(&f, &claim.id, "context", " Updated rationale ")
        .await
        .unwrap_err();
    let diagnostic = outcome(&error);
    assert_eq!(diagnostic["outcome"], "committed");
    assert!(diagnostic["objectId"].is_null());
    assert!(error.operation_id.is_some());
    assert_eq!(counts(&f), (1, 1, 1));
    f.store
        .connection()
        .unwrap()
        .authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
    let f = reopen_actual_fixture(f, false).await;
    let rows = links(&f, &claim.id).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, original.id);
    assert_eq!(rows[0].created_at, original.created_at);
    assert_eq!(rows[0].rationale, "Updated rationale");
    evidence_matrix::close(f).await;
}
