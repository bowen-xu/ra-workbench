//! Sidebar structure edits are explicit user edits over a settled review only.
use super::*;

pub(crate) fn check_navigation(db: &Connection, before: &StoredNote) -> Result<(), String> {
    for note in active_reviews(db, &before.page_id)? {
        if !resolved(&note)
            || serde_json::to_value(&note.saved).ok() != serde_json::to_value(Some(before)).ok()
        {
            return Err("请先完成修改审阅或核对版本冲突，再移动页面链接。".into());
        }
    }
    Ok(())
}
fn active_reviews(db: &Connection, page: &str) -> Result<Vec<PendingNote>, String> {
    let mut statement = db.prepare("select candidate_json from agent_note_patch_candidates where page_id=?1 and state in ('pending','accepted')").map_err(|e| e.to_string())?;
    let values = statement
        .query_map([page], |row| row.get::<_, String>(0))
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    values
        .into_iter()
        .map(|value| serde_json::from_str(&value).map_err(|e| e.to_string()))
        .collect()
}
pub(crate) fn synchronize_navigation(
    tx: &rusqlite::Transaction<'_>,
    before: &StoredNote,
    after: &StoredNote,
) -> Result<(), String> {
    check_navigation(tx, before)?;
    crate::notes::hbs::synchronize_navigation_if_settled(tx, before, after)?;
    let mut statement = tx.prepare("select conversation_id,proposal_id,candidate_json from agent_note_patch_candidates where page_id=?1 and state in ('pending','accepted')").map_err(|e| e.to_string())?;
    let rows = statement
        .query_map([&before.page_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    drop(statement);
    for (conversation, proposal, raw) in rows {
        let mut note: PendingNote = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
        note.candidate = Some(after.clone());
        note.saved = Some(after.clone());
        note.review_document = Some(after.document.clone());
        note.review_metadata = Some(metadata(Some(after)));
        note.manual_edited = true;
        // Same settled manual-edit state as the existing review writer. The old
        // operation's whole Undo must not erase a subsequent user navigation edit.
        tx.execute("update agent_note_patch_candidates set candidate_json=?4,state='pending',persistence_version=2 where conversation_id=?1 and page_id=?2 and proposal_id=?3",
            params![conversation,before.page_id,proposal,serde_json::to_string(&note).map_err(|e| e.to_string())?]).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn navigation_adopts_only_settled_exact_reviews_and_keeps_later_manual_edits_safe() {
        let mut db = Connection::open_in_memory().unwrap();
        db.execute_batch("create table agent_note_patch_candidates(conversation_id text,page_id text,proposal_id text,candidate_json text,state text,persistence_version integer)").unwrap();
        let before = StoredNote {
            page_id: "page".into(),
            title: "page".into(),
            parent_id: None,
            position: 0,
            revision: 1,
            status: "active".into(),
            document: NoteDocument {
                title: Some("page".into()),
                page_id: "page".into(),
                schema_version: 1,
                blocks: vec![],
            },
        };
        let note: PendingNote = serde_json::from_value(serde_json::json!({"path":"source://notes/page","originalPath":"source://notes/page","scopeRequestId":Uuid::new_v4(),"baseline":before,"candidate":before,"saved":before,"reviewDocument":before.document,"reviewedMetadata":true,"imports":[]})).unwrap();
        db.execute("insert into agent_note_patch_candidates values('conversation','page','proposal',?1,'accepted',2)",[serde_json::to_string(&note).unwrap()]).unwrap();
        let mut after = before.clone();
        after.revision = 2;
        after.document.blocks.push(crate::notes::NoteDocumentBlock { id: "link".into(), block_type: "researchObjectReference".into(), props: serde_json::json!({}), content: serde_json::json!({"kind":"reference","targetKind":"note_page","targetId":"other","labelSnapshot":"other","quoteSnapshot":""}), children: vec![] });
        let tx = db.transaction().unwrap();
        synchronize_navigation(&tx, &before, &after).unwrap();
        tx.commit().unwrap();
        check_navigation(&db, &after).unwrap();
        assert!(check_navigation(&db, &before).is_err());
        let loaded = active_reviews(&db, "page").unwrap().pop().unwrap();
        assert!(loaded.manual_edited);
        assert!(resolved(&loaded));
        let mut pending = loaded;
        pending.review_document = Some(before.document);
        db.execute(
            "update agent_note_patch_candidates set candidate_json=?1",
            [serde_json::to_string(&pending).unwrap()],
        )
        .unwrap();
        assert!(check_navigation(&db, &after).is_err());
    }
}
