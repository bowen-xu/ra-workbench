use super::*;
use crate::hbs_storage::{self, Commit};

const PAGE: &str = "00000000-0000-4000-8000-000000000186";
fn setup() -> (Connection, StoredNote) {
    let mut db = Connection::open_in_memory().unwrap();
    crate::storage::migrate(&mut db).unwrap();
    let document = json!({"schemaVersion":1,"pageId":PAGE,"blocks":[{"id":"one","type":"paragraph","props":{},"content":{"kind":"inline","items":[{"type":"text","text":"baseline","styles":[]}]},"children":[]}]});
    crate::notes::hbs::seed(&mut db, PAGE, serde_json::from_value(document).unwrap()).unwrap();
    let original = note::load(&db, PAGE).unwrap();
    (db, original)
}
fn changed(page: &StoredNote, text: &str) -> StoredNote {
    let mut page = page.clone();
    page.document.blocks[0].content["items"][0]["text"] = json!(text);
    page
}
fn hbs_document(page: &StoredNote) -> Value {
    let mut value = serde_json::to_value(&page.document).unwrap();
    value["title"] = json!(page.title);
    value
}
fn pending(original: &StoredNote, candidate: StoredNote, saved: Option<StoredNote>) -> PendingNote {
    PendingNote {
        path: format!("source://notes/{PAGE}"),
        original_path: format!("source://notes/{PAGE}"),
        scope_request_id: Uuid::new_v4(),
        baseline: Some(original.clone()),
        candidate: Some(candidate),
        saved,
        review_document: Some(original.document.clone()),
        review_metadata: Some(metadata(Some(original))),
        reviewed_metadata: false,
        manual_edited: false,
        imports: vec![],
    }
}
fn insert(db: &Connection, note: &PendingNote) {
    db.execute(
        "insert into agent_conversations(id,title) values('conversation','HBS legacy fixture')",
        [],
    )
    .unwrap();
    db.execute("insert into agent_note_patch_candidates(conversation_id,page_id,proposal_id,generation,baseline_json,candidate_json,state,persistence_version) values('conversation',?1,'proposal',7,?2,?3,'pending',?4)",params![PAGE,serde_json::to_string(&note.baseline).unwrap(),serde_json::to_string(note).unwrap(),if note.saved.is_some(){2}else{1}]).unwrap();
    db.execute("insert into agent_note_patch_snapshots(conversation_id,proposal_id,generation,notes_json) values('conversation','proposal',7,?1)",[serde_json::to_string(&[note]).unwrap()]).unwrap();
}
fn commit(
    db: &mut Connection,
    request: &str,
    snapshot: &Value,
    saved: Value,
    baseline: Value,
) -> Value {
    crate::hbs_storage::test_sources::commit(
        db,
        "note",
        PAGE,
        Commit {
            id: request.into(),
            expected_version: snapshot["version"].as_i64().unwrap(),
            saved,
            baseline,
        },
    )
    .unwrap()["snapshot"]
        .clone()
}

#[test]
fn deferred_legacy_candidate_is_recovered_without_acceptance_or_rewriting_s() {
    let (mut db, original) = setup();
    let candidate = changed(&original, "unsaved legacy candidate");
    let pending = pending(&original, candidate.clone(), None);
    insert(&db, &pending);
    db.execute("delete from hbs_note_baselines", []).unwrap();
    let snapshot = hbs_storage::read(&db, "note", PAGE).unwrap();
    assert_eq!(snapshot["saved"], hbs_document(&original));
    assert_eq!(snapshot["baseline"], hbs_document(&original));
    let recovered = crate::hbs_storage::test_sources::preservation_read(&db, "note", PAGE).unwrap();
    assert_eq!(recovered["content"], hbs_document(&candidate));
    assert_eq!(recovered["reviewAncestor"], recovered["content"]);
    let saved = commit(
        &mut db,
        "save-recovered",
        &snapshot,
        recovered["content"].clone(),
        snapshot["baseline"].clone(),
    );
    assert_ne!(saved["saved"], saved["baseline"], "saving is not Keep");
    let live = active(&db, PAGE).unwrap().pop().unwrap().2;
    assert!(live.manual_edited);
    assert!(!resolved(&live));
    assert_eq!(
        serde_json::to_value(&live.review_document).unwrap(),
        hbs_document(&original)
    );
}

