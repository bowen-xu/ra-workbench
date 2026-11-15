//! User-only proposal review/application. No model tool can accept a note proposal.
use super::note_review::views::page_review;
use super::*;
use crate::notes::text_change_set::{PageChange, apply as apply_changes};
use note::{PendingNote, StoredNote};
use rusqlite::{Connection, OptionalExtension, params};
#[path = "decisions/authorization.rs"]
mod authorization;
pub(super) use authorization::authorize;

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Review {
    #[serde(default)]
    pub persistence_version: u8,
    pub proposal_id: String,
    pub generation: i64,
    pub pages: Vec<PageReview>,
    pub baseline_matches: bool,
    pub state: String,
    pub operation_generation: i64,
    pub whole_undo_available: bool,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageReview {
    pub page_id: String,
    pub path: String,
    pub title: String,
    pub kind: String,
    pub before: Option<String>,
    pub after: Option<String>,
    // Rich documents are a user-interface contract, never model tool output.
    pub before_document: Option<crate::notes::NoteDocument>,
    pub after_document: Option<crate::notes::NoteDocument>,
    pub review_document: Option<crate::notes::NoteDocument>,
    pub chunks: Vec<super::note_review::chunks::Chunk>,
    pub metadata_pending: bool,
    #[serde(default)]
    pub structural_metadata_pending: bool,
    pub expected_revision: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery: Option<PageRecovery>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageRecovery {
    pub current_revision: i64,
    pub status: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Decision {
    pub conversation_id: Uuid,
    pub proposal_id: String,
    pub generation: i64,
    pub request_id: Uuid,
    pub action: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Receipt {
    pub request_id: Uuid,
    pub proposal_id: String,
    pub action: String,
    #[serde(default)]
    pub generation: i64,
    pub pages: Vec<StoredNote>,
    pub(crate) originals: Vec<PendingNote>,
}

pub(super) fn active(db: &Connection, conversation: Uuid) -> Result<(), String> {
    if conversation.is_nil()
        || !db
            .query_row(
                "select exists(select 1 from agent_conversations where id=?1 and archived=0)",
                [conversation.to_string()],
                |row| row.get::<_, bool>(0),
            )
            .map_err(|error| error.to_string())?
    {
        return Err("conversation is unavailable or archived".into());
    }
    Ok(())
}
pub(super) fn group(
    db: &Connection,
    conversation: Uuid,
    proposal: &str,
) -> Result<(i64, Vec<PendingNote>), String> {
    group_for_edit(db, conversation, proposal, false)
}
pub(super) fn group_for_edit(
    db: &Connection,
    conversation: Uuid,
    proposal: &str,
    allow_saved: bool,
) -> Result<(i64, Vec<PendingNote>), String> {
    active(db, conversation)?;
    let mut statement=db.prepare("select generation,candidate_json from agent_note_patch_candidates where conversation_id=?1 and proposal_id=?2 and (state='pending' or (?3 and state='accepted' and persistence_version=2)) order by page_id").map_err(|error|error.to_string())?;
    let rows = statement
        .query_map(
            params![conversation.to_string(), proposal, allow_saved],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
        )
        .map_err(|error| error.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| error.to_string())?;
    let generation = rows
        .first()
        .map(|row| row.0)
        .ok_or("proposal is no longer pending")?;
    if rows.iter().any(|row| row.0 != generation) {
        return Err("proposal generation mismatch".into());
    }
    Ok((
        generation,
        rows.into_iter()
            .map(|(_, value)| serde_json::from_str(&value).map_err(|error| error.to_string()))
            .collect::<Result<_, _>>()?,
    ))
}
pub(super) fn matches(db: &Connection, notes: &[PendingNote]) -> bool {
    notes
        .iter()
        .all(|note| super::note_review::persistence::matches(db, note))
}
pub(crate) fn review(db: &Connection, conversation: Uuid) -> Result<Option<Review>, String> {
    review_for_page(db, conversation, None)
}
fn review_for_page(
    db: &Connection,
    conversation: Uuid,
    page: Option<Uuid>,
) -> Result<Option<Review>, String> {
    active(db, conversation)?;
    // Page recovery must never materialize another page, including a purged
    // sibling whose historical proposal still exists. Conversation reads retain
    // their full group; page reads expose only their addressed resource.
    let proposal:Option<String>=db.query_row("select proposal_id from agent_note_patch_candidates where conversation_id=?1 and state='pending' and (?2 is null or page_id=?2) order by generation desc limit 1",params![conversation.to_string(),page.map(|id|id.to_string())],|row|row.get(0)).optional().map_err(|error|error.to_string())?;
    let Some(proposal) = proposal else {
        return Ok(None);
    };
    let (generation, mut notes) = group(db, conversation, &proposal)?;
    if let Some(page) = page {
        let id = page.to_string();
        notes.retain(|note| {
            note.candidate
                .as_ref()
                .or(note.baseline.as_ref())
                .is_some_and(|value| value.page_id == id)
        });
    }
    let baseline_matches = matches(db, &notes);
    let whole_undo_available = !notes.iter().any(|note| note.manual_edited);
    let pages = notes
        .iter()
        .map(|note| super::note_review::views::with_authority(db, note))
        .collect::<Result<_, String>>()?;
    Ok(Some(Review {
        persistence_version: if notes.iter().any(|note| note.saved.is_some()) {
            2
        } else {
            1
        },
        proposal_id: proposal,
        generation,
        pages,
        baseline_matches,
        state: "pending".into(),
        operation_generation: generation,
        whole_undo_available,
    }))
}

#[tauri::command]
pub async fn agent_read_text_patch_proposal(
    store: tauri::State<'_, AppStore>,
    conversation_id: Uuid,
    proposal_id: Option<String>,
    generation: Option<i64>,
    page_id: Option<Uuid>,
) -> Result<Option<Review>, String> {
    let store = store.inner().clone();
    tokio::task::spawn_blocking(move || {
        let db = store.connection().map_err(|error| error.to_string())?;
        if let Some(proposal) = proposal_id.as_deref() {
            let generation = match generation {
                Some(generation) => Some(generation),
                None => {
                    super::note_review::snapshots::first_generation(&db, conversation_id, proposal)?
                }
            };
            if let Some(generation) = generation {
                return super::note_review::snapshots::read(
                    &db,
                    conversation_id,
                    proposal,
                    generation,
                );
            }
        }
        review_selected(&db, conversation_id, proposal_id.as_deref(), page_id)
    })
    .await
    .map_err(|_| "proposal read failed")?
}
#[tauri::command]
pub fn agent_note_text_patch_conversations(
    store: tauri::State<'_, AppStore>,
    page_id: Uuid,
) -> Result<Vec<String>, String> {
    let db = store.connection().map_err(|error| error.to_string())?;
    let mut statement=db.prepare("select distinct p.conversation_id from agent_note_patch_candidates p join agent_conversations c on c.id=p.conversation_id where p.page_id=?1 and p.state in ('pending','accepted') and c.archived=0 order by p.conversation_id").map_err(|error|error.to_string())?;
    statement
        .query_map([page_id.to_string()], |row| row.get(0))
        .map_err(|error| error.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| error.to_string())
}
pub(crate) fn review_selected(
    db: &Connection,
    conversation: Uuid,
    proposal: Option<&str>,
    page: Option<Uuid>,
) -> Result<Option<Review>, String> {
    let pending = if page.is_some() {
        review_for_page(db, conversation, page)?
    } else {
        review(db, conversation)?
    };
    let latest: Option<String> = if proposal.is_none() && page.is_none() && pending.is_none() {
        db.query_row("select proposal_id from agent_note_patch_decisions where conversation_id=?1 order by rowid desc limit 1", [conversation.to_string()], |row| row.get(0))
            .optional().map_err(|error| error.to_string())?
    } else {
        None
    };
    let Some(proposal) = proposal.or(latest.as_deref()) else {
        return Ok(pending);
    };
    if pending
        .as_ref()
        .is_some_and(|review| review.proposal_id == proposal)
    {
        return Ok(pending);
    }
    let result:Option<String>=db.query_row("select result_json from agent_note_patch_decisions where conversation_id=?1 and proposal_id=?2 order by rowid desc limit 1",params![conversation.to_string(),proposal],|row|row.get(0)).optional().map_err(|error|error.to_string())?;
    let Some(result) = result else {
        let known:bool=db.query_row("select exists(select 1 from agent_text_patch_calls c,json_each(c.results_json,'$.targets') t where c.conversation_id=?1 and coalesce(json_extract(t.value,'$.changeId'),json_extract(t.value,'$.proposalId'))=?2)",params![conversation.to_string(),proposal],|row|row.get(0)).map_err(|error|error.to_string())?;
        return Ok(if known { pending } else { None });
    };
    let receipt: Receipt = serde_json::from_str(&result).map_err(|error| error.to_string())?;
    let baseline_matches = receipt.pages.iter().all(|page| {
        note::load(db, &page.page_id).ok().is_some_and(|current| {
            serde_json::to_value(current).ok() == serde_json::to_value(page).ok()
        })
    });
    let pages = receipt
        .originals
        .iter()
        .map(page_review)
        .collect::<Result<_, String>>()?;
    Ok(Some(Review {
        persistence_version: if receipt.originals.iter().any(|note| note.saved.is_some()) {
            2
        } else {
            1
        },
        proposal_id: proposal.into(),
        generation: receipt.generation,
        pages,
        baseline_matches,
        whole_undo_available: receipt.action == "accept"
            && baseline_matches
            && !receipt.originals.iter().any(|note| note.manual_edited),
        operation_generation: receipt.generation,
        state: receipt.action,
    }))
}
#[tauri::command]
pub async fn agent_decide_text_patch(
    store: tauri::State<'_, AppStore>,
    request: Decision,
) -> Result<Receipt, super::decision_requests::Failure> {
    super::decision_requests::run(store.inner().clone(), request).await
}
pub(super) async fn execute(store: AppStore, request: Decision) -> Result<Receipt, String> {
    {
        let db = store.connection().map_err(|error| error.to_string())?;
        if let Some(receipt) = existing(&db, &request)? {
            return Ok(receipt);
        }
    }
    let guard = {
        let targets = {
            let db = store.connection().map_err(|error| error.to_string())?;
            let notes = if request.action == "undo" {
                let value: String = db.query_row("select result_json from agent_note_patch_decisions where conversation_id=?1 and proposal_id=?2 and action='accept' and coalesce(json_extract(result_json,'$.generation'),0)=?3 order by rowid desc limit 1", params![request.conversation_id.to_string(),request.proposal_id,request.generation], |row| row.get(0)).map_err(|_| "accepted proposal generation is unavailable")?;
                serde_json::from_str::<Receipt>(&value)
                    .map_err(|error| error.to_string())?
                    .originals
            } else {
                group(&db, request.conversation_id, &request.proposal_id)?.1
            };
            notes
                .iter()
                .filter_map(|note| note.baseline.as_ref().or(note.candidate.as_ref()))
                .map(|page| page.page_id.clone())
                .collect::<std::collections::BTreeSet<_>>()
        };
        Some(store.text_editors().acquire_pages(Some(&targets)).await?)
    };
    let receipt = tokio::task::spawn_blocking(move || decide(&store, request))
        .await
        .map_err(|_| "proposal decision did not confirm completion")??;
    if let Some(guard) = guard {
        guard.complete(receipt.pages.clone());
    }
    Ok(receipt)
}
pub(crate) fn decide(store: &AppStore, request: Decision) -> Result<Receipt, String> {
    if request.request_id.is_nil()
        || !["accept", "reject", "undo"].contains(&request.action.as_str())
    {
        return Err("invalid proposal decision".into());
    }
    let mut db = store.connection().map_err(|error| error.to_string())?;
    let tx = db.transaction().map_err(|error| error.to_string())?;
    let receipt = decide_in(store, &tx, request)?;
    tx.commit().map_err(|error| error.to_string())?;
    Ok(receipt)
}
pub(super) fn decide_in(
    store: &AppStore,
    tx: &rusqlite::Transaction<'_>,
    request: Decision,
) -> Result<Receipt, String> {
    active(tx, request.conversation_id)?;
    if let Some(receipt) = existing(tx, &request)? {
        return Ok(receipt);
    }
    if request.action == "undo" {
        return undo(store, tx, &request);
    }
    let (generation, mut notes) = group(tx, request.conversation_id, &request.proposal_id)?;
    if generation != request.generation {
        return Err("proposal changed; review the latest generation".into());
    }
    if request.action == "reject" && notes.iter().any(|note| note.manual_edited) {
        return Err(
            "candidate contains newer manual edits; use inline Undo to preserve input".into(),
        );
    }
    let pages = if request.action == "accept" {
        if !matches(tx, &notes) {
            return Err("saved note changed; preserve edits and regenerate the proposal".into());
        }
        authorize(store, tx, request.conversation_id, &notes, false)?;
        if notes.iter().any(|note| note.saved.is_some()) {
            for note in &mut notes {
                if super::note_review::resolved(note) {
                    continue;
                }
                note.review_document = note.candidate.as_ref().map(|page| page.document.clone());
                note.review_metadata = Some(super::note_review::metadata(note.candidate.as_ref()));
                note.reviewed_metadata = true;
            }
            super::note_review::persistence::checkpoint(tx, &mut notes)?;
        } else {
            let changes = notes
                .iter()
                .filter(|note| {
                    serde_json::to_value(&note.baseline).ok()
                        != serde_json::to_value(&note.candidate).ok()
                })
                .map(|note| PageChange {
                    before: note.baseline.clone(),
                    after: note.candidate.clone(),
                })
                .collect::<Vec<_>>();
            apply_changes(tx, &changes).map_err(|error| error.to_string())?;
        }
        notes
            .iter()
            .map(|note| {
                let page = note
                    .candidate
                    .as_ref()
                    .or(note.baseline.as_ref())
                    .ok_or("missing note identity")?;
                note::load(tx, &page.page_id)
            })
            .collect::<Result<Vec<_>, String>>()?
    } else if notes.iter().any(|note| note.saved.is_some()) {
        if !matches(tx, &notes) {
            return Err("saved note changed; preserve input before Undo".into());
        }
        authorize(store, tx, request.conversation_id, &notes, true)?;
        let mut originals = notes.clone();
        for note in &mut originals {
            let document = super::note_review::baseline_document(note);
            // Review Undo affects content. Created page identities and their
            // ordinary creation entrances are not body candidates.
            super::note_review::install_document(note, document);
        }
        super::note_review::persistence::checkpoint(tx, &mut originals)?
    } else {
        vec![]
    };
    // A whole-operation decision explicitly settles the resulting body. The
    // deferred-save branch predates checkpoint(), but must publish the same B.
    for page in &pages {
        crate::notes::hbs::synchronize_baseline_if_initialized(tx, &page.page_id, &page.document)?;
    }
    tx.execute("update agent_note_patch_candidates set state=?3 where conversation_id=?1 and proposal_id=?2",params![request.conversation_id.to_string(),request.proposal_id,if request.action=="accept"{"accepted"}else{"rejected"}]).map_err(|error|error.to_string())?;
    if request.action == "accept" {
        for note in &notes {
            tx.execute("update agent_note_patch_candidates set candidate_json=?3 where conversation_id=?1 and page_id=?2", params![request.conversation_id.to_string(), note.candidate.as_ref().or(note.baseline.as_ref()).ok_or("missing page")?.page_id, serde_json::to_string(note).map_err(|error|error.to_string())?]).map_err(|error|error.to_string())?;
        }
    }
    if request.action == "reject" {
        tx.execute("delete from agent_note_patch_assets where conversation_id=?1 and page_id in(select page_id from agent_note_patch_candidates where conversation_id=?1 and proposal_id=?2)",params![request.conversation_id.to_string(),request.proposal_id]).map_err(|error|error.to_string())?;
    }
    let receipt = Receipt {
        request_id: request.request_id,
        proposal_id: request.proposal_id.clone(),
        action: request.action.clone(),
        generation: request.generation,
        pages,
        originals: notes,
    };
    persist(tx, &request, &receipt)?;
    Ok(receipt)
}
pub(super) fn existing(db: &Connection, request: &Decision) -> Result<Option<Receipt>, String> {
    active(db, request.conversation_id)?;
    let previous:Option<(String,String,String)>=db.query_row("select conversation_id,proposal_id,result_json from agent_note_patch_decisions where request_id=?1",[request.request_id.to_string()],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).optional().map_err(|error|error.to_string())?;
    if let Some((conversation, proposal, result)) = previous {
        let receipt: Receipt = serde_json::from_str(&result).map_err(|error| error.to_string())?;
        if conversation != request.conversation_id.to_string()
            || proposal != request.proposal_id
            || receipt.action != request.action
            || receipt.generation != request.generation
        {
            return Err("request ID belongs to a different decision".into());
        }
        return Ok(Some(receipt));
    }
    Ok(None)
}
fn persist(db: &Connection, request: &Decision, receipt: &Receipt) -> Result<(), String> {
    db.execute("insert into agent_note_patch_decisions(request_id,conversation_id,proposal_id,action,result_json) values(?1,?2,?3,?4,?5)",params![request.request_id.to_string(),request.conversation_id.to_string(),request.proposal_id,request.action,serde_json::to_string(receipt).map_err(|error|error.to_string())?]).map_err(|error|error.to_string())?;
    Ok(())
}
fn undo(
    store: &AppStore,
    tx: &rusqlite::Transaction<'_>,
    request: &Decision,
) -> Result<Receipt, String> {
    let accepted:String=tx.query_row("select result_json from agent_note_patch_decisions where conversation_id=?1 and proposal_id=?2 and action='accept' and coalesce(json_extract(result_json,'$.generation'),0)=?3 order by rowid desc limit 1",params![request.conversation_id.to_string(),request.proposal_id,request.generation],|row|row.get(0)).map_err(|_|"accepted proposal generation is unavailable")?;
    let accepted: Receipt = serde_json::from_str(&accepted).map_err(|error| error.to_string())?;
    if super::note_review::manual_pending(tx, &accepted.pages)? {
        return Err(
            "note has newer manual candidate input; whole-group Undo cannot erase it".into(),
        );
    }
    let notes = accepted.originals;
    if notes.iter().any(|note| note.manual_edited) {
        return Err("note contains newer manual edits; whole-group Undo cannot erase them".into());
    }
    let originals = notes.clone();
    authorize(store, tx, request.conversation_id, &notes, true)?;
    if notes.len() != accepted.pages.len() {
        return Err("proposal undo metadata no longer matches".into());
    }
    let changes = notes
        .into_iter()
        .map(|note| {
            let original = note
                .baseline
                .as_ref()
                .or(note.candidate.as_ref())
                .ok_or("missing page")?;
            let before = accepted
                .pages
                .iter()
                .find(|page| page.page_id == original.page_id)
                .ok_or("missing accepted page")?
                .clone();
            Ok(PageChange {
                before: Some(before),
                after: note.baseline,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let pages = apply_changes(tx, &changes).map_err(|error| error.to_string())?;
    for page in &pages {
        crate::notes::hbs::synchronize_baseline_if_initialized(tx, &page.page_id, &page.document)?;
    }
    tx.execute("update agent_note_patch_candidates set state='undone' where conversation_id=?1 and proposal_id=?2",params![request.conversation_id.to_string(),request.proposal_id]).map_err(|error|error.to_string())?;
    let receipt = Receipt {
        request_id: request.request_id,
        proposal_id: request.proposal_id.clone(),
        action: "undo".into(),
        generation: request.generation,
        pages,
        originals,
    };
    persist(tx, request, &receipt)?;
    Ok(receipt)
}
