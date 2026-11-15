//! Durable user decisions over verified tool results. Never edits note content.
use super::{
    note_review::{self, Error, Scope},
    repository::SqliteConversationCatalog,
};
use crate::storage::AppStore;
use research_agent_core::conversation::ConversationId;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Request {
    pub request_id: Uuid,
    pub scope: Scope,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Receipt {
    pub schema_version: u8,
    pub decision_id: String,
    pub conversation_id: String,
    pub page_id: String,
    pub turn_id: String,
    pub item_id: String,
    pub proposal_id: String,
    pub created_at: String,
}

#[tauri::command]
pub(crate) async fn agent_reject_note_rewrite(
    store: tauri::State<'_, AppStore>,
    request: Request,
) -> Result<Receipt, Error> {
    reject(&store, request).await
}
#[tauri::command]
pub(crate) async fn agent_note_rejection_status(
    store: tauri::State<'_, AppStore>,
    request: Scope,
) -> Result<Option<Receipt>, Error> {
    status(&store, request).await
}

fn valid(scope: Scope) -> Result<(), Error> {
    if [
        scope.conversation_id,
        scope.page_id,
        scope.turn_id,
        scope.item_id,
    ]
    .iter()
    .any(Uuid::is_nil)
    {
        return Err(Error::InvalidInput);
    }
    Ok(())
}
fn read(db: &Connection, scope: Scope) -> Result<Option<Receipt>, Error> {
    let saved = db.query_row("select decision_id,conversation_id,page_id,turn_id,item_id,proposal_id,created_at from agent_note_rejections where conversation_id=?1 and item_id=?2",
        params![scope.conversation_id.to_string(),scope.item_id.to_string()], |r|Ok(Receipt {
            schema_version:1,
            decision_id:r.get(0)?,conversation_id:r.get(1)?,page_id:r.get(2)?,turn_id:r.get(3)?,item_id:r.get(4)?,proposal_id:r.get(5)?,created_at:r.get(6)?,
        })).optional().map_err(|_|Error::Unavailable)?;
    if saved.as_ref().is_some_and(|s| {
        s.page_id != scope.page_id.to_string() || s.turn_id != scope.turn_id.to_string()
    }) {
        return Err(Error::Corrupt);
    }
    Ok(saved)
}
pub(crate) async fn status(store: &AppStore, scope: Scope) -> Result<Option<Receipt>, Error> {
    valid(scope)?;
    let owned = store.clone();
    tokio::task::spawn_blocking(move || {
        let mut db = owned.connection().map_err(|_| Error::Unavailable)?;
        let tx = db.transaction().map_err(|_| Error::Unavailable)?;
        note_review::bound_record(&tx, scope)?;
        read(&tx, scope)
    })
    .await
    .map_err(|_| Error::Unavailable)?
}
pub(crate) async fn reject(store: &AppStore, request: Request) -> Result<Receipt, Error> {
    valid(request.scope)?;
    if request.request_id.is_nil() {
        return Err(Error::InvalidInput);
    }
    let scope = request.scope;
    let owned = store.clone();
    let record = tokio::task::spawn_blocking(move || {
        let mut db = owned.connection().map_err(|_| Error::Unavailable)?;
        let tx = db.transaction().map_err(|_| Error::Unavailable)?;
        note_review::bound_record(&tx, scope)
    })
    .await
    .map_err(|_| Error::Unavailable)??;
    let (_, projection) = SqliteConversationCatalog::from_app_store(store)
        .projection_snapshot(ConversationId::from_uuid(scope.conversation_id))
        .await
        .map_err(|e| match e {
            super::repository::SnapshotExportError::TooLarge => Error::Budget,
            super::repository::SnapshotExportError::Corrupt => Error::Corrupt,
            _ => Error::Unavailable,
        })?;
    let owned = store.clone();
    tokio::task::spawn_blocking(move || {
        let proposal=note_review::verified_proposal(&record,&projection,scope)?;
        let proposal_id=proposal["proposalId"].as_str().ok_or(Error::Corrupt)?;
        let mut db=owned.connection().map_err(|_|Error::Unavailable)?;
        let tx=db.transaction_with_behavior(TransactionBehavior::Immediate).map_err(|_|Error::Unavailable)?;
        if note_review::bound_record(&tx,scope)? != record { return Err(Error::Corrupt); }
        if super::note_application::read(&tx,scope)?.is_some() { return Err(Error::Decided); }
        // A request identity cannot be repurposed for another item or scope.
        let collision:bool=tx.query_row("select exists(select 1 from agent_note_rejections where decision_id=?1 and (conversation_id<>?2 or page_id<>?3 or turn_id<>?4 or item_id<>?5))",
            params![request.request_id.to_string(),scope.conversation_id.to_string(),scope.page_id.to_string(),scope.turn_id.to_string(),scope.item_id.to_string()],|r|r.get(0)).map_err(|_|Error::Unavailable)?;
        if collision { return Err(Error::InvalidInput); }
        if let Some(saved)=read(&tx,scope)? {
            if saved.proposal_id!=proposal_id { return Err(Error::Corrupt); }
            return Ok(saved);
        }
        tx.execute("insert into agent_note_rejections(decision_id,conversation_id,page_id,turn_id,item_id,proposal_id) values(?1,?2,?3,?4,?5,?6)",
            params![request.request_id.to_string(),scope.conversation_id.to_string(),scope.page_id.to_string(),scope.turn_id.to_string(),scope.item_id.to_string(),proposal_id]).map_err(|_|Error::Unavailable)?;
        let receipt=read(&tx,scope)?.ok_or(Error::Unavailable)?;
        tx.commit().map_err(|_|Error::Unavailable)?;
        Ok(receipt)
    }).await.map_err(|_|Error::Unavailable)?
}