#[test]
fn hbs_and_agent_share_live_baseline_but_metadata_and_frozen_history_are_preserved() {
    let (mut db, original) = setup();
    let mut group = vec![pending(
        &original,
        changed(&original, "Agent new"),
        Some(original.clone()),
    )];
    let tx = db.transaction().unwrap();
    super::super::persistence::checkpoint(&tx, &mut group).unwrap();
    tx.commit().unwrap();
    let mut live = group.pop().unwrap();
    live.candidate.as_mut().unwrap().title = "pending title metadata".into();
    insert(&db, &live);
    let frozen: String = db
        .query_row(
            "select notes_json from agent_note_patch_snapshots",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let snapshot = hbs_storage::read(&db, "note", PAGE).unwrap();
    assert_eq!(snapshot["baseline"], hbs_document(&original));
    assert_eq!(
        snapshot["saved"]["blocks"][0]["content"]["items"][0]["text"],
        "Agent new"
    );
    // Import an existing persisted proposal without an HBS baseline row.
    db.execute("delete from hbs_note_baselines", []).unwrap();
    assert_eq!(hbs_storage::read(&db, "note", PAGE).unwrap(), snapshot);
    let kept = commit(
        &mut db,
        "keep-body",
        &snapshot,
        snapshot["saved"].clone(),
        snapshot["saved"].clone(),
    );
    let live = active(&db, PAGE).unwrap().pop().unwrap().2;
    assert_eq!(
        live.candidate.as_ref().unwrap().title,
        "pending title metadata"
    );
    assert!(!resolved(&live), "body Keep cannot accept page metadata");
    assert_eq!(
        serde_json::to_value(&live.review_document).unwrap(),
        kept["saved"]
    );
    assert_eq!(
        db.query_row("select state from agent_note_patch_candidates", [], |r| {
            r.get::<_, String>(0)
        })
        .unwrap(),
        "pending"
    );
    assert_eq!(
        db.query_row(
            "select notes_json from agent_note_patch_snapshots",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        frozen
    );
    // Another Agent checkpoint must adopt the newly reviewed B, never leave
    // the earlier HBS baseline behind while advancing real note_blocks.
    let mut group = vec![live];
    group[0].candidate = Some(changed(group[0].saved.as_ref().unwrap(), "Agent next"));
    let tx = db.transaction().unwrap();
    super::super::persistence::checkpoint(&tx, &mut group).unwrap();
    tx.commit().unwrap();
    let latest = hbs_storage::read(&db, "note", PAGE).unwrap();
    assert_eq!(latest["baseline"], kept["saved"]);
    assert_eq!(
        latest["saved"]["blocks"][0]["content"]["items"][0]["text"],
        "Agent next"
    );
}

#[test]
fn failed_legacy_synchronization_rolls_back_note_baseline_and_dedup_receipt() {
    let (mut db, original) = setup();
    insert(
        &db,
        &pending(&original, original.clone(), Some(original.clone())),
    );
    db.execute_batch("create trigger fail_legacy_update before update on agent_note_patch_candidates begin select raise(abort,'forced legacy failure'); end;").unwrap();
    let snapshot = hbs_storage::read(&db, "note", PAGE).unwrap();
    let candidate = changed(&original, "rollback");
    let result = crate::hbs_storage::test_sources::commit(
        &mut db,
        "note",
        PAGE,
        Commit {
            id: "rollback-live".into(),
            expected_version: original.revision,
            saved: serde_json::to_value(candidate.document).unwrap(),
            baseline: snapshot["baseline"].clone(),
        },
    );
    assert!(result.unwrap_err().contains("forced legacy failure"));
    assert_eq!(hbs_storage::read(&db, "note", PAGE).unwrap(), snapshot);
    assert_eq!(
        db.query_row(
            "select count(*) from hbs_commit_receipts where request_id='rollback-live'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}

#[test]
fn accepted_deferred_legacy_proposal_is_not_imported_as_pending_review() {
    let (mut db, original) = setup();
    let snapshot = hbs_storage::read(&db, "note", PAGE).unwrap();
    let candidate = changed(&original, "already accepted legacy candidate");
    let saved = commit(
        &mut db,
        "accepted-deferred",
        &snapshot,
        serde_json::to_value(&candidate.document).unwrap(),
        serde_json::to_value(&candidate.document).unwrap(),
    );
    insert(&db, &pending(&original, candidate, None));
    db.execute(
        "update agent_note_patch_candidates set state='accepted'",
        [],
    )
    .unwrap();
    db.execute("delete from hbs_note_baselines", []).unwrap();
    assert_eq!(
        hbs_storage::read(&db, "note", PAGE).unwrap()["baseline"],
        saved["saved"]
    );
    assert!(
        crate::hbs_storage::test_sources::preservation_read(&db, "note", PAGE)
            .unwrap()
            .is_null()
    );
}
