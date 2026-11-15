//! Only the real Native/Core query-collection path, using a disposable SQL/log pair.
use super::*;
use crate::runtime_plugins::literature_nal_query::QueryOutcome;
use research_workbench_native_sdk::nal_query::{Content, Stored, StoredRow};
use rusqlite::hooks::{AuthAction, AuthContext, Authorization, TransactionOperation};
use std::sync::atomic::{AtomicUsize, Ordering};

fn content() -> Content {
    serde_json::from_value(json!({"definitions":[
        {"id":"parent","name":"合成父查询","conditions":[{
            "subject":{"kind":"variable","name":"x"},
            "predicate":{"kind":"concept","id":"synthetic-domain"}}]},
        {"id":"child","name":"嵌套查询","parentId":"parent","outputVariables":["x"],"outputMode":"all",
         "conditions":[{"kind":"group","connector":"disjunction","children":[{
            "copula":"implication","subject":{"kind":"statement","subject":{"kind":"variable","name":"x"},
                "predicate":{"kind":"paper","id":"synthetic-paper"}},
            "predicate":{"kind":"concept","id":"synthetic-result"}},
            {"kind":"group","connector":"conjunction","children":[{
                "subject":{"kind":"product","terms":[{"kind":"variable","name":"x"},
                    {"kind":"concept","id":"synthetic-domain"}]},
                "predicate":{"kind":"concept","id":"synthetic-relation"}}]}]}]}
    ],"cache":{"version":0,"opaque":[null,false,"synthetic"]},"legacyImported":true}))
    .expect("fixed synthetic query collection")
}

async fn read(
    store: &AppStore,
    service: &Arc<LiteratureTraceService>,
    key: &str,
) -> Result<Stored, Box<dyn std::error::Error>> {
    Ok(service
        .read_nal_queries(store.shared_agent_connection(), key.into())
        .await?)
}

pub(super) async fn run(
    store: &AppStore,
    service: &Arc<LiteratureTraceService>,
    key: &str,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let initial = read(store, service, key).await?;
    if initial
        != (Stored {
            revision: 0,
            content: Content {
                definitions: vec![],
                cache: None,
                legacy_imported: false,
            },
        })
    {
        return Err("fresh query collection differs from the original singleton migration".into());
    }
    let first_content = content();
    let first = service
        .save_nal_queries(
            store.shared_agent_connection(),
            key.into(),
            first_content.clone(),
            0,
        )
        .await?;
    if first
        != (Stored {
            revision: 1,
            content: first_content.clone(),
        })
    {
        return Err(
            "native query create did not preserve the complete original Stored shape".into(),
        );
    }
    let mut updated_content = first_content;
    updated_content.definitions[1].name = "更新后的合成查询".into();
    updated_content.cache = Some(json!({"version":-1,"opaque":{"members":["synthetic-paper"]}}));
    let updated = service
        .save_nal_queries(
            store.shared_agent_connection(),
            key.into(),
            updated_content.clone(),
            1,
        )
        .await?;
    if updated
        != (Stored {
            revision: 2,
            content: updated_content.clone(),
        })
    {
        return Err("native query update did not retain complete definitions/cache".into());
    }
    let stale = service
        .save_nal_queries(store.shared_agent_connection(), key.into(), content(), 1)
        .await
        .err()
        .ok_or("stale query save unexpectedly committed")?;
    if stale.code != "invalid_input"
        || stale.message != "查询集合已变化，请重试"
        || stale.query_outcome != Some(QueryOutcome::RolledBack)
        || stale.query_expected_revision != Some(1)
        || stale.instance_id.is_none()
        || stale.request_id.is_none()
        || read(store, service, key).await? != updated
    {
        return Err(format!("query CAS rejection lost its rollback/identity: {stale:?}").into());
    }

    let begin_attempts = Arc::new(AtomicUsize::new(0));
    let captured = Arc::clone(&begin_attempts);
    store
        .connection()?
        .authorizer(Some(move |context: AuthContext<'_>| {
            if matches!(
                context.action,
                AuthAction::Transaction {
                    operation: TransactionOperation::Begin
                }
            ) {
                captured.fetch_add(1, Ordering::SeqCst);
                Authorization::Deny
            } else {
                Authorization::Allow
            }
        }));
    let mut invalid = updated_content.clone();
    invalid.definitions[0].name.clear();
    let invalid_result = service
        .save_nal_queries(store.shared_agent_connection(), key.into(), invalid, 2)
        .await;
    let invalid_begin_attempts = begin_attempts.load(Ordering::SeqCst);
    let denied_result = service
        .save_nal_queries(
            store.shared_agent_connection(),
            key.into(),
            updated_content.clone(),
            2,
        )
        .await;
    let valid_begin_attempts = begin_attempts.load(Ordering::SeqCst);
    store
        .connection()?
        .authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
    let invalid = invalid_result
        .err()
        .ok_or("invalid query unexpectedly committed")?;
    let denied = denied_result
        .err()
        .ok_or("denied BEGIN unexpectedly succeeded")?;
    if invalid.code != "invalid_input"
        || invalid.message != "查询集合格式无效"
        || invalid.query_outcome != Some(QueryOutcome::NotStarted)
        || invalid_begin_attempts != 0
        || denied.code != "storage_error"
        || denied.query_outcome != Some(QueryOutcome::NotStarted)
        || valid_begin_attempts != 1
        || read(store, service, key).await? != updated
    {
        return Err(format!("native query validation/BEGIN order changed: invalid={invalid:?}; denied={denied:?}; attempts={invalid_begin_attempts}/{valid_begin_attempts}").into());
    }

    store.connection()?.execute_batch("create temp trigger synthetic_query_corrupt after update on nal_query_collections begin update nal_query_collections set definitions='null' where id=1;end;")?;
    let corrupt_result = service
        .save_nal_queries(
            store.shared_agent_connection(),
            key.into(),
            updated_content.clone(),
            2,
        )
        .await;
    store
        .connection()?
        .execute_batch("drop trigger synthetic_query_corrupt;")?;
    let corrupt = corrupt_result
        .err()
        .ok_or("invalid structural readback unexpectedly committed")?;
    if corrupt.code != "invalid_input"
        || corrupt.message != "关系数据结构无效"
        || corrupt.query_outcome != Some(QueryOutcome::RolledBack)
        || corrupt.message.contains("查询集合已变化")
        || read(store, service, key).await? != updated
    {
        return Err(
            format!("query precommit structural decode did not roll back: {corrupt:?}").into(),
        );
    }

    // This is a synthetic legacy row, not a second production save path. Reads
    // retain the original structural-only rule even for invalid save-time names.
    let mut legacy = updated.clone();
    legacy.content.definitions[0].name.clear();
    let original_json = serde_json::to_string(&updated.content.definitions)?;
    store.connection()?.execute(
        "update nal_query_collections set definitions=?1 where id=1",
        [serde_json::to_string(&legacy.content.definitions)?],
    )?;
    let legacy_result = read(store, service, key).await;
    store.connection()?.execute(
        "update nal_query_collections set definitions=?1 where id=1",
        [&original_json],
    )?;
    if legacy_result? != legacy {
        return Err("native query read added semantic validation to an existing legacy row".into());
    }

    updated_content.cache = Some(json!({"syntheticFinal":true,"version":3}));
    let final_stored = service
        .save_nal_queries(
            store.shared_agent_connection(),
            key.into(),
            updated_content.clone(),
            2,
        )
        .await?;
    if final_stored
        != (Stored {
            revision: 3,
            content: updated_content,
        })
        || read(store, service, key).await? != final_stored
    {
        return Err("query retry after confirmed rollback did not save exactly once".into());
    }
    // This typed Core caller can distinguish Some(JSON null) from None even
    // though the existing public Content JSON intentionally cannot.
    let mut null_content = final_stored.content;
    null_content.cache = Some(serde_json::Value::Null);
    let null_stored = service
        .save_nal_queries(
            store.shared_agent_connection(),
            key.into(),
            null_content.clone(),
            3,
        )
        .await?;
    let stored_row = raw_row(store)?;
    if null_stored
        != (Stored {
            revision: 4,
            content: null_content,
        })
        || read(store, service, key).await? != null_stored
        || stored_row.cache_json.as_deref() != Some("null")
        || stored_row.clone().decode()? != null_stored
    {
        return Err("typed Some(JSON null) query save was changed to SQL NULL".into());
    }
    Ok(
        json!({"stored":null_stored,"storedRow":stored_row,"initialRevision":initial.revision,
        "completeStoredCompared":true,"staleCas":stale,"staleDidNotWrite":true,
        "invalidBeforeBegin":invalid,"invalidBeginAttempts":invalid_begin_attempts,
        "validBeginWasDenied":denied,"validBeginAttempts":valid_begin_attempts,
        "structuralDecodeRollback":corrupt,"readOnlyStructuralLegacyRow":true,
        "successfulSaveAfterRollback":true,"typedSomeNullPreserved":true,
        "coreOnly":true}),
    )
}

