//! User conflict resolution owns body, copy, stale-review retirement and receipt
//! in one transaction. Immutable Agent snapshots/receipts remain historical data.
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{
    document_repository::{load_document_from, save_document_in},
    error::{NotesCommandError, NotesError},
    model::{
        NoteDocument, NoteDocumentBlock, NoteDocumentSaveRequest, NoteDocumentSnapshot,
        NotePageDraft,
    },
};

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Request {
    request_id: Uuid,
    pub(super) document: NoteDocument,
    expected_revision: i64,
    copy: bool,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Receipt {
    original: NoteDocumentSnapshot,
    #[serde(skip_serializing_if = "Option::is_none")]
    copy: Option<NoteDocumentSnapshot>,
}
fn encode(value: &impl Serialize) -> Result<String, NotesError> {
    serde_json::to_string(value).map_err(|error| NotesError::InvalidInput(error.to_string()))
}

pub(super) fn resolve(
    connection: &mut Connection,
    request: Request,
) -> Result<Receipt, NotesError> {
    if request.request_id.is_nil() {
        return Err(NotesError::InvalidInput("冲突操作 ID 无效".into()));
    }
    let tx = connection.transaction()?;
    let encoded = encode(&request)?;
    let previous: Option<(String, String)> = tx
        .query_row(
            "select request_json,result_json from note_conflict_resolutions where request_id=?1",
            [request.request_id.to_string()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    if let Some((input, result)) = previous {
        if input != encoded {
            return Err(NotesError::InvalidInput(
                "冲突操作 ID 已用于不同内容".into(),
            ));
        }
        return serde_json::from_str(&result)
            .map_err(|error| NotesError::InvalidInput(error.to_string()));
    }
    let page_id = request.document.page_id.clone();
    let current = load_document_from(&tx, &page_id)?;
    if current.page.revision != request.expected_revision {
        return Err(NotesError::RevisionConflict {
            current_revision: current.page.revision,
        });
    }
    // Running original decisions must be reconciled before a new user choice.
    let running: bool = tx.query_row("select exists(select 1 from agent_note_decision_requests r join agent_note_patch_candidates p on p.conversation_id=r.conversation_id where p.page_id=?1 and r.state='running')", [&page_id], |row| row.get(0))?;
    if running {
        return Err(NotesError::InvalidInput(
            "原审阅操作尚未确认，请先核对原操作".into(),
        ));
    }
    let copy = if request.copy {
        let id = super::create_page_with_paper_in(
            &tx,
            NotePageDraft {
                title: format!(
                    "{}（冲突副本）",
                    current.page.title.chars().take(230).collect::<String>()
                ),
                // Top-level copies do not mutate the source's parent while resolving.
                parent_page_id: None,
                parent_block_position: None,
            },
            None,
            None,
        )?;
        let mut document = request.document.clone();
        document.page_id = id;
        document.title = None; // Preserve the new conflict copy's generated name.
        remap(&mut document.blocks);
        Some(save_document_in(
            &tx,
            NoteDocumentSaveRequest {
                document,
                expected_revision: 0,
                save_reason: "structure".into(),
            },
        )?)
    } else {
        None
    };
    let document = if request.copy {
        current.document
    } else {
        request.document
    };
    let original = save_document_in(
        &tx,
        NoteDocumentSaveRequest {
            document,
            expected_revision: request.expected_revision,
            save_reason: "manual".into(),
        },
    )?;
    retire_reviews(&tx, &page_id)?;
    let receipt = Receipt { original, copy };
    tx.execute("insert into note_conflict_resolutions(request_id,page_id,request_json,result_json) values(?1,?2,?3,?4)", params![request.request_id.to_string(), page_id, encoded, encode(&receipt)?])?;
    tx.commit()?;
    Ok(receipt)
}

pub(super) fn retire_reviews(
    tx: &rusqlite::Transaction<'_>,
    page_id: &str,
) -> Result<(), NotesError> {
    // Invalidate old group credentials while leaving other pages pending.
    tx.execute("update agent_note_patch_candidates set generation=generation+1 where (conversation_id,proposal_id) in (select conversation_id,proposal_id from agent_note_patch_candidates where page_id=?1 and state in ('pending','accepted'))", [page_id])?;
    tx.execute("update agent_note_patch_candidates set state='rejected' where page_id=?1 and state in ('pending','accepted')", [page_id])?;
    Ok(())
}
fn remap(blocks: &mut [NoteDocumentBlock]) {
    for block in blocks {
        block.id = Uuid::new_v4().to_string();
        if block.block_type == "childPage" {
            block.block_type = "researchObjectReference".into();
        }
        remap(&mut block.children);
    }
}

#[tauri::command]
pub fn resolve_note_conflict(
    store: tauri::State<'_, crate::storage::AppStore>,
    request: Request,
) -> Result<Receipt, NotesCommandError> {
    let mut db = store.connection().map_err(NotesError::from)?;
    resolve(&mut db, request).map_err(Into::into)
}

#[tauri::command]
pub fn load_note_revision(
    store: tauri::State<'_, crate::storage::AppStore>,
    page_id: String,
    revision_id: String,
) -> Result<NoteDocument, NotesCommandError> {
    let db = store.connection().map_err(NotesError::from)?;
    revision(&db, &page_id, &revision_id).map_err(Into::into)
}
pub(super) fn revision(
    db: &Connection,
    page_id: &str,
    revision_id: &str,
) -> Result<NoteDocument, NotesError> {
    let json: String = db
        .query_row(
            "select document_json from note_revisions where page_id=?1 and id=?2",
            params![page_id, revision_id],
            |row| row.get(0),
        )
        .optional()?
        .ok_or(NotesError::NotFound)?;
    serde_json::from_str(&json).map_err(|error| NotesError::InvalidInput(error.to_string()))
}
