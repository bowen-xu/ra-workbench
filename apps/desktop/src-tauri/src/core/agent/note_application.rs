//! Accept a native verified rewrite with one atomic note save and durable receipt.
//! The editor host must serialize this command with its save controller; this
//! command never accepts renderer-provided document bytes or bypasses revisions.
use super::note_review::{self, Error, Scope};
use crate::{
    notes::{
        NoteDocumentSaveRequest,
        agent_context::{NoteContextError, NoteContextRequest, materialize_in},
        load_document_value, save_document_in,
    },
    storage::AppStore,
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Request {
    pub scope: Scope,
    pub request_id: Uuid,
    pub expected_revision: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Receipt {
    pub schema_version: u8,
    pub application_id: String,
    pub conversation_id: String,
    pub page_id: String,
    pub turn_id: String,
    pub item_id: String,
    pub proposal_id: String,
    pub before_revision: i64,
    pub applied_revision: i64,
    pub before_hash: String,
    pub after_hash: String,
    pub created_at: String,
}

#[tauri::command]
pub(crate) async fn agent_accept_note_rewrite(
    store: tauri::State<'_, AppStore>,
    request: Request,
) -> Result<Receipt, Error> {
    accept(&store, request).await
}

#[tauri::command]
pub(crate) async fn agent_note_application_status(
    store: tauri::State<'_, AppStore>,
    request: Scope,
) -> Result<Option<Receipt>, Error> {
    status(&store, request).await
}

pub(super) fn valid(scope: Scope) -> Result<(), Error> {
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

pub(super) fn read(db: &Connection, scope: Scope) -> Result<Option<Receipt>, Error> {
    let result = db
        .query_row(
            "select application_id,conversation_id,page_id,turn_id,item_id,proposal_id,
         before_revision,applied_revision,before_hash,after_hash,created_at
         from agent_note_applications where conversation_id=?1 and item_id=?2",
            params![scope.conversation_id.to_string(), scope.item_id.to_string()],
            |r| {
                Ok(Receipt {
                    schema_version: 1,
                    application_id: r.get(0)?,
                    conversation_id: r.get(1)?,
                    page_id: r.get(2)?,
                    turn_id: r.get(3)?,
                    item_id: r.get(4)?,
                    proposal_id: r.get(5)?,
                    before_revision: r.get(6)?,
                    applied_revision: r.get(7)?,
                    before_hash: r.get(8)?,
                    after_hash: r.get(9)?,
                    created_at: r.get(10)?,
                })
            },
        )
        .optional()
        .map_err(|_| Error::Unavailable)?;
    if result.as_ref().is_some_and(|r| {
        r.page_id != scope.page_id.to_string() || r.turn_id != scope.turn_id.to_string()
    }) {
        return Err(Error::Corrupt);
    }
    Ok(result)
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

pub(crate) async fn accept(store: &AppStore, request: Request) -> Result<Receipt, Error> {
    valid(request.scope)?;
    if request.request_id.is_nil()
        || !(0..9_007_199_254_740_991).contains(&request.expected_revision)
    {
        return Err(Error::InvalidInput);
    }
    // Review replays the bounded journal and recomputes the original tool result.
    // It does not grant a lease on the page; all live checks are repeated below.
    let review = note_review::review(store, request.scope).await?;
    let owned = store.clone();
    tokio::task::spawn_blocking(move || {
        let scope=request.scope;
        let proposal_id=review.proposal["proposalId"].as_str().ok_or(Error::Corrupt)?;
        let mut db=owned.connection().map_err(|_|Error::Unavailable)?;
        let tx=db.transaction_with_behavior(TransactionBehavior::Immediate).map_err(|_|Error::Unavailable)?;
        let record=note_review::bound_record(&tx,scope)?;
        if review.proposal["contextSnapshotId"]!=record.context_snapshot_id.to_string() {
            return Err(Error::Corrupt);
        }
        let collision:bool=tx.query_row(
            "select exists(select 1 from agent_note_applications where application_id=?1
             and (conversation_id<>?2 or page_id<>?3 or turn_id<>?4 or item_id<>?5))",
            params![request.request_id.to_string(),scope.conversation_id.to_string(),scope.page_id.to_string(),
                scope.turn_id.to_string(),scope.item_id.to_string()],|r|r.get(0),
        ).map_err(|_|Error::Unavailable)?;
        if collision { return Err(Error::InvalidInput); }
        if let Some(saved)=read(&tx,scope)? {
            if saved.proposal_id!=proposal_id { return Err(Error::Corrupt); }
            if saved.before_revision!=request.expected_revision { return Err(Error::Conflict); }
            // An acknowledgement is historical, never permission to overwrite a
            // newer editor buffer with the applied version on recovery.
            return Ok(saved);
        }
        let rejected:bool=tx.query_row(
            "select exists(select 1 from agent_note_rejections where conversation_id=?1 and item_id=?2)",
            params![scope.conversation_id.to_string(),scope.item_id.to_string()],|r|r.get(0),
        ).map_err(|_|Error::Unavailable)?;
        if rejected { return Err(Error::Decided); }
        if !review.baseline_matches || review.current_revision!=request.expected_revision {
            return Err(Error::Conflict);
        }
        let full=materialize_in(&tx,&NoteContextRequest {
            page_id:scope.page_id,expected_revision:request.expected_revision,block_id:None,
        }).map_err(|e|match e {
            NoteContextError::Conflict=>Error::Conflict,NoteContextError::Budget=>Error::Budget,
            _=>Error::Unavailable,
        })?;
        let candidate=note_review::candidate_document(&record,&review.proposal,&full)?;
        let before=load_document_value(&tx,&scope.page_id.to_string()).map_err(|_|Error::Unavailable)?;
        // Body proposals do not write title fields. Hash the resulting formal
        // snapshot with the untouched authoritative title, including raw spaces.
        let mut projected=candidate.clone();
        projected.title.clone_from(&before.title);
        let before_hash=document_hash(&before)?;
        let after_hash=document_hash(&projected)?;
        let saved=save_document_in(&tx,NoteDocumentSaveRequest {
            document:candidate,expected_revision:request.expected_revision,save_reason:"manual".into(),
        }).map_err(|_|Error::Unavailable)?;
        if saved.page.revision!=request.expected_revision+1 || document_hash(&saved.document)?!=after_hash {
            return Err(Error::Corrupt);
        }
        tx.execute(
            "insert into agent_note_applications(application_id,conversation_id,page_id,turn_id,item_id,
             proposal_id,before_revision,applied_revision,before_hash,after_hash) values(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
            params![request.request_id.to_string(),scope.conversation_id.to_string(),scope.page_id.to_string(),
                scope.turn_id.to_string(),scope.item_id.to_string(),proposal_id,request.expected_revision,
                saved.page.revision,before_hash,after_hash],
        ).map_err(|_|Error::Unavailable)?;
        let receipt=read(&tx,scope)?.ok_or(Error::Unavailable)?;
        tx.commit().map_err(|_|Error::Unavailable)?;
        Ok(receipt)
    }).await.map_err(|_|Error::Unavailable)?
}

pub(super) fn document_hash(document: &crate::notes::NoteDocument) -> Result<String, Error> {
    let bytes = serde_json::to_vec(document).map_err(|_| Error::Corrupt)?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}