pub(super) async fn reopen(
    store: &AppStore,
    service: &Arc<LiteratureTraceService>,
    key: &str,
    directory: &Path,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let original: serde_json::Value =
        serde_json::from_slice(&std::fs::read(directory.join("receipt.json"))?)?;
    // The original Stored JSON serializes both cache variants as null. The raw
    // four-column receipt preserves the typed distinction for cold comparison.
    let expected_row: StoredRow = serde_json::from_value(original["storedRow"].clone())?;
    let expected = expected_row.clone().decode()?;
    let actual = read(store, service, key).await?;
    let actual_row = raw_row(store)?;
    if actual != expected
        || actual.revision != 4
        || actual_row != expected_row
        || actual_row.cache_json.as_deref() != Some("null")
        || actual.content.cache != Some(serde_json::Value::Null)
        || serde_json::to_value(&expected)? != original["stored"]
    {
        return Err(
            "cold-reopened Core query collection differs from the committed complete Stored".into(),
        );
    }
    let count: i64 =
        store
            .connection()?
            .query_row("select count(*) from nal_query_collections", [], |row| {
                row.get(0)
            })?;
    if count != 1 {
        return Err("cold-reopened query collection lost its original singleton".into());
    }
    Ok(
        json!({"originalReceipt":"receipt.json","stored":actual,"storedRow":actual_row,
        "sameCompleteStored":true,"sameRawStoredRow":true,"typedSomeNullPreserved":true,
        "singletonRows":count,"seededOrReplayed":false,"coreOnly":true}),
    )
}

fn raw_row(store: &AppStore) -> Result<StoredRow, Box<dyn std::error::Error>> {
    Ok(store.connection()?.query_row(
        "select revision,definitions,cache,legacy_imported from nal_query_collections where id=1",
        [],
        |row| {
            Ok(StoredRow {
                revision: row.get(0)?,
                definitions_json: row.get(1)?,
                cache_json: row.get(2)?,
                legacy_imported: row.get(3)?,
            })
        },
    )?)
}
