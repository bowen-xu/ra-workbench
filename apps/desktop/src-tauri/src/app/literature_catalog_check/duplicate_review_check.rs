//! Real native duplicate-review calls over one synthetic Core SQL/log owner.
use super::*;
use crate::runtime_plugins::literature_duplicates::{
    DecisionSqlState, ReviewRequest, ReviewResult,
};
use research_workbench_native_sdk::duplicate_review::{
    DuplicateCandidate, DuplicateDecisionRequestV1, MergeOperation,
};
use std::collections::BTreeSet;

#[path = "duplicate_review_check/fixture.rs"]
mod fixture;
#[path = "duplicate_review_check/readback.rs"]
mod readback;

fn request(id: &str, decision: &str) -> ReviewRequest {
    ReviewRequest::Decide(DuplicateDecisionRequestV1 {
        candidate_id: id.into(),
        decision: decision.into(),
    })
}

async fn candidates(
    store: &AppStore,
    service: &Arc<LiteratureTraceService>,
    key: &str,
    note: &Value,
) -> Result<Vec<DuplicateCandidate>, Error> {
    let before = readback::snapshot(store)?;
    let result = service
        .review_duplicates(
            store.shared_agent_connection(),
            key.into(),
            ReviewRequest::Candidates,
        )
        .await?;
    readback::assert_note(store, note)?;
    if readback::snapshot(store)? != before {
        return Err("native candidate projection wrote to the synthetic database".into());
    }
    match result {
        ReviewResult::Candidates(items) => Ok(items),
        _ => Err("native candidate projection returned the wrong result kind".into()),
    }
}

async fn history(
    store: &AppStore,
    service: &Arc<LiteratureTraceService>,
    key: &str,
    note: &Value,
) -> Result<Vec<MergeOperation>, Error> {
    let before = readback::snapshot(store)?;
    let result = service
        .review_duplicates(
            store.shared_agent_connection(),
            key.into(),
            ReviewRequest::History,
        )
        .await?;
    readback::assert_note(store, note)?;
    if readback::snapshot(store)? != before {
        return Err("native history projection wrote to the synthetic database".into());
    }
    match result {
        ReviewResult::History(items) => Ok(items),
        _ => Err("native history projection returned the wrong result kind".into()),
    }
}

struct ExpectedFailure {
    code: &'static str,
    message: Option<&'static str>,
    sql: DecisionSqlState,
}

async fn rejected(
    store: &AppStore,
    service: &Arc<LiteratureTraceService>,
    key: &str,
    note: &Value,
    request: ReviewRequest,
    expected: ExpectedFailure,
) -> Result<Value, Error> {
    let before = readback::snapshot(store)?;
    let error = service
        .review_duplicates(store.shared_agent_connection(), key.into(), request)
        .await
        .err()
        .ok_or("synthetic rejected review must not succeed")?;
    readback::assert_note(store, note)?;
    if error.code != expected.code
        || error.package_key.as_deref() != Some(key)
        || error.plugin_id != "literature_library"
        || error.duplicate_sql != Some(expected.sql)
        || expected
            .message
            .is_some_and(|message| error.message != message)
        || error.exit_code.is_some()
        || error.signal.is_some()
        || error.cleanup_error.is_some()
    {
        return Err(format!("unexpected native duplicate diagnostic: {error:?}").into());
    }
    for id in [error.instance_id.as_deref(), error.request_id.as_deref()] {
        if id.is_none_or(|id| uuid::Uuid::parse_str(id).is_err()) {
            return Err("native duplicate failure lost its real instance/request identity".into());
        }
    }
    if expected.code == "storage_error" {
        let cause = error
            .data_error
            .as_ref()
            .ok_or("SQL failure lost its data diagnostic")?;
        if cause.phase != "execute"
            || cause.sqlite_extended_code != Some(1811)
            || cause.safe_cause != "ConstraintViolation"
            || error.message != format!("{}：{}", cause.message, cause.safe_cause)
        {
            return Err("actual FAIL lost its safe original SQL cause or numeric code".into());
        }
    }
    let encoded = serde_json::to_string(&error)?;
    if encoded.contains(fixture::PRIVATE_TRIGGER) {
        return Err("native error exposed raw synthetic trigger text".into());
    }
    Ok(json!({"error":error,"sqlBefore":before,"sqlAfter":readback::snapshot(store)?}))
}

fn unchanged(failure: &Value) -> Result<(), Error> {
    if failure["sqlBefore"] != failure["sqlAfter"] {
        return Err("rejected zero-effect duplicate call changed its fixed SQL snapshot".into());
    }
    Ok(())
}

