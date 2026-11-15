//! Fixed original SQL and file publication; Native waits finish before entry.
use super::{CaptureFailure, CaptureResources};
use crate::{
    runtime_plugins::{
        literature_attachments::{self, AttachmentResources},
        literature_catalog::save::{self, SaveResources},
    },
    storage::AppStore,
};
use research_workbench_native_sdk::{
    browser_capture::*,
    catalog_save::{CatalogSaveOutcome, CatalogSavePlan},
};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde_json::{Value, json};
mod pdf;
mod transaction;

type Result<T> = std::result::Result<T, CaptureFailure>;
#[derive(Clone)]
pub(crate) struct CaptureStorage {
    pub(crate) sql: CaptureResources,
    pub(crate) catalog: SaveResources,
    pub(crate) attachments: AttachmentResources,
}

pub(crate) fn execute(
    store: &AppStore,
    resources: &CaptureStorage,
    scope: &str,
    prepared: BrowserCapturePreparationV1,
    bytes: &[u8],
) -> Result<Value> {
    match prepared {
        BrowserCapturePreparationV1::Capture { plan } => capture(store, resources, scope, *plan),
        BrowserCapturePreparationV1::Attachment { plan } => {
            pdf::save_attachment(store, resources, scope, plan, bytes)
        }
        BrowserCapturePreparationV1::Standalone { plan } => {
            pdf::save_standalone(store, resources, scope, *plan, bytes)
        }
        BrowserCapturePreparationV1::Session {
            session_id,
            projection,
        } => {
            let receipt = session_status(store, &resources.sql, scope, &session_id)?;
            Ok(match projection {
                SessionProjectionV1::Full => receipt,
                SessionProjectionV1::SessionId => json!({"sessionID":session_id}),
                SessionProjectionV1::Null => Value::Null,
            })
        }
        BrowserCapturePreparationV1::Value { value } => Ok(value),
        BrowserCapturePreparationV1::Rejected { reason } => Err(reason.into()),
    }
}

fn capture(
    store: &AppStore,
    resources: &CaptureStorage,
    scope: &str,
    plan: PreparedCaptureV1,
) -> Result<Value> {
    let mut connection = store.connection()?;
    if let Some(receipt) = stored_receipt(
        &connection,
        &resources.sql,
        scope,
        &plan.session_id,
        &plan.request_sha256,
    )? {
        return Ok(receipt);
    }
    transaction::run(&mut connection, |tx| {
        insert_session(
            tx,
            &resources.sql,
            scope,
            &plan.session_id,
            &plan.operation,
            &plan.payload_json,
            &plan.request_sha256,
        )?;
        let mut saved = Vec::with_capacity(plan.items.len());
        for item in plan.items {
            let save = save_plan(item.save)?;
            let id = uuid::Uuid::new_v4().to_string();
            save::save_in_transaction(tx, &resources.catalog, &save, &id)?;
            record_item(
                tx,
                &resources.sql,
                scope,
                &plan.session_id,
                &item.external_id,
                &id,
                &item.raw,
            )?;
            for identifier in item.identifiers {
                let identifier = identifier?;
                tx.execute(
                    resources.sql.sql("identifier-insert"),
                    params![
                        uuid::Uuid::new_v4().to_string(),
                        id,
                        identifier.kind,
                        identifier.normalized,
                        identifier.display
                    ],
                )?;
            }
            save::refresh_search(tx, &resources.catalog, &id)?;
            tx.execute(
                resources.sql.sql("provenance-source"),
                params![id, item.source_reference.to_string()],
            )?;
            saved.push(json!({"id":item.receipt.id,"itemID":id,"itemType":item.receipt.item_type,"title":item.receipt.title,"warnings":item.receipt.warnings}));
        }
        let receipt = json!({"sessionID":plan.session_id,"items":saved});
        finish_session(tx, &resources.sql, scope, &plan.session_id, &receipt)?;
        Ok(receipt)
    })
}

fn save_plan(outcome: CatalogSaveOutcome) -> Result<CatalogSavePlan> {
    match outcome {
        CatalogSaveOutcome::Planned { plan } if plan.item.id.is_none() => Ok(*plan),
        CatalogSaveOutcome::Rejected { .. } => Err(CaptureFailure::InvalidInput),
        _ => Err(CaptureFailure::Technical(
            "browser_capture_target_identity_mismatch".into(),
        )),
    }
}

fn stored_receipt(
    db: &Connection,
    sql: &CaptureResources,
    scope: &str,
    session: &str,
    digest: &str,
) -> Result<Option<Value>> {
    let saved: Option<(String, String)> = db
        .query_row(sql.sql("session-receipt"), params![scope, session], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .optional()?;
    saved
        .map(|(saved_hash, receipt)| {
            if saved_hash != digest {
                return Err(CaptureFailure::Conflict);
            }
            decode_receipt(&receipt)
        })
        .transpose()
}
fn decode_receipt(value: &str) -> Result<Value> {
    serde_json::from_str(value).map_err(|error| {
        CaptureFailure::Technical(format!(
            "capture_receipt_json_invalid; line={}; column={}",
            error.line(),
            error.column()
        ))
    })
}
fn insert_session(
    db: &Connection,
    sql: &CaptureResources,
    scope: &str,
    session: &str,
    operation: &str,
    payload: &str,
    digest: &str,
) -> Result<()> {
    db.execute(
        sql.sql("session-insert"),
        params![scope, session, operation, digest, payload],
    )?;
    Ok(())
}
fn finish_session(
    db: &Connection,
    sql: &CaptureResources,
    scope: &str,
    session: &str,
    receipt: &Value,
) -> Result<()> {
    db.execute(
        sql.sql("session-finish"),
        params![scope, session, receipt.to_string()],
    )?;
    Ok(())
}
fn record_item(
    db: &Connection,
    sql: &CaptureResources,
    scope: &str,
    session: &str,
    external: &str,
    id: &str,
    raw: &Value,
) -> Result<()> {
    db.execute(
        sql.sql("item-insert"),
        params![
            scope,
            session,
            external,
            id,
            raw["itemType"]
                .as_str()
                .ok_or(CaptureFailure::InvalidInput)?,
            raw.to_string()
        ],
    )?;
    Ok(())
}
fn session_status(
    store: &AppStore,
    sql: &CaptureResources,
    scope: &str,
    session: &str,
) -> Result<Value> {
    let connection = store.connection()?;
    let receipt: String = connection
        .query_row(sql.sql("status-receipt"), params![scope, session], |row| {
            row.get(0)
        })
        .optional()?
        .ok_or(CaptureFailure::NotFound)?;
    let attachments = connection
        .prepare(sql.sql("status-attachments"))?
        .query_map(params![scope, session], |row| row.get::<_, String>(0))?
        .map(|row| decode_receipt(&row?))
        .collect::<Result<Vec<_>>>()?;
    Ok(json!({"sessionID":session,"receipt":decode_receipt(&receipt)?,"attachments":attachments}))
}
