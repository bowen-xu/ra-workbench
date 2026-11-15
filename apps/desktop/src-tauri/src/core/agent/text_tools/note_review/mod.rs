//! User-only rich review owner. D/B are distinct from the formal CAS baseline S.
#[path = "chunks.rs"]
pub(super) mod chunks;
#[path = "creation.rs"]
mod creation;
#[path = "hbs.rs"]
mod hbs;
#[path = "history.rs"]
mod history;
#[path = "retain_current.rs"]
mod retain_current;
pub(crate) use hbs::{legacy_hbs_baseline, legacy_hbs_checkpoint, synchronize_hbs_commit};
#[path = "navigation.rs"]
mod navigation;
pub(crate) use navigation::{check_navigation, synchronize_navigation};
#[path = "persistence.rs"]
pub(super) mod persistence;
pub(crate) use persistence::synchronize_metadata;
#[path = "projection.rs"]
mod projection;
#[path = "snapshots.rs"]
pub(super) mod snapshots;
#[path = "views.rs"]
pub(super) mod views;
use super::{
    decisions,
    note::{PendingNote, StoredNote},
    *,
};
use crate::notes::NoteDocument;
use rusqlite::{Connection, params};

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Request {
    pub conversation_id: Uuid,
    pub proposal_id: String,
    pub generation: i64,
    pub request_id: Uuid,
    pub page_id: String,
    pub action: String,
    pub chunk_id: Option<String>,
    pub document: Option<NoteDocument>,
    pub history_request_id: Option<Uuid>,
    pub direction: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_revision: Option<i64>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Receipt {
    pub request_id: Uuid,
    pub proposal_id: String,
    pub action: String,
    pub generation: i64,
    pub review: Option<decisions::Review>,
    pub pages: Vec<StoredNote>,
}
pub(super) fn metadata_equal(a: Option<&StoredNote>, b: Option<&StoredNote>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => {
            a.page_id == b.page_id
                && a.title == b.title
                && a.parent_id == b.parent_id
                && a.position == b.position
                && a.status == b.status
        }
        _ => false,
    }
}
pub(super) fn metadata_pending(note: &PendingNote) -> bool {
    // A formally created page already exists, exactly like a UI-created page.
    // Content review must not require a second acceptance of its identity.
    if note.saved.is_some() {
        return false;
    }
    !note.reviewed_metadata && baseline_metadata(note) != metadata(note.candidate.as_ref())
}
pub(super) fn metadata(page: Option<&StoredNote>) -> note::ReviewMetadata {
    note::ReviewMetadata {
        exists: page.is_some(),
        title: page.map(|p| p.title.clone()),
        parent_id: page.and_then(|p| p.parent_id.clone()),
        position: page.map_or(0, |p| p.position),
        status: page.map(|p| p.status.clone()),
    }
}
pub(super) fn baseline_metadata(note: &PendingNote) -> note::ReviewMetadata {
    note.review_metadata
        .clone()
        .unwrap_or_else(|| metadata(note.baseline.as_ref()))
}
pub(super) fn baseline_document(note: &PendingNote) -> NoteDocument {
    let mut document = note
        .review_document
        .clone()
        .or_else(|| note.baseline.as_ref().map(|p| p.document.clone()))
        .unwrap_or_else(|| {
            let page = note.candidate.as_ref().expect("pending identity");
            let mut document = chunks::empty(&page.page_id);
            document.title = page
                .document
                .title
                .clone()
                .or_else(|| Some(page.title.clone()));
            document
        });
    if document.title.is_none() {
        document.title = baseline_metadata(note)
            .title
            .or_else(|| note.baseline.as_ref().map(|page| page.title.clone()))
            .or_else(|| note.candidate.as_ref().map(|page| page.title.clone()));
    }
    document
}
fn id(note: &PendingNote) -> &str {
    &note
        .candidate
        .as_ref()
        .or(note.baseline.as_ref())
        .expect("pending identity")
        .page_id
}
pub(super) fn resolved(note: &PendingNote) -> bool {
    !metadata_pending(note)
        && chunks::equal(
            &baseline_document(note),
            &note
                .candidate
                .as_ref()
                .map(|p| p.document.clone())
                .unwrap_or_else(|| chunks::empty(id(note))),
        )
}
fn unchanged(note: &PendingNote) -> bool {
    metadata_equal(note.baseline.as_ref(), note.candidate.as_ref())
        && match (&note.baseline, &note.candidate) {
            (Some(a), Some(b)) => chunks::equal(&a.document, &b.document),
            (None, None) => true,
            _ => false,
        }
}
pub(super) fn manual_pending(db: &Connection, pages: &[StoredNote]) -> Result<bool, String> {
    for page in pages {
        let exists: bool = db.query_row("select exists(select 1 from agent_note_patch_candidates where page_id=?1 and state='pending' and json_extract(candidate_json,'$.manualEdited')=1)", [&page.page_id], |row|row.get(0)).map_err(|error|error.to_string())?;
        if exists {
            return Ok(true);
        }
    }
    Ok(false)
}
fn validate(request: &Request) -> Result<(), String> {
    if request.request_id.is_nil()
        || request.conversation_id.is_nil()
        || request.generation < 1
        || request.proposal_id.is_empty()
        || request.proposal_id.len() > 256
        || request
            .chunk_id
            .as_ref()
            .is_some_and(|id| id.is_empty() || id.len() > 128)
        || Uuid::parse_str(&request.page_id).is_err()
        || !["keep", "undo", "edit", "restore", "retain-current"].contains(&request.action.as_str())
    {
        return Err("invalid note review request".into());
    }
    if matches!(request.action.as_str(), "edit" | "retain-current") != request.document.is_some()
        || (request.action == "restore") != request.history_request_id.is_some()
        || (request.action == "restore") != request.direction.is_some()
        || request.chunk_id.is_some() && !["keep", "undo"].contains(&request.action.as_str())
        || (request.action == "retain-current") != request.expected_revision.is_some()
        || request
            .expected_revision
            .is_some_and(|revision| revision < 0)
    {
        return Err("note review request payload does not match its action".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request() -> Request {
        Request {
            conversation_id: Uuid::new_v4(),
            proposal_id: "proposal".into(),
            generation: 1,
            request_id: Uuid::new_v4(),
            page_id: Uuid::new_v4().to_string(),
            action: "keep".into(),
            chunk_id: None,
            document: None,
            history_request_id: None,
            direction: None,
            expected_revision: None,
        }
    }
    #[test]
    fn legacy_request_roundtrip_keeps_the_deployed_fingerprint_shape() {
        let original = request();
        let raw = serde_json::to_string(&original).unwrap();
        assert!(!raw.contains("expectedRevision"));
        let restored: Request = serde_json::from_str(&raw).unwrap();
        assert_eq!(serde_json::to_string(&restored).unwrap(), raw);
        assert!(validate(&restored).is_ok());
        let mut invalid = restored;
        invalid.action = "retain-current".into();
        assert!(validate(&invalid).is_err());
    }
    #[test]
    fn rejects_unrelated_or_unbounded_private_ipc_payloads_before_storage_access() {
        let mut orphan = request();
        orphan.direction = Some("undo".into());
        assert!(validate(&orphan).is_err());
        let mut orphan = request();
        orphan.history_request_id = Some(Uuid::new_v4());
        assert!(validate(&orphan).is_err());
        let mut wrong = request();
        wrong.action = "edit".into();
        assert!(validate(&wrong).is_err());
        let mut huge = request();
        huge.proposal_id = "p".repeat(257);
        assert!(validate(&huge).is_err());
        let mut wrong = request();
        wrong.chunk_id = Some(String::new());
        assert!(validate(&wrong).is_err());
    }
}
#[tauri::command]
pub async fn agent_resolve_note_text_patch(
    store: tauri::State<'_, AppStore>,
    request: Request,
) -> Result<Receipt, super::decision_requests::Failure> {
    resolve(store.inner().clone(), request)
        .await
        .map_err(|message| super::decision_requests::Failure {
            status: if message.contains("completion is unknown") {
                "unknown"
            } else {
                "failed"
            },
            message,
        })
}
pub(crate) async fn resolve(store: AppStore, request: Request) -> Result<Receipt, String> {
    validate(&request)?;
    if request.action == "retain-current" {
        return tokio::task::spawn_blocking(move || retain_current::resolve_in(&store, request))
            .await
            .map_err(|_| {
                "MODEL_NOTE_RECOVERY_UNKNOWN: completion is unknown; retry the original request"
                    .to_owned()
            })?;
    }
    if let Some(receipt) = creation::settle_published_creation(&store, &request)? {
        return Ok(receipt);
    }
    let targets = {
        let db = store.connection().map_err(|e| e.to_string())?;
        if let Some(receipt) = history::existing(&db, &request)? {
            return Ok(receipt);
        }
        if request.action == "restore" {
            history::targets(&db, &request)?
        } else {
            decisions::group(&db, request.conversation_id, &request.proposal_id)?
                .1
                .iter()
                .map(|note| id(note).to_owned())
                .collect()
        }
    };
    // Candidate typing already uses the mounted save owner. Do not recursively
    // flush that owner while it is committing its candidate edit.
    let guard = if request.action == "edit" {
        None
    } else {
        Some(store.text_editors().acquire_pages(Some(&targets)).await?)
    };
    let worker_store = store.clone();
    let origin = if request.action == "restore" {
        "restore"
    } else {
        "review"
    };
    let history = request.action != "restore";
    let request_id = request.request_id;
    let receipt = tokio::task::spawn_blocking(move || resolve_in(&worker_store, request))
        .await
        .map_err(|_| "note review completion is unknown; retry the same request identity")??;
    if let Some(guard) = guard {
        let reviews = receipt
            .review
            .as_ref()
            .map(|review| {
                review
                    .pages
                    .iter()
                    .filter_map(|page| {
                        page.review_document.as_ref().map(|baseline| {
                            (
                                page.page_id.clone(),
                                editor_barrier::ReviewInstall {
                                    baseline: baseline.clone(),
                                    origin: origin.into(),
                                    request_id: history.then_some(request_id),
                                    add_to_history: history,
                                },
                            )
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        guard.complete_review(receipt.pages.clone(), reviews);
    }
    Ok(receipt)
}
pub(crate) fn resolve_in(store: &AppStore, request: Request) -> Result<Receipt, String> {
    validate(&request)?;
    if request.action == "retain-current" {
        return retain_current::resolve_in(store, request);
    }
    let mut db = store.connection().map_err(|e| e.to_string())?;
    let tx = db
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    if let Some(receipt) = history::existing(&tx, &request)? {
        return Ok(receipt);
    }
    let (before, mut group) = if request.action == "restore" {
        history::restore(&tx, &request)?
    } else {
        let (generation, group) = decisions::group_for_edit(
            &tx,
            request.conversation_id,
            &request.proposal_id,
            request.action == "edit",
        )?;
        if generation != request.generation {
            return Err("proposal changed; read the latest review before deciding".into());
        }
        (group.clone(), group)
    };
    if !decisions::matches(&tx, &group) {
        return Err(
            "saved note changed; preserve current input and compare the saved version".into(),
        );
    }
    if request.action != "restore" {
        let selected = group
            .iter_mut()
            .find(|note| id(note) == request.page_id)
            .ok_or("page is outside this proposal")?;
        mutate(selected, &request)?;
    }
    // Validation runs over the complete related group before any formal write.
    decisions::authorize(store, &tx, request.conversation_id, &group, false)?;
    for note in &group {
        if let Some(page) = &note.candidate {
            let mapping = note
                .baseline
                .as_ref()
                .map(|p| p.document.clone())
                .unwrap_or_else(|| page.document.clone());
            crate::notes::document_validation::validate_preserving(&page.document, &mapping)?;
            let text = crate::notes::text_view::render(&page.document, &page.title)?;
            let (title, parsed) = crate::notes::text_view::parse(&page.page_id, &text)?;
            if crate::notes::text_view::render(&parsed, &title)? != text {
                return Err(
                    "candidate cannot be projected losslessly; preserve editor input".into(),
                );
            }
        }
    }
    let generation = request
        .generation
        .checked_add(1)
        .ok_or("proposal generation overflow")?;
    let mut pages = if group.iter().any(|note| note.saved.is_some()) {
        persistence::checkpoint(&tx, &mut group)?
    } else {
        vec![]
    };
    persist_group(&tx, &request, generation, &group)?;
    let complete = group.iter().all(resolved);
    if complete {
        let action = if request.action != "restore"
            && group.iter().all(unchanged)
            && !group.iter().any(|note| note.manual_edited)
        {
            "reject"
        } else {
            "accept"
        };
        let completed = decisions::decide_in(
            store,
            &tx,
            decisions::Decision {
                conversation_id: request.conversation_id,
                proposal_id: request.proposal_id.clone(),
                generation,
                request_id: request.request_id,
                action: action.into(),
            },
        )?
        .pages;
        if pages.is_empty() {
            pages = completed;
        }
    }
    let review = decisions::review_selected(
        &tx,
        request.conversation_id,
        Some(&request.proposal_id),
        None,
    )?;
    let receipt = Receipt {
        request_id: request.request_id,
        proposal_id: request.proposal_id.clone(),
        action: request.action.clone(),
        generation,
        review,
        pages,
    };
    history::persist(&tx, &request, &receipt, &before, &group)?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(receipt)
}
fn mutate(note: &mut PendingNote, request: &Request) -> Result<(), String> {
    if request.action == "undo"
        && request.chunk_id.is_none()
        && !baseline_metadata(note).exists
        && note.saved.is_none()
    {
        return Err("new page and its parent relationships require whole-group Undo".into());
    }
    let mut baseline = baseline_document(note);
    let mut candidate = note
        .candidate
        .as_ref()
        .map(|p| p.document.clone())
        .unwrap_or_else(|| chunks::empty(id(note)));
    if request.action == "edit" {
        let edited = projection::preserve(
            &candidate,
            request.document.clone().ok_or("missing editor document")?,
        )?;
        note.manual_edited |= !chunks::equal(&candidate, &edited);
        if note.candidate.is_none() {
            return Err("a recycled candidate has no editable body".into());
        }
        baseline = chunks::synchronize(&baseline, &candidate, &edited)?;
        candidate = edited;
    } else {
        chunks::resolve(
            &mut baseline,
            &mut candidate,
            request.chunk_id.as_deref(),
            request.action == "keep",
        )?;
        if request.chunk_id.is_none() && note.saved.is_none() {
            if request.action == "undo" {
                // Metadata belongs to the page/relationship group. Undo all cannot
                // retain a moved parent or created identity accidentally.
                let original = baseline_metadata(note);
                if note.candidate.is_none() {
                    note.candidate = note.baseline.clone();
                }
                let page = note
                    .candidate
                    .as_mut()
                    .ok_or("metadata restore lost the page identity")?;
                page.title = original.title.ok_or("metadata baseline lost its title")?;
                page.parent_id = original.parent_id;
                page.position = original.position;
                page.status = original.status.ok_or("metadata baseline lost its status")?;
            }
            note.review_metadata = Some(metadata(note.candidate.as_ref()));
            note.reviewed_metadata = true;
        }
    }
    install_document(note, candidate);
    note.review_document = Some(baseline);
    Ok(())
}
/// HBS content decisions carry their title in the canonical document. Project
/// its display label when installing it; text-tool authoring retains its own
/// explicit title envelope until the ordinary repository publication step.
pub(super) fn install_document(note: &mut PendingNote, document: NoteDocument) {
    if let Some(page) = &mut note.candidate {
        if let Some(title) = &document.title {
            page.title = crate::notes::validate_title(title).unwrap_or_else(|| "无标题".into());
        }
        page.document = document;
    }
}
fn persist_group(
    db: &Connection,
    request: &Request,
    generation: i64,
    group: &[PendingNote],
) -> Result<(), String> {
    for note in group {
        db.execute("update agent_note_patch_candidates set generation=?3,baseline_json=?4,candidate_json=?5,state='pending',persistence_version=?7 where conversation_id=?1 and page_id=?2 and proposal_id=?6", params![request.conversation_id.to_string(), id(note), generation, serde_json::to_string(&note.baseline).map_err(|e|e.to_string())?, serde_json::to_string(note).map_err(|e|e.to_string())?, request.proposal_id, if note.saved.is_some() {2} else {1}]).map_err(|e| e.to_string())?;
    }
    Ok(())
}
