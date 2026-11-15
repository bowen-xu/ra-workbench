//! Initial page creation uses the existing note/HBS writers in one transaction.
//! Empty pages are canonical immediately, before an editor is opened.
use crate::{hbs_storage, storage::AppStore};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct DraftCommit {
    commit: hbs_storage::Commit,
    favorite: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    source: Option<super::model::NoteDocumentSaveRequest>,
}

pub(crate) fn publish(
    db: &mut Connection,
    id: &str,
    request: DraftCommit,
) -> Result<Value, String> {
    if id.is_empty()
        || id.len() > 500
        || request.commit.id.is_empty()
        || request.commit.id.len() > 256
        || request.commit.expected_version < 0
    {
        return Err("MODEL_INVALID_REQUEST：新建页面身份或版本无效".into());
    }
    let bytes = serde_json::to_vec(&request).map_err(|e| e.to_string())?;
    if bytes.len() > 64 * 1024 * 1024 {
        return Err("MODEL_REQUEST_TOO_LARGE：新建页面超过容量限制".into());
    }
    let creation = format!("{:x}", Sha256::digest(bytes));
    let fingerprint = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&request.commit).map_err(|e| e.to_string())?)
    );
    let tx = db.transaction().map_err(|e| e.to_string())?;
    let prior: Option<(String, String)> = tx.query_row("select fingerprint,result_json from hbs_commit_receipts where kind='note' and resource_id=?1 and request_id=?2",params![id,request.commit.id],|row|Ok((row.get(0)?,row.get(1)?))).optional().map_err(|e|e.to_string())?;
    if let Some((hash, raw)) = prior {
        let result: Value = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
        if hash != fingerprint || result["creationFingerprint"] != creation {
            return Err("MODEL_REQUEST_ID_REUSED：新建页面请求不能更改内容".into());
        }
        return Ok(result);
    }
    let exists: bool = tx
        .query_row(
            "select exists(select 1 from note_pages where id=?1)",
            [id],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;
    if exists {
        return Err("MODEL_INVALID_RESOURCE：页面身份已存在，新建操作未覆盖原页面".into());
    }
    let document: super::NoteDocument = serde_json::from_value(request.commit.saved.clone())
        .map_err(|e| format!("MODEL_INVALID_DOCUMENT：{e}"))?;
    if document.page_id != id {
        return Err("MODEL_INVALID_DOCUMENT：新建页面身份不匹配".into());
    }
    super::document_validation::validate_document(&document)
        .map_err(|e| format!("MODEL_INVALID_DOCUMENT：{e}"))?;
    let title = document
        .title
        .clone()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "无标题".into());
    super::page_creation::create_page_with_paper_in(
        &tx,
        super::NotePageDraft {
            title,
            parent_page_id: None,
            parent_block_position: None,
        },
        None,
        Some(id),
    )
    .map_err(|e| format!("MODEL_INVALID_DOCUMENT：{e}"))?;
    // Private draft versions may have acknowledged empty edits in memory. The
    // first durable version continues that monotonic sequence, inside this tx.
    tx.execute(
        "update note_pages set revision=?2,is_favorite=?3 where id=?1",
        params![id, request.commit.expected_version, request.favorite],
    )
    .map_err(|e| e.to_string())?;
    let snapshot = super::hbs::commit(
        &tx,
        id,
        request.commit.expected_version,
        request.commit.saved,
        request.commit.baseline,
    )?;
    let source = if let Some(source) = request.source {
        if source.document.page_id == id {
            return Err("MODEL_INVALID_DOCUMENT：新页面不能链接到自身来源".into());
        }
        let source_id = source.document.page_id.clone();
        let before = super::text_change_set::read(&tx, &source_id)
            .map_err(|e| format!("MODEL_SOURCE_SAVE_FAILED：{e}"))?;
        if before.status != "active" {
            return Err("MODEL_SOURCE_SAVE_FAILED：来源页面不是活动笔记".into());
        }
        crate::agent::text_tools::note_review::check_navigation(&tx, &before)
            .map_err(|e| format!("MODEL_SOURCE_SAVE_FAILED：{e}"))?;
        super::document_repository::save_preserving_in(&tx, source, &before.document)
            .map_err(|e| format!("MODEL_SOURCE_SAVE_FAILED：{e}"))?;
        let after = super::text_change_set::read(&tx, &source_id).map_err(|e| e.to_string())?;
        crate::agent::text_tools::note_review::synchronize_navigation(&tx, &before, &after)?;
        Some(
            super::document_repository::load_document_from(&tx, &source_id)
                .map_err(|e| e.to_string())?,
        )
    } else {
        None
    };
    let page =
        super::document_repository::load_document_from(&tx, id).map_err(|e| e.to_string())?;
    let result = json!({"type":"receipt","requestId":request.commit.id,"snapshot":snapshot,"page":page,"source":source,"creationFingerprint":creation});
    tx.execute("insert into hbs_commit_receipts(kind,resource_id,request_id,fingerprint,result_json) values('note',?1,?2,?3,?4)",params![id,request.commit.id,fingerprint,result.to_string()]).map_err(|e|e.to_string())?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(result)
}

#[tauri::command]
pub(crate) async fn publish_note_draft(
    store: tauri::State<'_, AppStore>,
    resource_id: String,
    request: DraftCommit,
) -> Result<Value, String> {
    let store = store.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut db = store.connection().map_err(|e| e.to_string())?;
        publish(&mut db, &resource_id, request)
    })
    .await
    .map_err(|e| format!("MODEL_DRAFT_SAVE_FAILED：{e}"))?
}

#[cfg(test)]
#[path = "draft/tests.rs"]
mod tests;
