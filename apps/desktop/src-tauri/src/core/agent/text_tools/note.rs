use super::*;
use crate::notes::{NoteDocument, NoteDocumentBlock};
use rusqlite::{Connection, OptionalExtension, params};
#[path = "note/admission.rs"]
mod admission;
#[path = "note/backend.rs"]
mod backend;
#[path = "note/block_moves.rs"]
mod block_moves;
#[path = "note/delivery.rs"]
pub(super) mod delivery;
#[path = "note/effects.rs"]
mod effects;
#[path = "note/receipt.rs"]
mod receipt;
#[path = "note/resources.rs"]
mod resources;
#[path = "note/scope.rs"]
pub(super) mod scope;
#[path = "note/storage.rs"]
mod storage;
pub(crate) use storage::{PendingNote, ReviewMetadata, StoredNote, canonical, load, pending};

pub(super) struct NoteBackend {
    pub scope: Option<crate::agent::writing_resources::OwnedWritingScope>,
    pub store: AppStore,
    pub record: SubmissionRecord,
}
pub(super) fn check_imports(
    backend: &NoteBackend,
    db: &Connection,
    note: &PendingNote,
) -> Result<(), String> {
    resources::check_imports(backend, db, note)
}
pub(super) fn check_references(
    backend: &NoteBackend,
    db: &Connection,
    notes: &[PendingNote],
    target: &PendingNote,
) -> Result<(), String> {
    let pages = notes
        .iter()
        .map(|note| {
            let mut note = note.clone();
            if note.path != target.path {
                note.candidate = None;
            }
            note
        })
        .collect::<Vec<_>>();
    resources::validate(backend, db, &pages)
}
fn address(path: &str) -> Result<(Option<String>, String), String> {
    let parts = path.split('/').collect::<Vec<_>>();
    if parts.len() != 3 || parts[0] != "notes" {
        return Err("invalid note virtual path".into());
    }
    let parent = if parts[1] == "root" {
        None
    } else {
        Some(
            Uuid::parse_str(parts[1])
                .map_err(|_| "invalid note parent")?
                .to_string(),
        )
    };
    let name = parts[2]
        .strip_suffix(".md")
        .filter(|value| !value.is_empty())
        .ok_or("note path must end in .md")?;
    Ok((parent, name.into()))
}
impl NoteBackend {
    fn location(
        &self,
        db: &Connection,
        path: &str,
        moving: Option<&str>,
    ) -> Result<(Option<String>, String), String> {
        if !path.starts_with("source://") {
            return address(path);
        }
        if let Some(page) = moving {
            let parts = text_patch::address::parse(path)?;
            if parts == ["notes"] {
                return Ok((None, page.to_owned()));
            }
            let addresses::Target::Note { page: parent, .. } =
                addresses::resolve(db, &self.record, path)?
            else {
                return Err("move destination must be a note or the notes directory".into());
            };
            return Ok((Some(parent), page.to_owned()));
        }
        match addresses::resolve(db, &self.record, path)? {
            addresses::Target::Note { page, parent } => Ok((parent, page)),
            _ => Err("source does not identify a note".into()),
        }
    }

    fn model_eligible(&self, db: &Connection) -> Result<(), String> {
        let finished: bool = db.query_row("select exists(select 1 from agent_items where conversation_id=?1 and json_extract(payload_json,'$.payload.type')='turnFinished' and json_extract(payload_json,'$.payload.turnId')=?2)", params![self.record.conversation_id.to_string(), self.record.turn_id.to_string()], |row| row.get(0)).map_err(|error| error.to_string())?;
        if finished {
            Err("turn finished; recover the original result or start a new submission".into())
        } else {
            Ok(())
        }
    }
    pub(super) fn editor_targets(
        &self,
        paths: &[String],
        include_candidates: bool,
    ) -> Result<std::collections::BTreeSet<String>, String> {
        let db = self.store.connection().map_err(|error| error.to_string())?;
        admission::targets(self, &db, paths, include_candidates)
    }

