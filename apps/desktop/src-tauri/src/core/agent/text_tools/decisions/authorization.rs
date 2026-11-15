//! Original and latest resource scopes gate every review/storage transition.
use super::*;

pub(in crate::agent::text_tools) fn authorize(
    store: &AppStore,
    db: &Connection,
    conversation: Uuid,
    notes: &[PendingNote],
    restore: bool,
) -> Result<(), String> {
    let latest_request:String=db.query_row("select request_id from agent_submissions where conversation_id=?1 order by rowid desc limit 1",[conversation.to_string()],|row|row.get(0)).map_err(|error|error.to_string())?;
    let latest_request =
        Uuid::parse_str(&latest_request).map_err(|_| "invalid latest authorization")?;
    let latest = crate::agent::repository::submissions::read(db, conversation, latest_request)
        .map_err(|error| error.to_string())?
        .ok_or("current authorization unavailable")?;
    for note in notes {
        let record =
            crate::agent::repository::submissions::read(db, conversation, note.scope_request_id)
                .map_err(|error| error.to_string())?
                .ok_or("proposal authorization submission is missing")?;
        let backend = note::NoteBackend {
            scope: None,
            store: store.clone(),
            record,
        };
        note::check_imports(&backend, db, note)?;
        if !restore {
            note::check_references(&backend, db, notes, note)?;
        }
        let target = match &note.baseline {
            Some(page) => page.page_id.clone(),
            None => note::scope::creation_source(&backend, db, note)?,
        };
        let target = target.as_str();
        let restore_access = restore
            || note
                .saved
                .as_ref()
                .is_some_and(|page| page.status == "trashed");
        if restore_access {
            backend.authorize_restore(db, target)?;
        } else {
            backend.authorize(db, target, true)?;
        }
        if latest_request != note.scope_request_id {
            note::scope::retains_scope(&backend.record, &latest, target)?;
            let current = note::NoteBackend {
                scope: None,
                store: store.clone(),
                record: latest.clone(),
            };
            note::check_imports(&current, db, note)?;
            if !restore {
                note::check_references(&current, db, notes, note)?;
            }
            if restore_access {
                current.authorize_restore(db, target)?;
            } else {
                current.authorize(db, target, true)?;
            }
        }
        if note.baseline.as_ref().map(|page| &page.parent_id)
            != note.candidate.as_ref().map(|page| &page.parent_id)
            && let Some(parent) = note
                .candidate
                .as_ref()
                .and_then(|page| page.parent_id.as_deref())
        {
            backend.authorize(db, parent, true)?;
            note::NoteBackend {
                scope: None,
                store: store.clone(),
                record: latest.clone(),
            }
            .authorize(db, parent, true)?;
        }
    }
    Ok(())
}
