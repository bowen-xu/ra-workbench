//! Invert one verified, applied inline rewrite without overwriting later edits.
//! Content and the immutable undo receipt share the Notes save transaction.
use super::{
    note_application,
    note_review::{self, Error, Scope},
};
use crate::{
    notes::{
        NoteDocument, NoteDocumentBlock, NoteDocumentSaveRequest,
        agent_context::{NoteContextRequest, materialize_in},
        load_document_value, save_document_in,
    },
    storage::AppStore,
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Request {
    pub scope: Scope,
    pub application_id: Uuid,
    pub request_id: Uuid,
    pub expected_revision: i64,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Receipt {
    pub schema_version: u8,
    pub undo_id: String,
    pub application_id: String,
    pub conversation_id: String,
    pub page_id: String,
    pub turn_id: String,
    pub item_id: String,
    pub proposal_id: String,
    pub before_revision: i64,
    pub restored_revision: i64,
    pub before_hash: String,
    pub restored_hash: String,
    pub created_at: String,
}

#[tauri::command]
pub(crate) async fn agent_undo_note_rewrite(
    store: tauri::State<'_, AppStore>,
    request: Request,
) -> Result<Receipt, Error> {
    undo(&store, request).await
}
#[tauri::command]
pub(crate) async fn agent_note_undo_status(
    store: tauri::State<'_, AppStore>,
    request: Scope,
) -> Result<Option<Receipt>, Error> {
    status(&store, request).await
}
fn read(
    db: &Connection,
    application: &note_application::Receipt,
) -> Result<Option<Receipt>, Error> {
    db.query_row(
        "select undo_id,before_revision,restored_revision,before_hash,restored_hash,created_at
        from agent_note_undos where application_id=?1",
        [&application.application_id],
        |r| {
            Ok(Receipt {
                schema_version: 1,
                undo_id: r.get(0)?,
                application_id: application.application_id.clone(),
                conversation_id: application.conversation_id.clone(),
                page_id: application.page_id.clone(),
                turn_id: application.turn_id.clone(),
                item_id: application.item_id.clone(),
                proposal_id: application.proposal_id.clone(),
                before_revision: r.get(1)?,
                restored_revision: r.get(2)?,
                before_hash: r.get(3)?,
                restored_hash: r.get(4)?,
                created_at: r.get(5)?,
            })
        },
    )
    .optional()
    .map_err(|_| Error::Unavailable)
}
pub(crate) async fn status(store: &AppStore, scope: Scope) -> Result<Option<Receipt>, Error> {
    note_application::valid(scope)?;
    let owned = store.clone();
    tokio::task::spawn_blocking(move || {
        let mut db = owned.connection().map_err(|_| Error::Unavailable)?;
        let tx = db.transaction().map_err(|_| Error::Unavailable)?;
        note_review::bound_record(&tx, scope)?;
        match note_application::read(&tx, scope)? {
            Some(application) => read(&tx, &application),
            None => Ok(None),
        }
    })
    .await
    .map_err(|_| Error::Unavailable)?
}
pub(crate) async fn undo(store: &AppStore, request: Request) -> Result<Receipt, Error> {
    note_application::valid(request.scope)?;
    if request.request_id.is_nil()
        || request.application_id.is_nil()
        || !(1..9_007_199_254_740_991).contains(&request.expected_revision)
    {
        return Err(Error::InvalidInput);
    }
    // Recompute original edits from the bounded immutable journal. Never accept
    // an inverse document supplied by the renderer or infer one from prose.
    let (record, proposal) = note_review::verified_record(store, request.scope).await?;
    let owned = store.clone();
    tokio::task::spawn_blocking(move || {
        let scope=request.scope;
        let mut db=owned.connection().map_err(|_|Error::Unavailable)?;
        let tx=db.transaction_with_behavior(TransactionBehavior::Immediate).map_err(|_|Error::Unavailable)?;
        if note_review::bound_record(&tx,scope)?!=record { return Err(Error::Corrupt); }
        let application=note_application::read(&tx,scope)?.ok_or(Error::Unavailable)?;
        if application.application_id!=request.application_id.to_string() { return Err(Error::InvalidInput); }
        if proposal["proposalId"]!=application.proposal_id || proposal["contextSnapshotId"]!=record.context_snapshot_id.to_string() {
            return Err(Error::Corrupt);
        }
        if application.applied_revision!=request.expected_revision { return Err(Error::Conflict); }
        let collision:bool=tx.query_row("select exists(select 1 from agent_note_undos where undo_id=?1 and application_id<>?2)",
            params![request.request_id.to_string(),application.application_id],|r|r.get(0)).map_err(|_|Error::Unavailable)?;
        if collision { return Err(Error::InvalidInput); }
        // An old acknowledgement is returned even after later manual edits, but
        // does not authorize reinstalling its historical body in the editor.
        if let Some(receipt)=read(&tx,&application)? { return Ok(receipt); }
        materialize_in(&tx,&NoteContextRequest { page_id:scope.page_id,expected_revision:request.expected_revision,block_id:None })
            .map_err(|e|match e { crate::notes::agent_context::NoteContextError::Conflict=>Error::Conflict,
                crate::notes::agent_context::NoteContextError::Budget=>Error::Budget,_=>Error::Unavailable })?;
        let current=load_document_value(&tx,&scope.page_id.to_string()).map_err(|_|Error::Unavailable)?;
        if note_application::document_hash(&current)?!=application.after_hash { return Err(Error::Conflict); }
        let candidate=inverse(current,&proposal)?;
        if note_application::document_hash(&candidate)?!=application.before_hash { return Err(Error::Corrupt); }
        // This inverse owns body edits only; the verified current title remains
        // outside its write set, including on deployed body-only schemas.
        let mut body_candidate=candidate;
        body_candidate.title=None;
        let saved=save_document_in(&tx,NoteDocumentSaveRequest { document:body_candidate,expected_revision:request.expected_revision,save_reason:"manual".into() })
            .map_err(|_|Error::Unavailable)?;
        if saved.page.revision!=request.expected_revision+1 || note_application::document_hash(&saved.document)?!=application.before_hash {
            return Err(Error::Corrupt);
        }
        tx.execute("insert into agent_note_undos(undo_id,application_id,before_revision,restored_revision,before_hash,restored_hash)
            values(?1,?2,?3,?4,?5,?6)", params![request.request_id.to_string(),application.application_id,
            request.expected_revision,saved.page.revision,application.after_hash,application.before_hash]).map_err(|_|Error::Unavailable)?;
        let receipt=read(&tx,&application)?.ok_or(Error::Unavailable)?;
        tx.commit().map_err(|_|Error::Unavailable)?;
        Ok(receipt)
    }).await.map_err(|_|Error::Unavailable)?
}
fn inverse(mut document: NoteDocument, proposal: &Value) -> Result<NoteDocument, Error> {
    fn find<'a>(
        blocks: &'a mut [NoteDocumentBlock],
        id: &str,
    ) -> Option<&'a mut NoteDocumentBlock> {
        blocks.iter_mut().find_map(|block| {
            if block.id == id {
                Some(block)
            } else {
                find(&mut block.children, id)
            }
        })
    }
    for edit in proposal["edits"].as_array().ok_or(Error::Corrupt)? {
        let block = find(
            &mut document.blocks,
            edit["blockId"].as_str().ok_or(Error::Corrupt)?,
        )
        .ok_or(Error::Corrupt)?;
        if block.content != edit["after"] {
            return Err(Error::Corrupt);
        }
        block.content = edit["before"].clone();
    }
    Ok(document)
}
