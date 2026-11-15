//! Versioned reading aids are separate from human annotations and AI conversations.
use super::error::{AiCommandError, AiError};
use crate::storage::AppStore;
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tauri::State;

fn text(value: &Value, field: &str, limit: usize, empty: bool) -> bool {
    value
        .get(field)
        .and_then(Value::as_str)
        .is_some_and(|s| s.chars().count() <= limit && (empty || !s.trim().is_empty()))
}
fn validate(draft: &Value) -> Result<(&str, &str), AiError> {
    let invalid = || AiError::InvalidInput("词义解释或来源位置无效。".into());
    let attachment = draft
        .get("attachmentId")
        .and_then(Value::as_str)
        .ok_or_else(invalid)?;
    let hash = draft
        .get("contentSha256")
        .and_then(Value::as_str)
        .ok_or_else(invalid)?;
    let context = &draft["context"];
    let explanation = &draft["explanation"];
    let segments = draft["segments"].as_array().ok_or_else(invalid)?;
    if attachment.is_empty()
        || attachment.len() > 128
        || hash.len() != 64
        || !hash
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        || !text(context, "word", 100, false)
        || !text(context, "before", 600, true)
        || !text(context, "after", 600, true)
        || !text(explanation, "translation", 120, false)
        || !text(explanation, "explanation", 1000, false)
        || !text(explanation, "providerId", 240, false)
        || !text(explanation, "modelId", 240, false)
        || (explanation.get("modelRevision").is_some()
            && !text(explanation, "modelRevision", 240, true))
        || segments.is_empty()
        || segments.len() > 2
        || segments.iter().any(|segment| {
            let Some(page) = segment["pageIndex"].as_u64() else {
                return true;
            };
            let Some(rects) = segment["rects"].as_array() else {
                return true;
            };
            page > 100_000
                || !text(segment, "text", 100, false)
                || !text(segment, "prefix", 64, true)
                || !text(segment, "suffix", 64, true)
                || rects.is_empty()
                || rects.len() > 16
                || rects.iter().any(|rect| {
                    let numbers = ["x", "y", "width", "height"].map(|key| rect[key].as_f64());
                    if numbers.iter().any(|number| {
                        number.is_none_or(|n| !n.is_finite() || !(0.0..=1.0).contains(&n))
                    }) {
                        return true;
                    }
                    let [Some(x), Some(y), Some(width), Some(height)] = numbers else {
                        return true;
                    };
                    width <= 0.0 || height <= 0.0 || x + width > 1.001 || y + height > 1.001
                })
        })
    {
        return Err(invalid());
    }
    if serde_json::to_vec(draft).map_err(|_| invalid())?.len() > 20_000 {
        return Err(invalid());
    }
    Ok((attachment, hash))
}

fn save(connection: &Connection, draft: Value) -> Result<Value, AiError> {
    let (attachment, hash) = validate(&draft)?;
    let current: Option<String> = connection
        .query_row(
            "select content_sha256 from attachments where id = ?1",
            [attachment],
            |row| row.get(0),
        )
        .optional()?;
    if current.as_deref() != Some(hash) {
        return Err(AiError::InvalidInput(
            "PDF 版本已改变，未将解释保存到其他附件。".into(),
        ));
    }
    let id = format!(
        "{:x}",
        Sha256::digest(format!("{attachment}:{hash}:{}", draft["segments"]).as_bytes())
    );
    let mut result = draft.clone();
    result["id"] = Value::String(id.clone());
    connection.execute("insert into word_explanations(id, attachment_id, content_sha256, payload_json, updated_at) values(?1,?2,?3,?4,strftime('%Y-%m-%dT%H:%M:%fZ','now')) on conflict(id) do update set payload_json=excluded.payload_json, updated_at=excluded.updated_at",
        params![id, attachment, hash, result.to_string()])?;
    Ok(result)
}
fn list(connection: &Connection, attachment: &str, hash: &str) -> Result<Vec<Value>, AiError> {
    let mut query = connection.prepare("select payload_json from word_explanations where attachment_id=?1 and content_sha256=?2 order by updated_at,id")?;
    let rows = query.query_map(params![attachment, hash], |row| row.get::<_, String>(0))?;
    rows.map(|row| {
        let value: Value = serde_json::from_str(&row?)
            .map_err(|_| AiError::InvalidInput("已保存词义解释格式无效。".into()))?;
        validate(&value)?;
        Ok(value)
    })
    .collect()
}
#[tauri::command]
pub async fn save_word_explanation(
    store: State<'_, AppStore>,
    draft: Value,
) -> Result<Value, AiCommandError> {
    let store = store.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let connection = store.connection()?;
        save(&connection, draft)
    })
    .await
    .map_err(|_| AiError::TemporaryUnavailable)?
    .map_err(Into::into)
}
#[tauri::command]
pub async fn list_word_explanations(
    store: State<'_, AppStore>,
    attachment_id: String,
    content_sha256: String,
) -> Result<Vec<Value>, AiCommandError> {
    if attachment_id.len() > 128 || content_sha256.len() != 64 {
        return Err(AiError::InvalidInput("附件身份无效。".into()).into());
    }
    let store = store.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let connection = store.connection()?;
        list(&connection, &attachment_id, &content_sha256)
    })
    .await
    .map_err(|_| AiError::TemporaryUnavailable)?
    .map_err(Into::into)
}

