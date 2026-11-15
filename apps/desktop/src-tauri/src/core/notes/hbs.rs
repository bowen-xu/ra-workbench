//! Typed HBS storage adapter. The same note save operation owns block, link,
//! attachment and revision updates; HBS only adds an independent baseline.
use super::{NoteDocument, document_repository, model::NoteDocumentSaveRequest};
use rusqlite::{OptionalExtension, Transaction, params};
use serde_json::{Value, json};
#[cfg(test)]
#[path = "hbs_title_tests.rs"]
mod title_tests;
fn storage_error(error: super::error::NotesError) -> String {
    use super::error::NotesError;
    let code = match &error {
        NotesError::NotFound => "MODEL_NOT_FOUND",
        NotesError::RevisionConflict { .. } => "MODEL_VERSION_CONFLICT",
        NotesError::InvalidInput(_) => "MODEL_INVALID_DOCUMENT",
        _ => "MODEL_STORAGE_ERROR",
    };
    format!("{code}：{error}")
}

pub(crate) fn read(db: &rusqlite::Connection, id: &str) -> Result<Value, String> {
    let current = document_repository::load_document_from(db, id).map_err(storage_error)?;
    let baseline: Option<String> = db
        .query_row(
            "select baseline_json from hbs_note_baselines where page_id=?1",
            [id],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let baseline = baseline
        .map(|raw| serde_json::from_str::<NoteDocument>(&raw))
        .transpose()
        .map_err(|e| e.to_string())?;
    let legacy = if baseline
        .as_ref()
        .is_none_or(|document| document.title.is_none())
    {
        crate::agent::text_tools::note_review::legacy_hbs_baseline(db, id)?
    } else {
        None
    };
    let mut baseline = match baseline {
        Some(baseline) => baseline,
        None => legacy.clone().unwrap_or_else(|| current.document.clone()),
    };
    if baseline.title.is_none() {
        baseline.title = legacy
            .and_then(|document| document.title)
            .or_else(|| Some(current.page.title.clone()));
    }
    Ok(json!({"version":current.page.revision,"saved":current.document,"baseline":baseline}))
}
pub(crate) fn commit(
    tx: &Transaction<'_>,
    id: &str,
    expected: i64,
    saved: Value,
    baseline: Value,
) -> Result<Value, String> {
    let mut saved: NoteDocument =
        serde_json::from_value(saved).map_err(|e| format!("MODEL_INVALID_DOCUMENT：{e}"))?;
    let mut baseline: NoteDocument =
        serde_json::from_value(baseline).map_err(|e| format!("MODEL_INVALID_DOCUMENT：{e}"))?;
    if saved.page_id != id || baseline.page_id != id {
        return Err("MODEL_INVALID_DOCUMENT：页面身份不能改变".into());
    }
    super::document_validation::validate_document(&baseline)
        .map_err(|e| format!("MODEL_INVALID_DOCUMENT：{e}"))?;
    let current = document_repository::load_document_from(tx, id).map_err(storage_error)?;
    if saved.title.is_none() {
        saved.title = current.document.title.clone();
    }
    if baseline.title.is_none() {
        let existing = read(tx, id)?;
        baseline.title = existing["baseline"]["title"].as_str().map(str::to_owned);
    }
    if current.page.status != "active" {
        return Err("MODEL_RESOURCE_INACTIVE：非活动笔记不能提交正文或审阅决定".into());
    }
    let before = super::text_change_set::read(tx, id).map_err(storage_error)?;
    if current.page.revision != expected {
        return Err("MODEL_VERSION_CONFLICT：笔记已更新".into());
    }
    if serde_json::to_value(&current.document).ok() != serde_json::to_value(&saved).ok() {
        document_repository::save_preserving_in(
            tx,
            NoteDocumentSaveRequest {
                document: saved,
                expected_revision: expected,
                save_reason: "manual".into(),
            },
            &current.document,
        )
        .map_err(storage_error)?;
    } else {
        tx.execute("update note_pages set revision=revision+1,updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') where id=?1 and revision=?2",params![id,expected]).map_err(|e| e.to_string())?;
    }
    store_baseline(tx, id, &baseline)?;
    let after = super::text_change_set::read(tx, id).map_err(storage_error)?;
    crate::agent::text_tools::note_review::synchronize_hbs_commit(tx, &before, &after, &baseline)?;
    read(tx, id)
}
pub(crate) fn store_baseline(
    db: &rusqlite::Connection,
    id: &str,
    baseline: &NoteDocument,
) -> Result<(), String> {
    let raw = serde_json::to_string(baseline).map_err(|e| e.to_string())?;
    db.execute("insert into hbs_note_baselines(page_id,baseline_json) values(?1,?2) on conflict(page_id) do update set baseline_json=excluded.baseline_json",params![id,raw]).map_err(|e| e.to_string())?;
    db.execute(
        "delete from hbs_note_baseline_assets where page_id=?1",
        [id],
    )
    .map_err(|e| e.to_string())?;
    db.execute("insert into hbs_note_baseline_assets(page_id,content_sha256) select ?1,cast(value as text) from json_tree(?2) where key='assetId' group by value",params![id,raw]).map_err(|e|e.to_string())?;
    Ok(())
}
/// An explicit metadata rename edits that field in B without accepting body
/// candidates. S is already updated in the caller's page transaction.
pub(crate) fn synchronize_title(
    db: &rusqlite::Connection,
    id: &str,
    title: &str,
) -> Result<(), String> {
    let initialized: bool = db.query_row("select exists(select 1 from sqlite_master where type='table' and name='hbs_note_baselines')", [], |row| row.get(0)).map_err(|e| e.to_string())?;
    if !initialized {
        return Ok(());
    }
    let snapshot = read(db, id)?;
    let mut baseline: NoteDocument =
        serde_json::from_value(snapshot["baseline"].clone()).map_err(|e| e.to_string())?;
    baseline.title = Some(title.into());
    store_baseline(db, id, &baseline)
}
/// Older isolated domain fixtures predate HBS; fully migrated app databases
/// always have this table. Agent checkpoints must update B atomically with S.
pub(crate) fn synchronize_baseline_if_initialized(
    db: &rusqlite::Connection,
    id: &str,
    baseline: &NoteDocument,
) -> Result<(), String> {
    let initialized: bool = db.query_row("select exists(select 1 from sqlite_master where type='table' and name='hbs_note_baselines')", [], |row| row.get(0)).map_err(|e| e.to_string())?;
    if initialized {
        store_baseline(db, id, baseline)?;
    }
    Ok(())
}
pub(crate) fn synchronize_navigation_if_settled(
    db: &rusqlite::Connection,
    before: &super::text_change_set::PageState,
    after: &super::text_change_set::PageState,
) -> Result<(), String> {
    let initialized: bool = db.query_row("select exists(select 1 from sqlite_master where type='table' and name='hbs_note_baselines')", [], |row| row.get(0)).map_err(|e| e.to_string())?;
    if !initialized {
        return Ok(());
    }
    let baseline: Option<String> = db
        .query_row(
            "select baseline_json from hbs_note_baselines where page_id=?1",
            [&before.page_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let baseline = baseline
        .map(|raw| serde_json::from_str::<Value>(&raw).map_err(|e| e.to_string()))
        .transpose()?;
    // A navigation writer does not own outstanding body review decisions.
    // Only a settled baseline may follow the explicit ordinary structure edit.
    let mut before_document = before.document.clone();
    if before_document.title.is_none() {
        before_document.title = Some(before.title.clone());
    }
    if baseline.is_none()
        || baseline == Some(serde_json::to_value(&before_document).map_err(|e| e.to_string())?)
    {
        let mut document = after.document.clone();
        if document.title.is_none() {
            document.title = before_document.title.clone();
        }
        store_baseline(db, &after.page_id, &document)?;
    }
    Ok(())
}
#[cfg(any(test, feature = "development-data"))]
pub(crate) fn seed(
    db: &mut rusqlite::Connection,
    id: &str,
    mut document: NoteDocument,
) -> Result<Value, String> {
    if let Ok(value) = read(db, id) {
        return Ok(value);
    }
    let tx = db.transaction().map_err(|e| e.to_string())?;
    super::page_creation::create_page_with_paper_in(
        &tx,
        super::model::NotePageDraft {
            title: "HBS 结构笔记（合成数据）".into(),
            parent_page_id: None,
            parent_block_position: None,
        },
        None,
        Some(id),
    )
    .map_err(|e| e.to_string())?;
    document.page_id = id.into();
    let value = serde_json::to_value(document).map_err(|e| e.to_string())?;
    let result = commit(&tx, id, 0, value.clone(), value)?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(result)
}
