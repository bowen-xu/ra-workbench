use super::*;
use std::collections::BTreeSet;

fn assets(blocks: &[NoteDocumentBlock]) -> BTreeSet<String> {
    let mut result = BTreeSet::new();
    for block in blocks {
        if block.content["kind"] == "asset"
            && let Some(id) = block.content["assetId"].as_str()
        {
            result.insert(id.into());
        }
        result.extend(assets(&block.children));
    }
    result
}
fn references(blocks: &[NoteDocumentBlock]) -> BTreeSet<(String, String)> {
    let mut result = BTreeSet::new();
    for block in blocks {
        if block.content["kind"] == "reference" && block.block_type != "childPage" {
            result.insert((
                block.content["targetKind"].as_str().unwrap_or("").into(),
                block.content["targetId"].as_str().unwrap_or("").into(),
            ));
        }
        fn inline(value: &Value, result: &mut BTreeSet<(String, String)>) {
            match value {
                Value::Array(values) => {
                    for value in values {
                        inline(value, result);
                    }
                }
                Value::Object(values) => {
                    if value["type"] == "mention" {
                        result.insert((
                            value["targetKind"].as_str().unwrap_or("").into(),
                            value["targetId"].as_str().unwrap_or("").into(),
                        ));
                    }
                    for value in values.values() {
                        inline(value, result);
                    }
                }
                _ => {}
            }
        }
        inline(&block.content, &mut result);
        result.extend(references(&block.children));
    }
    result
}
fn authorized_attachment(
    backend: &NoteBackend,
    db: &Connection,
    id: &str,
) -> Result<String, String> {
    if let Some(image) = backend
        .record
        .input
        .user_input
        .iter()
        .find_map(|input| match input {
            research_agent_core::user_input::UserInput::Annotation {
                image: Some(image), ..
            } if image.capture_id.to_string() == id => Some(image),
            _ => None,
        })
    {
        let valid:bool=db.query_row("select exists(select 1 from agent_evidence_images where id=?1 and content_sha256=?2 and media_type=?3 and byte_size=?4)",params![id,image.asset_id,image.media_type,image.byte_size],|row|row.get(0)).map_err(|error|error.to_string())?;
        return if valid {
            Ok(image.asset_id.clone())
        } else {
            Err("captured image handle changed or expired".into())
        };
    }
    let item: String = db
        .query_row("select a.item_id from attachments a join bibliographic_items i on i.id=a.item_id where a.id=?1 and i.trashed_at is null", [id], |row| {
            row.get(0)
        })
        .map_err(|_| "import source is unavailable")?;
    let allowed=backend.record.input.context.iter().any(|source|{
        let resource=crate::agent::context_session::resource_grants::resource(source);
        match resource{
            Some(crate::agent::context_session::sources::Resource::Paper{resource_id,attachment_id,..})=>resource_id==item&&attachment_id.as_deref()==Some(id),
            Some(resource@crate::agent::context_session::sources::Resource::PaperGroup{..})=>crate::agent::context_session::group_scope::parts(db,&resource, None).is_ok_and(|parts|parts.iter().any(|part|part.selected&&matches!(&part.resource,crate::agent::context_session::sources::Resource::Paper{attachment_id:Some(attachment),..} if attachment==id))),
            _=>false
        }
    });
    if !allowed {
        return Err("attachment import source is outside readable context".into());
    }
    db.query_row(
        "select content_sha256 from attachments where id=?1",
        [id],
        |row| row.get(0),
    )
    .map_err(|error| error.to_string())
}
pub(super) fn validate(
    backend: &NoteBackend,
    db: &Connection,
    group: &[PendingNote],
) -> Result<(), String> {
    let mut known_assets = BTreeSet::new();
    let mut known_references = BTreeSet::new();
    for note in group {
        if note.baseline.is_none()
            && let Some(page) = &note.candidate
        {
            // This group has already authorized the creation source; its new
            // entrance may reference the identity before formal publication.
            known_references.insert(("note_page".into(), page.page_id.clone()));
        }
        if let Some(before) = &note.baseline {
            known_assets.extend(assets(&before.document.blocks));
            known_references.extend(references(&before.document.blocks));
        }
    }
    for note in group {
        let Some(after) = &note.candidate else {
            continue;
        };
        for asset in assets(&after.document.blocks) {
            if let Some(id) = asset.strip_prefix("import://") {
                authorized_attachment(backend, db, id)?;
                continue;
            }
            if asset.len() != 64 || !asset.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return Err("invented asset handle".into());
            }
            let exists: bool = db
                .query_row(
                    "select exists(select 1 from content_objects where sha256=?1)",
                    [&asset],
                    |row| row.get(0),
                )
                .map_err(|error| error.to_string())?;
            if !exists {
                return Err("asset is unavailable".into());
            }
            if !known_assets.contains(&asset) {
                let mut statement=db.prepare("select b.page_id from note_block_assets a join note_blocks b on b.id=a.block_id where a.content_sha256=?1").map_err(|error|error.to_string())?;
                let pages = statement
                    .query_map([&asset], |row| row.get::<_, String>(0))
                    .map_err(|error| error.to_string())?
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .map_err(|error| error.to_string())?;
                if !pages
                    .iter()
                    .any(|page| backend.authorize(db, page, false).is_ok())
                {
                    let held:bool=db.query_row("select exists(select 1 from agent_note_patch_assets where conversation_id=?1 and content_sha256=?2)",params![backend.record.conversation_id.to_string(),asset],|row|row.get(0)).map_err(|error|error.to_string())?;
                    if !held {
                        return Err("asset is outside the readable context".into());
                    }
                }
            }
        }
        for (kind, id) in references(&after.document.blocks) {
            if known_references.contains(&(kind.clone(), id.clone())) {
                continue;
            }
            let allowed = if kind == "reading_note" {
                db.query_row(
                    "select page_id from reading_note_sources where block_id=?1",
                    [&id],
                    |row| row.get::<_, String>(0),
                )
                .ok()
                .is_some_and(|page| backend.authorize(db, &page, false).is_ok())
            } else if kind == "note_page" {
                backend.authorize(db, &id, false).is_ok()
            } else {
                backend.record.input.context.iter().any(|source|source.resource_id==id&&((kind=="paper"&&source.kind==research_agent_core::submission::ContextSourceKind::Paper)||(kind=="manuscript"&&source.kind==research_agent_core::submission::ContextSourceKind::Manuscript)))
            };
            if !allowed {
                return Err("new reference target is outside readable context".into());
            }
        }
    }
    Ok(())
}
pub(super) fn retain_assets(
    db: &Connection,
    conversation: Uuid,
    note: &PendingNote,
) -> Result<(), String> {
    let page = note
        .candidate
        .as_ref()
        .or(note.baseline.as_ref())
        .ok_or("missing note")?;
    db.execute(
        "delete from agent_note_patch_assets where conversation_id=?1 and page_id=?2",
        params![conversation.to_string(), page.page_id],
    )
    .map_err(|error| error.to_string())?;
    let mut ids = BTreeSet::new();
    for document in note.baseline.iter().chain(note.candidate.iter()) {
        ids.extend(assets(&document.document.blocks));
    }
    for asset in ids {
        db.execute("insert into agent_note_patch_assets(conversation_id,page_id,content_sha256) values(?1,?2,?3)",params![conversation.to_string(),page.page_id,asset]).map_err(|error|error.to_string())?;
    }
    Ok(())
}
pub(super) fn check_imports(
    backend: &NoteBackend,
    db: &Connection,
    note: &PendingNote,
) -> Result<(), String> {
    let used = note
        .candidate
        .as_ref()
        .map(|page| assets(&page.document.blocks))
        .unwrap_or_default();
    for (id, hash) in &note.imports {
        if used.contains(hash) && authorized_attachment(backend, db, id)? != *hash {
            return Err("import source version changed; regenerate the proposal".into());
        }
    }
    Ok(())
}
pub(super) fn import(backend: &NoteBackend, group: &mut [PendingNote]) -> Result<(), String> {
    fn visit(
        backend: &NoteBackend,
        blocks: &mut [NoteDocumentBlock],
        imports: &mut Vec<(String, String)>,
    ) -> Result<(), String> {
        for block in blocks {
            if let Some(id) = block.content["assetId"]
                .as_str()
                .and_then(|id| id.strip_prefix("import://"))
                .map(str::to_owned)
            {
                let hash = {
                    let db = backend
                        .store
                        .connection()
                        .map_err(|error| error.to_string())?;
                    authorized_attachment(backend, &db, &id)?
                };
                if hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                    return Err("invalid object handle".into());
                }
                let path = backend
                    .store
                    .content_directory()
                    .join(&hash[..2])
                    .join(&hash[2..4])
                    .join(&hash);
                let metadata =
                    std::fs::symlink_metadata(&path).map_err(|_| "import source unavailable")?;
                if !metadata.file_type().is_file() || metadata.len() > 16 * 1024 * 1024 {
                    return Err("import source exceeds limits or is not a regular object".into());
                }
                let bytes = {
                    let _read = backend
                        .store
                        .content_read()
                        .map_err(|error| error.to_string())?;
                    std::fs::read(&path).map_err(|_| "import source unavailable")?
                };
                if format!("{:x}", Sha256::digest(&bytes)) != hash {
                    return Err("import source hash changed".into());
                }
                let request = crate::notes::assets::NoteAssetImportRequest {
                    bytes,
                    display_name: block.content["displayName"].as_str().unwrap_or("").into(),
                    mime_type: block.content["mimeType"].as_str().unwrap_or("").into(),
                };
                let asset = crate::notes::assets::import_note_asset_from(
                    &backend.store,
                    &crate::notes::NoteAssetManager::default(),
                    request,
                )
                .map_err(|error| error.to_string())?;
                imports.push((id, hash));
                block.content["assetId"] = json!(asset.id);
                block.content["displayName"] = json!(asset.display_name);
                block.content["mimeType"] = json!(asset.mime_type);
            }
            visit(backend, &mut block.children, imports)?;
        }
        Ok(())
    }
    for note in group {
        if let Some(candidate) = &mut note.candidate {
            visit(backend, &mut candidate.document.blocks, &mut note.imports)?;
        }
    }
    Ok(())
}
