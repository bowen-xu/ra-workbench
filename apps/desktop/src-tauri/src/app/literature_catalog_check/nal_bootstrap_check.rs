//! Real complete bootstrap calls; all fixture SQL stays in this development CLI.
use super::*;
use research_workbench_native_sdk::nal_state::{NalState, NalTerm};
use serde_json::Value;
use std::sync::atomic::Ordering;

type Error = Box<dyn std::error::Error>;

use super::{nal_bootstrap_fixture as fixture, nal_bootstrap_readback as readback};

async fn read(
    store: &AppStore,
    service: &Arc<LiteratureTraceService>,
    key: &str,
) -> Result<NalState, Error> {
    Ok(service
        .read_nal_bootstrap(store.shared_agent_connection(), key.into())
        .await?)
}

pub(super) async fn run(
    store: &AppStore,
    service: &Arc<LiteratureTraceService>,
    key: &str,
) -> Result<Value, Error> {
    let (note, queue_attempts) = fixture::seed(store)?;
    let note_before = fixture::note(store, &note)?;
    let before = readback::snapshot(store)?;
    let failure = service
        .read_nal_bootstrap(store.shared_agent_connection(), key.into())
        .await
        .err()
        .ok_or("malformed final fact unexpectedly committed bootstrap")?;
    let failure_json = serde_json::to_value(&failure)?;
    let attempted = queue_attempts.load(Ordering::SeqCst);
    if failure.code != "invalid_input"
        || failure.message != "关系数据结构无效"
        || failure_json["bootstrapOutcome"]["status"] != "rolledBack"
        || failure.package_key.as_deref() != Some(key)
        || failure.instance_id.is_none()
        || failure.request_id.is_none()
        || attempted == 0
        || readback::snapshot(store)? != before
        || fixture::note(store, &note)? != note_before
    {
        return Err(format!("native bootstrap did not roll back all original writes/queue after final fact decode: {failure:?}; queue attempts={attempted}").into());
    }
    // Repair only this CLI's malformed synthetic input. Its annotation cascades;
    // the real Core note and every prior rollback snapshot remain untouched.
    if store
        .connection()?
        .execute("delete from nal_statements where id=?1", [fixture::BAD])?
        != 1
    {
        return Err("synthetic malformed row repair did not affect exactly one row".into());
    }
    let imported = read(store, service, key).await?;
    assert_import(&imported, &note)?;
    if fixture::note(store, &note)? != note_before
        || imported.statements.len() != 8
        || readback::scalar(
            store,
            "select legacy_imported=1 and relation_semantics_version=1 from nal_state where id=1",
        )? != 1
    {
        return Err("complete original import/repair state, flags or Core note differs".into());
    }
    let metadata = readback::metadata(store)?;
    let stable = readback::snapshot(store)?;
    if read(store, service, key).await? != imported || readback::snapshot(store)? != stable {
        return Err(
            "completed bootstrap repeated import/repair or rewrote persistent metadata".into(),
        );
    }
    // Unique active concept names permit only one related-field placeholder.
    // First prove its new same-transaction reference protects it; then remove
    // just that synthetic link and exercise the original unused cleanup branch.
    let linked=imported.statements.iter().find(|statement| {
        matches!(&statement.subject,NalTerm::Product{terms} if terms.iter().any(|term|matches!(term,NalTerm::Concept{id} if id==fixture::PLACEHOLDER)))
    }).ok_or("new imported placeholder reference missing")?;
    if store
        .connection()?
        .execute("delete from nal_statements where id=?1", [&linked.id])?
        != 1
    {
        return Err("synthetic linked statement removal failed".into());
    }
    let final_state = read(store, service, key).await?;
    if final_state
        .concepts
        .iter()
        .any(|concept| concept.id == fixture::PLACEHOLDER)
        || final_state.statements.len() != 7
        || final_state.revision != imported.revision + 1
        || readback::scalar(
            store,
            "select deleted_at is not null from concepts where id='bootstrap-obsolete'",
        )? != 1
        || readback::scalar(
            store,
            "select count(*) from concept_search where concept_id='bootstrap-obsolete'",
        )? != 0
        || fixture::note(store, &note)? != note_before
    {
        return Err(
            "unused placeholder cleanup did not preserve original lifecycle and Core note".into(),
        );
    }
    let final_snapshot = readback::snapshot(store)?;
    if read(store, service, key).await? != final_state
        || readback::snapshot(store)? != final_snapshot
    {
        return Err("final bootstrap read did not remain persistent and idempotent".into());
    }
    Ok(
        json!({"state":final_state,"initialImportedState":imported,"sqlSnapshot":final_snapshot,
        "noteId":note,"noteDocument":note_before,"metadata":metadata,"finalDecodeFailure":failure_json,
        "wholeBootstrapRolledBack":true,"failedQueueInsertAttempts":attempted,
        "sameTransactionNewReferenceProtectedPlaceholder":true,"unusedPlaceholderCleaned":true,
        "repairStatementIdPreserved":true,"completeImportBranches":true,"completedReadsDidNotWrite":true,
        "coreOnly":true}),
    )
}

