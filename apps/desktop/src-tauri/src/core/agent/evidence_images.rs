//! Captured images reuse the content-addressed asset store. SQLite/drafts hold
//! only immutable handles, never binary arrays, data URLs or filesystem paths.
use crate::storage::AppStore;
use research_agent_core::{submission::SubmissionError, user_input::evidence::EvidenceImage};
use rusqlite::{OptionalExtension, params};
use serde::Deserialize;
use std::io::Read;
use tauri::State;
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PinRequest {
    asset_id: String,
    media_type: String,
    byte_size: usize,
}
#[tauri::command]
pub(crate) async fn agent_pin_evidence_image(
    store: State<'_, AppStore>,
    request: PinRequest,
) -> Result<EvidenceImage, String> {
    let store = store.inner().clone();
    tokio::task::spawn_blocking(move||{
        if request.asset_id.len()!=64 || !request.asset_id.bytes().all(|byte|byte.is_ascii_hexdigit()&&!byte.is_ascii_uppercase()) || request.byte_size==0 || request.byte_size>524288 || !matches!(request.media_type.as_str(),"image/png"|"image/jpeg"|"image/webp"){return Err(SubmissionError::InvalidInput);}
        let _guard=store.content_read().map_err(|_|SubmissionError::Unavailable)?;
        let db=store.connection().map_err(|_|SubmissionError::Unavailable)?;
        let image=EvidenceImage {asset_id:request.asset_id,capture_id:Uuid::new_v4(),media_type:request.media_type,byte_size:request.byte_size};
        let existing:Option<(String,usize)>=db.query_row("select mime_type,byte_size from content_objects where sha256=?1",[&image.asset_id],|row|Ok((row.get(0)?,row.get(1)?))).optional().map_err(|_|SubmissionError::Unavailable)?;
        if existing!=Some((image.media_type.clone(),image.byte_size)){return Err(SubmissionError::InvalidInput);}
        db.execute("insert into agent_evidence_images(id,content_sha256,media_type,byte_size,draft_held) values(?1,?2,?3,?4,1)",params![image.capture_id.to_string(),image.asset_id,image.media_type,image.byte_size]).map_err(|_|SubmissionError::Unavailable)?;
        Ok(image)
    }).await.map_err(|_|"图片引用存储不可用".to_owned())?.map_err(|_|"无法保留图片快照，请重新选择图片".to_owned())
}
pub(crate) fn read(store: &AppStore, image: &EvidenceImage) -> Result<Vec<u8>, SubmissionError> {
    let _guard = store
        .content_read()
        .map_err(|_| SubmissionError::Unavailable)?;
    let db = store
        .connection()
        .map_err(|_| SubmissionError::Unavailable)?;
    let path:Option<String>=db.query_row("select c.relative_path from agent_evidence_images e join content_objects c on c.sha256=e.content_sha256 where e.id=?1 and e.content_sha256=?2 and e.media_type=?3 and e.byte_size=?4",params![image.capture_id.to_string(),image.asset_id,image.media_type,image.byte_size],|row|row.get(0)).optional().map_err(|_|SubmissionError::Unavailable)?;
    let path = path.ok_or(SubmissionError::NotFound)?;
    drop(db);
    let components: Vec<_> = path.split('/').collect();
    let (name, parents) = components.split_last().ok_or(SubmissionError::Corrupt)?;
    let root = store
        .content_directory()
        .canonicalize()
        .map_err(|_| SubmissionError::Unavailable)?;
    let mut directory = crate::agent::instructions::directory::Directory::root(&root)
        .map_err(|_| SubmissionError::Unavailable)?;
    for part in parents {
        directory = directory
            .child(std::path::Path::new(part))
            .map_err(|_| SubmissionError::Corrupt)?;
    }
    let file = directory
        .config_file(name)
        .map_err(|_| SubmissionError::Corrupt)?
        .ok_or(SubmissionError::NotFound)?;
    if image.byte_size > 524288
        || file
            .metadata()
            .map_err(|_| SubmissionError::Unavailable)?
            .len()
            != image.byte_size as u64
    {
        return Err(SubmissionError::Corrupt);
    }
    let mut bytes = Vec::new();
    file.take(image.byte_size as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| SubmissionError::Unavailable)?;
    if !image.validates_bytes(&bytes) {
        return Err(SubmissionError::Corrupt);
    }
    Ok(bytes)
}
#[tauri::command]
pub(crate) async fn agent_read_evidence_image(
    store: State<'_, AppStore>,
    request: EvidenceImage,
) -> Result<Vec<u8>, String> {
    let store = store.inner().clone();
    tokio::task::spawn_blocking(move || read(&store, &request))
        .await
        .map_err(|_| "图片快照读取失败".to_owned())?
        .map_err(|_| "图片快照不可用或已损坏".to_owned())
}
#[tauri::command]
pub(crate) fn agent_release_evidence_image(
    store: State<'_, AppStore>,
    capture_id: Uuid,
) -> Result<(), String> {
    let db = store
        .connection()
        .map_err(|_| "图片引用存储不可用".to_owned())?;
    db.execute(
        "update agent_evidence_images set draft_held=0 where id=?1",
        [capture_id.to_string()],
    )
    .map_err(|_| "图片引用存储不可用".to_owned())?;
    Ok(())
}
#[tauri::command]
pub(crate) fn agent_reconcile_evidence_drafts(
    store: State<'_, AppStore>,
    capture_ids: Vec<Uuid>,
) -> Result<(), String> {
    reconcile_drafts(&store, capture_ids)
}