fn trigger_only(failure: &Value, id: &str, kind: &str) -> Result<(), Error> {
    let mut before = failure["sqlBefore"].clone();
    let mut after = failure["sqlAfter"].clone();
    let old = before
        .as_object_mut()
        .ok_or("before snapshot missing")?
        .remove("syntheticEffects")
        .ok_or("before effects missing")?;
    let current = after
        .as_object_mut()
        .ok_or("after snapshot missing")?
        .remove("syntheticEffects")
        .ok_or("after effects missing")?;
    let old = old.as_array().ok_or("before effects not an array")?;
    let current = current.as_array().ok_or("after effects not an array")?;
    if before != after
        || current.len() != old.len() + 1
        || &current[..old.len()] != old.as_slice()
        || current
            .last()
            .is_none_or(|row| row["candidate_id"] != id || row["kind"] != kind)
    {
        return Err("IGNORE/FAIL did not retain exactly its original early trigger write".into());
    }
    Ok(())
}

pub(super) async fn run(
    store: &AppStore,
    service: &Arc<LiteratureTraceService>,
    key: &str,
) -> Result<Value, Error> {
    let note = fixture::seed(store)?;
    let initial = candidates(store, service, key, &note).await?;
    let near = f64::from_bits(0x3fef_ffff_ffff_ffff);
    readback::assert_candidates(
        &initial,
        &[
            (fixture::ONE, 1.0),
            (fixture::NEAR, near),
            (fixture::POINT, 0.72),
        ],
    )?;
    let initial_history = history(store, service, key, &note).await?;
    readback::assert_history(&initial_history, &["history-archived", "history-active"])?;
    fixture::set_trash(store, None)?;
    let restored_history = history(store, service, key, &note).await?;
    readback::assert_history(
        &restored_history,
        &["history-trashed", "history-archived", "history-active"],
    )?;
    let restored_candidates = candidates(store, service, key, &note).await?;
    if restored_candidates
        .iter()
        .map(|item| item.id.as_str())
        .collect::<Vec<_>>()
        != [
            fixture::ONE,
            fixture::NEAR,
            "candidate-trashed",
            fixture::POINT,
        ]
    {
        return Err("candidate trash restoration changed original filtering/order".into());
    }
    fixture::set_trash(store, Some("2026"))?;
    let invalid = rejected(
        store,
        service,
        key,
        &note,
        request(fixture::NEAR, " dismiss "),
        ExpectedFailure {
            code: "invalid_input",
            message: Some("不支持的文献类型： dismiss "),
            sql: DecisionSqlState::NotDispatched,
        },
    )
    .await?;
    unchanged(&invalid)?;
    let mut successes = vec![];
    for (id, decision) in [(fixture::ONE, "keep_both"), (fixture::POINT, "dismiss")] {
        let before = readback::snapshot(store)?;
        match service
            .review_duplicates(
                store.shared_agent_connection(),
                key.into(),
                request(id, decision),
            )
            .await?
        {
            ReviewResult::Decided => {}
            _ => return Err("normal native decision returned wrong result".into()),
        }
        readback::assert_note(store, &note)?;
        let observed = readback::decision(store, id)?;
        if observed["decision"] != decision
            || observed["decidedAt"].as_str().is_none_or(str::is_empty)
            || observed["effects"] != 0
        {
            return Err("normal decision did not persist its actual choice/timestamp".into());
        }
        let after = readback::snapshot(store)?;
        let mut expected = before.clone();
        let target = expected["duplicateCandidates"]
            .as_array_mut()
            .ok_or("candidate snapshot missing")?
            .iter_mut()
            .find(|row| row["id"] == id)
            .ok_or("normal candidate snapshot missing")?;
        target["decision"] = json!(decision);
        target["decidedAt"] = observed["decidedAt"].clone();
        if expected != after {
            return Err("normal review changed more than its actual decision/timestamp".into());
        }
        successes.push(json!({"candidateId":id,"decision":decision,"actualSql":observed,"sqlBefore":before,"sqlAfter":after}));
    }
    let mut failures = vec![invalid];
    for id in [fixture::ONE, "missing-candidate", " candidate-next-down "] {
        let failure = rejected(
            store,
            service,
            key,
            &note,
            request(id, "dismiss"),
            ExpectedFailure {
                code: "not_found",
                message: Some("找不到文献或附件"),
                sql: DecisionSqlState::Returned {
                    affected_rows: 0,
                    observed_autocommit: true,
                },
            },
        )
        .await?;
        unchanged(&failure)?;
        failures.push(failure);
    }
    fixture::add_fault_candidates(store)?;
    let ignored = rejected(
        store,
        service,
        key,
        &note,
        request(fixture::IGNORE, "dismiss"),
        ExpectedFailure {
            code: "not_found",
            message: Some("找不到文献或附件"),
            sql: DecisionSqlState::Returned {
                affected_rows: 0,
                observed_autocommit: true,
            },
        },
    )
    .await?;
    trigger_only(&ignored, fixture::IGNORE, "ignore")?;
    let failed = rejected(
        store,
        service,
        key,
        &note,
        request(fixture::FAIL, "dismiss"),
        ExpectedFailure {
            code: "storage_error",
            message: None,
            sql: DecisionSqlState::Failed {
                observed_autocommit: true,
            },
        },
    )
    .await?;
    trigger_only(&failed, fixture::FAIL, "fail")?;
    failures.extend([ignored, failed]);
    let instance = failures[0]["error"]["instanceId"].clone();
    let mut requests = BTreeSet::new();
    for failure in &failures {
        if failure["error"]["instanceId"] != instance
            || !requests.insert(
                failure["error"]["requestId"]
                    .as_str()
                    .ok_or("request ID missing")?,
            )
        {
            return Err(
                "healthy duplicate calls replaced host or reused a request identity".into(),
            );
        }
    }
    let final_candidates = candidates(store, service, key, &note).await?;
    readback::assert_candidates(
        &final_candidates,
        &[
            (fixture::IGNORE, 1.0),
            (fixture::NEAR, near),
            (fixture::FAIL, 0.72),
        ],
    )?;
    let final_history = history(store, service, key, &note).await?;
    readback::assert_history(&final_history, &["history-archived", "history-active"])?;
    for id in [fixture::IGNORE, fixture::FAIL] {
        if readback::decision(store, id)? != json!({"decision":null,"decidedAt":null,"effects":1}) {
            return Err("IGNORE/FAIL changed decision or repeated its early write".into());
        }
    }
    Ok(json!({
        "noteId":fixture::NOTE,"noteDocument":note,"initialCandidates":readback::candidates(&initial)?,
        "initialHistory":initial_history,"restoredHistory":restored_history,"normalDecisions":successes,
        "failures":failures,"errorInstanceId":instance,"distinctErrorRequests":requests.len(),
        "finalCandidates":readback::candidates(&final_candidates)?,"finalHistory":final_history,
        "sqlSnapshot":readback::snapshot(store)?,"allThreeOriginalPorts":true,
        "originalConfidenceBitsPreserved":true,"successfulCallsExposeRequestIdentity":false,
        "snapshotScope":"four fixed synthetic/domain table projections, one Core note page/blocks/revisions and two synthetic trigger definitions; not a whole-database or file-byte snapshot",
        "coreOnly":true
    }))
}

