//! Resource-scoped write admission. Proposal snapshots are recovery/history
//! material, not a conversation-wide lock on otherwise independent resources.
use super::*;
use std::collections::BTreeSet;

pub(super) fn targets(
    backend: &NoteBackend,
    db: &Connection,
    paths: &[String],
    include_related: bool,
) -> Result<BTreeSet<String>, String> {
    let mut targets = BTreeSet::new();
    for path in paths {
        let leaf = text_patch::address::parse(path)
            .ok()
            .and_then(|parts| parts.last().cloned());
        let (parent, name) = backend.location(
            db,
            path,
            include_related.then_some(leaf.as_deref()).flatten(),
        )?;
        if include_related && let Some(parent) = parent {
            targets.insert(parent);
        }
        if Uuid::parse_str(&name).is_ok() {
            targets.insert(name);
        } else {
            targets.extend(storage::pending_alias_targets(
                db,
                backend.record.conversation_id,
                path,
            )?);
        }
    }
    if include_related {
        for note in storage::pending_for_targets(db, backend.record.conversation_id, &targets)? {
            if let Some(page) = note.candidate.as_ref().or(note.baseline.as_ref()) {
                targets.insert(page.page_id.clone());
            }
        }
    }
    Ok(targets)
}

pub(super) fn pending_changes(
    backend: &NoteBackend,
    db: &Connection,
    changes: &[TextChange],
) -> Result<Vec<PendingNote>, String> {
    let paths = changes
        .iter()
        .flat_map(|change| std::iter::once(change.path.clone()).chain(change.destination.clone()))
        .collect::<Vec<_>>();
    storage::pending_for_targets(
        db,
        backend.record.conversation_id,
        &targets(backend, db, &paths, true)?,
    )
}

pub(super) fn check_saved(
    db: &Connection,
    note: &PendingNote,
    request: Uuid,
) -> Result<(), String> {
    if let Some(base) = note.saved.as_ref().or(note.baseline.as_ref()) {
        let current = load(db, &base.page_id)?;
        if serde_json::to_value(&current).map_err(|error| error.to_string())?
            != serde_json::to_value(base).map_err(|error| error.to_string())?
        {
            return Err(format!(
                "MODEL_NOTE_PROPOSAL_STALE: saved note changed; page={}; expectedRevision={}; currentRevision={}; request={}; preserve saved input and review this proposal",
                base.page_id, base.revision, current.revision, request
            ));
        }
    }
    Ok(())
}