fn assert_import(state: &NalState, note: &str) -> Result<(), Error> {
    let concept = |label: &str| {
        state
            .concepts
            .iter()
            .find(|concept| concept.label == label)
            .map(|concept| concept.id.as_str())
            .ok_or("expected imported concept missing")
    };
    let a = concept("Synthetic collection A")?;
    let b = concept("Synthetic collection B")?;
    let symmetric = concept("Synthetic symmetric relation")?;
    let membership = concept("原归类")?;
    let tagged = concept("旧关系标注")?;
    let has = |subject: NalTerm, predicate: &str| {
        state
            .statements
            .iter()
            .any(|statement| statement.subject == subject && statement.predicate == term(predicate))
    };
    if !has(product(vec![term(a), term(b)]), symmetric)
        || !has(product(vec![term(b), term(a)]), symmetric)
        || !has(
            product(vec![
                NalTerm::Paper {
                    id: "bootstrap-paper".into(),
                },
                term(a),
            ]),
            membership,
        )
        || !has(
            product(vec![
                NalTerm::Paper {
                    id: "bootstrap-paper".into(),
                },
                term(b),
            ]),
            membership,
        )
        || !has(
            product(vec![
                NalTerm::Paper {
                    id: "bootstrap-paper".into(),
                },
                product(vec![term(a), term(b)]),
                term(symmetric),
            ]),
            tagged,
        )
        || !state
            .concepts
            .iter()
            .any(|concept| concept.id == fixture::PLACEHOLDER)
    {
        return Err(
            "import omitted direction, membership, assignment or linked placeholder".into(),
        );
    }
    let subfield = concept("旧上下位关系")?;
    if !has(product(vec![term(b), term(a)]), subfield) {
        return Err("sub-field import changed its original non-inheritance product".into());
    }
    let repaired = state
        .statements
        .iter()
        .find(|statement| statement.id == fixture::REPAIR)
        .ok_or("original repair statement id was replaced")?;
    let repair_predicate = concept("旧上下位关系`旧归类-2")?;
    if repaired.subject != product(vec![term("bootstrap-endpoint"), term("bootstrap-endpoint")])
        || repaired.predicate != term(repair_predicate)
        || repaired.origin != "legacy"
        || !state.annotations.iter().any(|annotation| {
            annotation.id == "bootstrap-repair-note"
                && annotation.target_id == fixture::REPAIR
                && annotation.note_page_id == note
        })
        || state
            .annotations
            .iter()
            .any(|annotation| annotation.id == "bootstrap-bad-note")
    {
        return Err(
            "repair lost original statement/annotation or ordered name collision semantics".into(),
        );
    }
    Ok(())
}

fn term(id: &str) -> NalTerm {
    NalTerm::Concept { id: id.into() }
}
fn product(terms: Vec<NalTerm>) -> NalTerm {
    NalTerm::Product { terms }
}

pub(super) async fn reopen(
    store: &AppStore,
    service: &Arc<LiteratureTraceService>,
    key: &str,
    directory: &Path,
) -> Result<Value, Error> {
    let original: Value = serde_json::from_slice(&std::fs::read(directory.join("receipt.json"))?)?;
    if original["status"] != "passed-nal-bootstrap-native-core-only"
        || original["packageKey"] != key
    {
        return Err("cold bootstrap reopen requires this exact successful package receipt".into());
    }
    let expected: NalState = serde_json::from_value(original["state"].clone())?;
    let before = readback::snapshot(store)?;
    if before != original["sqlSnapshot"] {
        return Err(
            "cold SQL/log reopen changed committed bootstrap metadata before native read".into(),
        );
    }
    let actual = read(store, service, key).await?;
    let after = readback::snapshot(store)?;
    let note = original["noteId"]
        .as_str()
        .ok_or("original Core note id missing")?;
    if actual != expected
        || before != after
        || fixture::note(store, note)? != original["noteDocument"]
        || readback::metadata(store)? != original["metadata"]
    {
        return Err(
            "cold reopened bootstrap changed complete State, metadata, queue or Core note".into(),
        );
    }
    Ok(
        json!({"state":actual,"sqlSnapshot":after,"noteId":note,"noteDocument":original["noteDocument"],
        "metadata":original["metadata"],"sameCompleteState":true,"sameSqlMetadataSnapshot":true,
        "sameCoreNote":true,"seededOrReplayed":false,"originalReceipt":"receipt.json",
        "coreOnly":true}),
    )
}