pub(super) async fn reopen(
    store: &AppStore,
    service: &Arc<LiteratureTraceService>,
    key: &str,
    directory: &Path,
) -> Result<Value, Error> {
    let original: Value = serde_json::from_slice(&std::fs::read(directory.join("receipt.json"))?)?;
    if original["status"] != "passed-duplicate-review-native-core-only"
        || original["packageKey"] != key
        || original["allThreeOriginalPorts"] != true
        || original["shutdown"] != "passed"
        || original["coreOnly"] != true
        || original["originalConfidenceBitsPreserved"] != true
        || original["noteId"] != fixture::NOTE
    {
        return Err(
            "cold duplicate review requires this exact successfully settled synthetic receipt"
                .into(),
        );
    }
    let note = &original["noteDocument"];
    readback::assert_note(store, note)?;
    let before = readback::snapshot(store)?;
    if before != original["sqlSnapshot"] {
        return Err("fresh SQL/log reopen changed the fixed duplicate/Core note snapshot".into());
    }
    let items = candidates(store, service, key, note).await?;
    let history = history(store, service, key, note).await?;
    let candidate_view = readback::candidates(&items)?;
    let after = readback::snapshot(store)?;
    if candidate_view != original["finalCandidates"]
        || serde_json::to_value(&history)? != original["finalHistory"]
        || before != after
    {
        return Err("cold native read changed original confidence bits/public JSON, history or persistent effects".into());
    }
    Ok(
        json!({"noteId":fixture::NOTE,"noteDocument":note,"finalCandidates":candidate_view,
        "finalHistory":history,"sqlSnapshot":after,"allThreeOriginalPorts":true,
        "sameFixedSqlSnapshot":true,"sameCoreNote":true,"originalConfidenceBitsPreserved":true,
        "successfulCallsExposeRequestIdentity":false,"seededOrReplayed":false,"originalReceipt":"receipt.json",
        "snapshotScope":original["snapshotScope"],"coreOnly":true}),
    )
}