    pub(super) fn authorize(&self, db: &Connection, page: &str, write: bool) -> Result<(), String> {
        let active: bool = db
            .query_row(
                "select exists(select 1 from agent_conversations where id=?1 and archived=0)",
                [self.record.conversation_id.to_string()],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        if !active {
            return Err("conversation is unavailable or archived".into());
        }
        if scope::authorized(db, &self.record, page, write)? {
            return Ok(());
        }
        if crate::agent::context_session::resource_tools::note_access(
            &self.store,
            &self.record,
            db,
            page,
            write,
            self.scope.clone(),
        )? {
            return Ok(());
        }
        if scope::authorized_creation(self, db, page)? {
            return Ok(());
        }
        Err("note is outside the selected resource access".into())
    }
    pub(super) fn authorize_restore(&self, db: &Connection, page: &str) -> Result<(), String> {
        use crate::agent::context_session::{resource_grants, sources::Resource};
        let granted=self.record.input.context.iter().any(|source|matches!(resource_grants::resource(source),Some(Resource::Note{resource_id}) if resource_id.to_string()==page)&&source.anchor.as_deref().and_then(|value|serde_json::from_str::<Value>(value).ok()).is_some_and(|value|value["contextAccess"]=="readWrite"));
        let exists: bool = db
            .query_row(
                "select exists(select 1 from note_pages where id=?1)",
                [page],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        if exists && (granted || scope::restorable(db, &self.record, page)?) {
            Ok(())
        } else {
            self.authorize(db, page, true)
        }
    }
    fn current(&self, db: &Connection, path: &str, saved: bool) -> Result<StoredNote, String> {
        let page = if saved && path.starts_with("source://") {
            match addresses::resolve_saved(db, &self.record, path)? {
                addresses::Target::Note { page, .. } => page,
                _ => return Err("source is not a saved note".into()),
            }
        } else {
            self.location(db, path, None)?.1
        };
        if !saved
            && let Some(candidate) =
                pending(db, self.record.conversation_id)?
                    .into_iter()
                    .find(|note| {
                        if Uuid::parse_str(&page).is_ok() {
                            // A creation's original path is its source page.
                            // It must never shadow that existing resource ID.
                            note.candidate
                                .as_ref()
                                .or(note.baseline.as_ref())
                                .is_some_and(|note| note.page_id == page)
                        } else {
                            note.path == path || note.original_path == path
                        }
                    })
        {
            admission::check_saved(db, &candidate, self.record.request_id)?;
            let candidate = candidate.candidate.ok_or("note is proposed for deletion")?;
            self.authorize(db, &candidate.page_id, false)?;
            return Ok(candidate);
        }
        Uuid::parse_str(&page).map_err(|_| "read the new note's returned canonical path")?;
        self.authorize(db, &page, false)?;
        let current = load(db, &page)?;
        if current.status != "active"
            || (!path.starts_with("source://")
                && address(path)? != (current.parent_id.clone(), current.page_id.clone()))
        {
            return Err("note moved or is not active; rediscover its path".into());
        }
        Ok(current)
    }
    pub(super) fn snapshot(
        &self,
        db: &Connection,
        path: &str,
        saved: bool,
    ) -> Result<TextSnapshot, String> {
        let note = self.current(db, path, saved)?;
        let text =
            crate::notes::text_view::render(&scope::view(&self.record, &note)?, &note.title)?;
        let (title, parsed) = crate::notes::text_view::parse(&note.page_id, &text)?;
        if crate::notes::text_view::render(&parsed, &title)? != text {
            return Err(format!(
                "MODEL_NOTE_TEXT_VIEW_UNSTABLE: note text encoding changed during round-trip; page={}; request={}; preserve editor content; this is a text codec failure, not a save conflict",
                note.page_id, self.record.request_id
            ));
        }
        let pending_generation:Option<i64>=db.query_row("select generation from agent_note_patch_candidates where conversation_id=?1 and page_id=?2 and state='pending'",params![self.record.conversation_id.to_string(),note.page_id],|row|row.get(0)).optional().map_err(|error|error.to_string())?;
        Ok(TextSnapshot {
            path: if path.starts_with("source://") {
                text_patch::address::format(&text_patch::address::parse(path)?)
            } else {
                canonical(&note)
            },
            name: Some(note.title.clone()),
            version: format!(
                "note:{}:{}:sha256:{:x}:mapping:{:x}",
                note.revision,
                if saved {
                    0
                } else {
                    pending_generation.unwrap_or(0)
                },
                Sha256::digest(text.as_bytes()),
                Sha256::digest(serde_json::to_vec(&note).map_err(|error| error.to_string())?)
            ),
            text,
        })
    }
    pub(super) fn prepare(
        &self,
        db: &Connection,
        changes: &[TextChange],
        request: &str,
    ) -> Result<Vec<PendingNote>, String> {
        self.model_eligible(db)?;
        let mut group = admission::pending_changes(self, db, changes)?;
        let mut addressed = std::collections::BTreeSet::new();
        for existing in &group {
            admission::check_saved(db, existing, self.record.request_id)?;
        }
        for change in changes {
            let current = if change.before.is_some() {
                Some(self.current(db, &change.path, false)?)
            } else {
                None
            };
            let (parent, name) = if current.is_none() && change.path.starts_with("source://") {
                let (_, parent) = self.location(db, &change.path, None)?;
                (Some(parent), "new-page".to_owned())
            } else {
                self.location(
                    db,
                    change.destination.as_deref().unwrap_or(&change.path),
                    change
                        .destination
                        .as_ref()
                        .and(current.as_ref().map(|page| page.page_id.as_str())),
                )?
            };
            let identity = current
                .as_ref()
                .map(|page| page.page_id.as_str())
                .or(parent.as_deref())
                .unwrap_or(&name);
            if !addressed.insert(identity.to_owned()) {
                return Err(
                    "the same note is addressed more than once through source aliases".into(),
                );
            }
            if let Some(current) = &current {
                if scope::selected(&self.record, &current.page_id).is_some()
                    && (change.destination.is_some() || change.after.is_none())
                {
                    return Err("a selected-block view cannot move or recycle its page".into());
                }
                self.authorize(db, &current.page_id, true)?;
                let actual = self.snapshot(db, &change.path, false)?;
                if change.before.as_ref().is_none_or(|before| {
                    before.version != actual.version || before.text != actual.text
                }) {
                    return Err("candidate changed since patch preparation".into());
                }
                if let Some(destination) = &change.destination {
                    if name != current.page_id {
                        return Err(
                            "Move to preserves the page ID; change the parent directory only"
                                .into(),
                        );
                    }
                    if destination != &canonical(current)
                        && let Some(parent) = &parent
                    {
                        self.authorize(db, parent, true)?;
                    }
                }
            } else {
                let parent = parent
                    .as_ref()
                    .ok_or("new notes require an authorized parent page")?;
                if scope::selected(&self.record, parent).is_some() {
                    return Err("page creation requires a full-parent note grant".into());
                }
                self.authorize(db, parent, true)?;
                if Uuid::parse_str(&name).is_ok()
                    && db
                        .query_row(
                            "select exists(select 1 from note_pages where id=?1)",
                            [&name],
                            |row| row.get::<_, bool>(0),
                        )
                        .map_err(|error| error.to_string())?
                {
                    return Err("creation target exists".into());
                }
                if !change.path.starts_with("source://")
                    && group.iter().any(|note| note.path == change.path)
                {
                    return Err("creation target already proposed".into());
                }
            }
            let page = current
                .as_ref()
                .map(|note| note.page_id.clone())
                .unwrap_or_else(|| {
                    Uuid::new_v5(
                        &self.record.conversation_id,
                        format!("{request}:{}", change.path).as_bytes(),
                    )
                    .to_string()
                });
            let prior = group.iter().position(|note| {
                note.candidate
                    .as_ref()
                    .or(note.baseline.as_ref())
                    .is_some_and(|note| note.page_id == page)
            });
            let baseline = if let Some(index) = prior {
                group[index].baseline.clone()
            } else {
                current.clone()
            };
            let candidate = if let Some(after) = &change.after {
                let empty = NoteDocument {
                    title: None,
                    schema_version: 1,
                    page_id: page.clone(),
                    blocks: vec![],
                };
                let (title, document) =
                    if let (Some(current), Some(snapshot)) = (&current, &change.before) {
                        let view = scope::view(&self.record, current)?;
                        let (title, document) = crate::notes::text_view::mapped_candidate(
                            &view,
                            &current.title,
                            &snapshot.text,
                            after,
                            &change.edits,
                        )?;
                        let document = scope::merge(&self.record, current, &title, document)?;
                        (title, document)
                    } else {
                        crate::notes::text_view::candidate(&empty, after)?
                    };
                let position = if let Some(current) = &current
                    && current.parent_id == parent
                {
                    current.position
                } else {
                    db.query_row("select coalesce(max(position),-1)+1 from note_pages where parent_page_id is ?1",[&parent],|row|row.get(0)).map_err(|error|error.to_string())?
                };
                Some(StoredNote {
                    page_id: page.clone(),
                    title,
                    parent_id: parent,
                    position,
                    revision: current.as_ref().map_or(0, |note| note.revision),
                    status: "active".into(),
                    document,
                })
            } else {
                None
            };
            if baseline.is_none() && candidate.is_none() {
                if let Some(index) = prior {
                    group.remove(index);
                }
                for note in &mut group {
                    if let Some(candidate) = &mut note.candidate {
                        effects::remove(&mut candidate.document.blocks, &page);
                    }
                }
                continue;
            }
            let next = PendingNote {
                path: candidate
                    .as_ref()
                    .map_or_else(|| change.path.clone(), canonical),
                original_path: prior
                    .filter(|index| group[*index].baseline.is_none())
                    .map(|index| group[index].original_path.clone())
                    .unwrap_or_else(|| change.path.clone()),
                scope_request_id: self.record.request_id,
                review_document: prior.and_then(|index| group[index].review_document.clone()),
                review_metadata: prior.and_then(|index| group[index].review_metadata.clone()),
                reviewed_metadata: prior.is_some_and(|index| {
                    super::note_review::metadata_equal(
                        group[index].candidate.as_ref(),
                        candidate.as_ref(),
                    ) && group[index].reviewed_metadata
                }),
                manual_edited: prior.is_some_and(|index| group[index].manual_edited),
                saved: prior.and_then(|index| group[index].saved.clone()),
                baseline,
                candidate,
                imports: prior
                    .map(|index| group[index].imports.clone())
                    .unwrap_or_default(),
            };
            if let Some(index) = prior {
                group[index] = next;
            } else {
                group.push(next);
            }
        }
        block_moves::resolve(&mut group)?;
        effects::expand(self, db, &mut group)?;
        resources::validate(self, db, &group)?;
        if serde_json::to_vec(&group)
            .map_err(|error| error.to_string())?
            .len()
            > 32 * 1024 * 1024
        {
            return Err("candidate group exceeds 32 MiB; use a smaller group".into());
        }
        Ok(group)
    }
}
