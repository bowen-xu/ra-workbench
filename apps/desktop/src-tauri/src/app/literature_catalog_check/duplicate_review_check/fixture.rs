use super::*;
use rusqlite::params;

pub(super) const NOTE: &str = "duplicate-review-core-note";
pub(super) const BODY: &str = "Preserved synthetic Core-owned duplicate review body";
pub(super) const ONE: &str = "candidate-one";
pub(super) const NEAR: &str = "candidate-next-down";
pub(super) const POINT: &str = "candidate-point72";
pub(super) const IGNORE: &str = "candidate-ignore";
pub(super) const FAIL: &str = "candidate-fail";
pub(super) const PRIVATE_TRIGGER: &str = "private synthetic duplicate trigger text";

pub(super) fn seed(store: &AppStore) -> Result<Value, Error> {
    let mut db = store.connection()?;
    let tx = db.transaction()?;
    tx.execute_batch(
        "insert into bibliographic_items(id,item_type,title,created_at,updated_at,archived_at,trashed_at)
         values('duplicate-left','journalArticle','Synthetic duplicate left','2026','2026',null,null),
         ('duplicate-right','journalArticle','Synthetic duplicate right','2026','2026',null,null),
         ('duplicate-archived','journalArticle','Synthetic archived endpoint','2026','2026','2026',null),
         ('duplicate-trashed','journalArticle','Synthetic trashed endpoint','2026','2026',null,'2026');
         insert into merge_operations(id,survivor_item_id,merged_item_id,snapshot_json,state,created_at)
         values('history-active','duplicate-left','duplicate-right','{}','applied','2026-11-05T00:00:03Z'),
         ('history-archived','duplicate-left','duplicate-archived','{}','applied','2026-11-05T00:00:04Z'),
         ('history-trashed','duplicate-left','duplicate-trashed','{}','applied','2026-11-05T00:00:05Z'),
         ('history-undone','duplicate-left','duplicate-right','{}','undone','2026-11-05T00:00:06Z');
         create table synthetic_duplicate_review_effects(candidate_id text not null,kind text not null);
         create trigger synthetic_duplicate_review_ignore before update of decision on duplicate_candidates
         when old.id='candidate-ignore' begin
           insert into synthetic_duplicate_review_effects values(old.id,'ignore');
           select raise(ignore);
         end;
         create trigger synthetic_duplicate_review_fail before update of decision on duplicate_candidates
         when old.id='candidate-fail' begin
           insert into synthetic_duplicate_review_effects values(old.id,'fail');
           select raise(fail,'private synthetic duplicate trigger text');
         end;",
    )?;
    crate::notes::create_page_with_paper_in(
        &tx,
        crate::notes::NotePageDraft {
            title: "Synthetic duplicate review Core note".into(),
            parent_page_id: None,
            parent_block_position: None,
        },
        None,
        Some(NOTE),
    )?;
    let mut document = crate::notes::load_document_value(&tx, NOTE)?;
    document.blocks[0].content =
        json!({"kind":"inline","items":[{"type":"text","text":BODY,"styles":[]}]});
    crate::notes::save_document_in(
        &tx,
        crate::notes::NoteDocumentSaveRequest {
            document,
            expected_revision: 0,
            save_reason: "manual".into(),
        },
    )?;
    for (id, right, confidence, decision) in [
        (ONE, "duplicate-right", 1.0, None),
        (
            NEAR,
            "duplicate-right",
            f64::from_bits(0x3fef_ffff_ffff_ffff),
            None,
        ),
        (POINT, "duplicate-right", 0.72, None),
        ("candidate-archived", "duplicate-archived", 0.9, None),
        ("candidate-trashed", "duplicate-trashed", 0.9, None),
        (
            "candidate-already-decided",
            "duplicate-right",
            1.0,
            Some("keep_both"),
        ),
    ] {
        tx.execute(
            "insert into duplicate_candidates(id,left_item_id,right_item_id,reason,confidence,decision,created_at)
             values(?1,'duplicate-left',?2,'doi',?3,?4,'2026')",
            params![id, right, confidence, decision],
        )?;
    }
    tx.commit()?;
    drop(db);
    let document = note(store)?;
    if document["blocks"][0]["content"]["items"][0]["text"] != BODY {
        return Err("original Core note writer did not retain the nonempty synthetic body".into());
    }
    Ok(document)
}

pub(super) fn note(store: &AppStore) -> Result<Value, Error> {
    let db = store.connection()?;
    Ok(serde_json::to_value(crate::notes::load_document_value(
        &db, NOTE,
    )?)?)
}

pub(super) fn add_fault_candidates(store: &AppStore) -> Result<(), Error> {
    let db = store.connection()?;
    for (id, confidence) in [(IGNORE, 1.0), (FAIL, 0.72)] {
        db.execute(
            "insert into duplicate_candidates(id,left_item_id,right_item_id,reason,confidence,created_at)
             values(?1,'duplicate-left','duplicate-right','doi',?2,'2026')",
            params![id, confidence],
        )?;
    }
    Ok(())
}

pub(super) fn set_trash(store: &AppStore, value: Option<&str>) -> Result<(), Error> {
    if store.connection()?.execute(
        "update bibliographic_items set trashed_at=?1 where id='duplicate-trashed'",
        [value],
    )? != 1
    {
        return Err("synthetic history endpoint state change did not affect one row".into());
    }
    Ok(())
}
