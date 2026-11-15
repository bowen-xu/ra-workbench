//! Immutable original save facts, referenced by the note transaction's receipt.
use super::*;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Reference {
    schema_version: u8,
    proposal_id: String,
    generation: i64,
    page_ids: BTreeSet<String>,
    review_baselines: BTreeMap<String, NoteDocument>,
}
pub(in crate::agent::text_tools) struct Delivery {
    pub pages: Vec<StoredNote>,
    pub reviews: BTreeMap<String, editor_barrier::ReviewInstall>,
}
#[derive(Debug)]
pub(in crate::agent::text_tools) enum Error {
    Sql(rusqlite::Error),
    Owner(crate::storage::StorageError),
    Missing,
    Invalid,
    Publish,
    Transaction,
}
impl From<rusqlite::Error> for Error {
    fn from(error: rusqlite::Error) -> Self {
        Self::Sql(error)
    }
}
pub(super) fn freeze(
    notes: &mut [PendingNote],
    saved: &[PendingNote],
    proposal: &str,
    generation: i64,
) -> Reference {
    let mut page_ids = BTreeSet::new();
    let mut review_baselines = BTreeMap::new();
    for note in notes {
        let id = note
            .candidate
            .as_ref()
            .or(note.baseline.as_ref())
            .map(|page| page.page_id.clone());
        note.saved = id.as_ref().and_then(|id| {
            saved.iter().find_map(|note| {
                note.saved
                    .as_ref()
                    .filter(|saved| &saved.page_id == id)
                    .cloned()
            })
        });
        if let Some(id) = id {
            if let Some(original) = saved
                .iter()
                .find(|note| note.saved.as_ref().is_some_and(|page| page.page_id == id))
            {
                // The history card's baseline is the preceding call's candidate;
                // editor delivery must retain the independently accepted B.
                review_baselines.insert(id.clone(), note_review::baseline_document(original));
            }
            page_ids.insert(id);
        }
    }
    Reference {
        schema_version: 1,
        proposal_id: proposal.into(),
        generation,
        page_ids,
        review_baselines,
    }
}

pub(in crate::agent::text_tools) fn read(
    db: &Connection,
    record: &SubmissionRecord,
    key: &str,
) -> Result<Option<Delivery>, Error> {
    if !db.is_autocommit() {
        return Err(Error::Transaction);
    }
    let result: Option<String> = db.query_row(
        "select results_json from agent_text_patch_calls where conversation_id=?1 and request_id=?2 and state='complete'",
        params![record.conversation_id.to_string(),key], |row|row.get(0),
    ).optional()?;
    let result: Value =
        serde_json::from_str(&result.ok_or(Error::Missing)?).map_err(|_| Error::Invalid)?;
    let Some(reference) = result.get("noteDelivery") else {
        return Ok(None);
    };
    let reference: Reference =
        serde_json::from_value(reference.clone()).map_err(|_| Error::Invalid)?;
    if reference.schema_version != 1
        || reference.generation < 1
        || reference.review_baselines.keys().collect::<BTreeSet<_>>()
            != reference.page_ids.iter().collect::<BTreeSet<_>>()
        || !result["targets"].as_array().is_some_and(|targets| {
            !targets.is_empty()
                && targets.iter().all(|target| {
                    target["changeId"] == reference.proposal_id
                        && target["generation"] == reference.generation
                        && target["status"] == "applied"
                })
        })
    {
        return Err(Error::Invalid);
    }
    let notes: Option<String> = db.query_row(
        "select notes_json from agent_note_patch_snapshots where conversation_id=?1 and proposal_id=?2 and generation=?3",
        params![record.conversation_id.to_string(),reference.proposal_id,reference.generation], |row|row.get(0),
    ).optional()?;
    let notes: Vec<PendingNote> =
        serde_json::from_str(&notes.ok_or(Error::Missing)?).map_err(|_| Error::Invalid)?;
    let mut pages = Vec::new();
    let mut reviews = BTreeMap::new();
    for note in notes {
        let page = note.saved.as_ref().ok_or(Error::Invalid)?;
        let id = note
            .candidate
            .as_ref()
            .or(note.baseline.as_ref())
            .ok_or(Error::Invalid)?;
        if page.page_id != id.page_id
            || page.document.page_id != page.page_id
            || !reference.page_ids.contains(&page.page_id)
            || reviews.contains_key(&page.page_id)
        {
            return Err(Error::Invalid);
        }
        let baseline = reference
            .review_baselines
            .get(&page.page_id)
            .ok_or(Error::Invalid)?;
        if baseline.page_id != page.page_id {
            return Err(Error::Invalid);
        }
        reviews.insert(
            page.page_id.clone(),
            editor_barrier::ReviewInstall {
                baseline: baseline.clone(),
                origin: "agent".into(),
                request_id: None,
                add_to_history: true,
            },
        );
        pages.push(page.clone());
    }
    if pages.len() != reference.page_ids.len() {
        return Err(Error::Invalid);
    }
    Ok(Some(Delivery { pages, reviews }))
}

pub(in crate::agent::text_tools) fn diagnostic(
    record: &SubmissionRecord,
    call: &str,
    error: &Error,
) -> Value {
    let key = format!("{call}:notes");
    let mut value = match error {
        Error::Sql(error) => {
            patch_journal::target_diagnostic(record, &key, "note-delivery-read", error)
        }
        Error::Owner(error) => {
            patch_journal::owner_diagnostic(record, &key, "note-delivery-read", error)
        }
        _ => patch_journal::unavailable_receipt(record, &key, None),
    };
    patch_journal::call_identity(&mut value, record, call);
    value["targetRequestId"] = json!(key);
    value["code"] = json!("TEXT_PATCH_DELIVERY_UNCONFIRMED");
    value["phase"] = json!(if matches!(error, Error::Publish) {
        "note-delivery-publish"
    } else {
        "note-delivery-read"
    });
    value["safeCause"] = json!(match error {
        Error::Sql(_) => "note_delivery_read_failed",
        Error::Owner(_) => "note_delivery_owner_unavailable",
        Error::Missing => "note_delivery_receipt_missing",
        Error::Invalid => "note_delivery_snapshot_invalid",
        Error::Publish => "note_delivery_publish_failed",
        Error::Transaction => "note_delivery_transaction_unconfirmed",
    });
    value
}