pub(crate) fn reconcile_drafts(store: &AppStore, capture_ids: Vec<Uuid>) -> Result<(), String> {
    if capture_ids.len() > 65536 {
        return Err("图片草稿数量过大".into());
    }
    let mut db = store
        .connection()
        .map_err(|_| "图片引用存储不可用".to_owned())?;
    let tx = db
        .transaction()
        .map_err(|_| "图片引用存储不可用".to_owned())?;
    tx.execute("update agent_evidence_images set draft_held=0", [])
        .map_err(|_| "图片引用存储不可用".to_owned())?;
    for id in capture_ids {
        tx.execute(
            "update agent_evidence_images set draft_held=1 where id=?1",
            [id.to_string()],
        )
        .map_err(|_| "图片引用存储不可用".to_owned())?;
    }
    tx.execute("delete from content_object_gc_candidates where content_sha256 in(select content_sha256 from content_object_references)",[]).map_err(|_|"图片引用存储不可用".to_owned())?;
    tx.commit().map_err(|_| "图片引用存储不可用".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    #[test]
    fn upgrade_reopen_preserves_cas_and_rejects_changed_images() {
        let temporary = tempfile::tempdir().unwrap();
        let db_path = temporary.path().join("store.sqlite3");
        let mut db = rusqlite::Connection::open(&db_path).unwrap();
        db.execute_batch(
            "create table schema_migrations(version integer primary key,applied_at text not null);",
        )
        .unwrap();
        // Deployed predecessor, including its recorded migration checksums.
        for (version, sql) in crate::storage_migrations::MIGRATIONS
            .iter()
            .filter(|(version, _)| *version < 156)
        {
            db.execute_batch(sql).unwrap();
            db.execute(
                "insert into schema_migrations values(?1, 'test')",
                [*version],
            )
            .unwrap();
            crate::storage_integrity::record_migration_checksum(&db, *version, sql).unwrap();
        }
        let previous: String = db
            .query_row(
                "select sql from sqlite_master where name='content_object_references'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(previous.contains("agent_hook_spill_artifact"));
        crate::storage::migrate(&mut db).unwrap();
        assert_eq!(
            db.query_row("select max(version) from schema_migrations", [], |row| row
                .get::<_, i64>(
                0
            ))
            .unwrap(),
            crate::storage_migrations::MIGRATIONS.last().unwrap().0
        );
        let bytes = b"\x89PNG\r\n\x1a\nfixture".to_vec();
        let hash = format!("{:x}", Sha256::digest(&bytes));
        let root = temporary.path().join("objects");
        let relative = format!("{}/{}/{}", &hash[..2], &hash[2..4], hash);
        std::fs::create_dir_all(root.join(&relative).parent().unwrap()).unwrap();
        std::fs::write(root.join(&relative), &bytes).unwrap();
        let image = EvidenceImage {
            asset_id: hash.clone(),
            capture_id: Uuid::new_v4(),
            media_type: "image/png".into(),
            byte_size: bytes.len(),
        };
        db.execute("insert into content_objects(sha256,relative_path,mime_type,byte_size,created_at) values(?1,?2,'image/png',?3,'now')",params![hash,relative,bytes.len()]).unwrap();
        db.execute(
            "insert into agent_evidence_images values(?1,?2,'image/png',?3,1)",
            params![image.capture_id.to_string(), hash, bytes.len()],
        )
        .unwrap();
        drop(db);
        let mut db = rusqlite::Connection::open(&db_path).unwrap();
        crate::storage::migrate(&mut db).unwrap();
        let store = AppStore::for_test(db, root.clone());
        assert_eq!(read(&store, &image).unwrap(), bytes);
        let record = serde_json::to_string(&image).unwrap();
        assert!(!record.contains("bytes"));
        assert!(record.contains("assetId"));
        assert_eq!(store.connection().unwrap().query_row("select count(*) from content_object_references where owner_kind='agent_evidence_image'",[],|row|row.get::<_,i64>(0)).unwrap(),1);
        {
            let db = store.connection().unwrap();
            let retained = || {
                db.query_row("select count(*) from content_object_references where owner_kind='agent_evidence_image'", [], |row| row.get::<_, i64>(0)).unwrap()
            };
            db.execute("update agent_evidence_images set draft_held=0", [])
                .unwrap();
            assert_eq!(retained(), 0);
            db.execute(
                "insert into agent_conversations(id,title) values('evidence-chat','Evidence')",
                [],
            )
            .unwrap();
            let input = serde_json::json!({"type":"annotation","image":image});
            let queue = serde_json::json!({"inputs":[{"userInput":[input]}]}).to_string();
            db.execute(
                "insert into agent_input_queues values('evidence-chat',0,?1)",
                [&queue],
            )
            .unwrap();
            assert_eq!(retained(), 1);
            db.execute(
                "delete from agent_input_queues where conversation_id='evidence-chat'",
                [],
            )
            .unwrap();
            assert_eq!(retained(), 0);
            db.execute(
                "insert into agent_input_queue_receipts values('receipt','evidence-chat',?1,'{}')",
                [&queue],
            )
            .unwrap();
            assert_eq!(retained(), 1);
            db.execute(
                "delete from agent_input_queue_receipts where request_id='receipt'",
                [],
            )
            .unwrap();
            assert_eq!(retained(), 0);
            let submission = serde_json::json!({"userInput":[input]}).to_string();
            db.execute("insert into agent_submissions(conversation_id,request_id,turn_id,context_snapshot_id,schema_version,input_json) values('evidence-chat','request','turn','snapshot',1,?1)", [&submission]).unwrap();
            assert_eq!(retained(), 1);
        }
        let mut wrong = image.clone();
        wrong.asset_id = "f".repeat(64);
        assert!(read(&store, &wrong).is_err());
        std::fs::write(root.join(relative), b"corrupt").unwrap();
        assert!(read(&store, &image).is_err());
    }
}