#[tauri::command]
pub async fn delete_word_explanation(
    store: State<'_, AppStore>,
    id: String,
    attachment_id: String,
    content_sha256: String,
) -> Result<(), AiCommandError> {
    if id.len() != 64
        || attachment_id.is_empty()
        || attachment_id.len() > 128
        || content_sha256.len() != 64
    {
        return Err(AiError::InvalidInput("词义解释身份无效。".into()).into());
    }
    let store = store.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let connection = store.connection()?;
        connection.execute(
            "delete from word_explanations where id=?1 and attachment_id=?2 and content_sha256=?3",
            params![id, attachment_id, content_sha256],
        )?;
        Ok::<_, AiError>(())
    })
    .await
    .map_err(|_| AiError::TemporaryUnavailable)?
    .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn upgrades_reopens_and_keeps_word_aids_separate_from_annotations() {
        let path =
            std::env::temp_dir().join(format!("word-explanations-{}.sqlite", uuid::Uuid::new_v4()));
        let mut db = Connection::open(&path).unwrap();
        db.pragma_update(None, "foreign_keys", true).unwrap();
        crate::storage_migration_tests::migrate_through(&mut db, 154);
        crate::storage_integrity::backfill_migration_checksums(&db).unwrap();
        let checksums = |db: &Connection| {
            db.prepare("select version,sha256 from migration_checksums where version<=154 order by version").unwrap()
            .query_map([],|r| Ok((r.get::<_,i64>(0)?,r.get::<_,String>(1)?))).unwrap().collect::<Result<Vec<_>,_>>().unwrap()
        };
        let before = checksums(&db);
        let hash = "a".repeat(64);
        db.execute("insert into content_objects(sha256,byte_size,mime_type,relative_path,created_at) values(?1,1,'application/pdf','fixture','now')", [&hash]).unwrap();
        db.execute("insert into attachments(id,content_sha256,display_name,created_at) values('pdf',?1,'fixture.pdf','now')", [&hash]).unwrap();
        crate::storage::migrate(&mut db).unwrap();
        assert_eq!(checksums(&db), before);
        let draft = serde_json::json!({"attachmentId":"pdf","contentSha256":hash,"context":{"word":"work","before":"In this ","after":", we study vision."},"segments":[{"pageIndex":0,"text":"work","prefix":"In this ","suffix":", we study vision.","rects":[{"x":0.1,"y":0.2,"width":0.1,"height":0.02}]}],"explanation":{"translation":"本研究","explanation":"指本文的研究工作。","providerId":"fixture","modelId":"test"}});
        let first = save(&db, draft.clone()).unwrap();
        let second = save(&db, draft.clone()).unwrap();
        assert_eq!(first["id"], second["id"]);
        drop(db);
        let mut db = Connection::open(&path).unwrap();
        crate::storage::migrate(&mut db).unwrap();
        assert_eq!(checksums(&db), before);
        assert_eq!(list(&db, "pdf", &hash).unwrap(), vec![first]);
        assert!(list(&db, "pdf", &"b".repeat(64)).unwrap().is_empty());
        let mut stale = draft.clone();
        stale["contentSha256"] = Value::String("b".repeat(64));
        assert!(save(&db, stale).is_err());
        let mut invalid = draft;
        invalid["segments"][0]["rects"][0]["width"] = Value::from(2.0);
        assert!(save(&db, invalid).is_err());
        assert_eq!(
            db.query_row("select count(*) from word_explanations", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            1
        );
        drop(db);
        std::fs::remove_file(path).unwrap();
    }
}
