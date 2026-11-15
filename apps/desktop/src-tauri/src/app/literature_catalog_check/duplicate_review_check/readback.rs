use super::*;
use rusqlite::{Connection, types::ValueRef};

fn rows(db: &Connection, sql: &str) -> Result<Value, Error> {
    let mut statement = db.prepare(sql)?;
    let names = statement
        .column_names()
        .iter()
        .map(|name| (*name).to_owned())
        .collect::<Vec<_>>();
    let mut cursor = statement.query([])?;
    let mut output = vec![];
    while let Some(row) = cursor.next()? {
        let mut object = serde_json::Map::new();
        for (index, name) in names.iter().enumerate() {
            let value = match row.get_ref(index)? {
                ValueRef::Null => Value::Null,
                ValueRef::Integer(value) => json!(value),
                ValueRef::Text(value) => json!(std::str::from_utf8(value)?),
                _ => return Err("unexpected non-text/non-integer synthetic snapshot column".into()),
            };
            object.insert(name.clone(), value);
        }
        output.push(Value::Object(object));
    }
    Ok(Value::Array(output))
}

pub(super) fn snapshot(store: &AppStore) -> Result<Value, Error> {
    let db = store.connection()?;
    let mut candidates = vec![];
    let mut statement = db.prepare(
        "select id,left_item_id,right_item_id,reason,confidence,decision,created_at,decided_at
         from duplicate_candidates order by id",
    )?;
    let mut cursor = statement.query([])?;
    while let Some(row) = cursor.next()? {
        candidates.push(json!({
            "id":row.get::<_,String>(0)?,"leftItemId":row.get::<_,String>(1)?,
            "rightItemId":row.get::<_,String>(2)?,"reason":row.get::<_,String>(3)?,
            "confidenceBits":format!("{:016x}",row.get::<_,f64>(4)?.to_bits()),
            "decision":row.get::<_,Option<String>>(5)?,"createdAt":row.get::<_,String>(6)?,
            "decidedAt":row.get::<_,Option<String>>(7)?
        }));
    }
    Ok(json!({
        "bibliographicItems":rows(&db,"select id,item_type,title,archived_at,trashed_at,created_at,updated_at from bibliographic_items where id like 'duplicate-%' order by id")?,
        "duplicateCandidates":candidates,
        "mergeOperations":rows(&db,"select id,survivor_item_id,merged_item_id,snapshot_json,state,created_at,undone_at from merge_operations order by id")?,
        "syntheticEffects":rows(&db,"select rowid,candidate_id,kind from synthetic_duplicate_review_effects order by rowid")?,
        "notePage":rows(&db,"select id,title,document_title,status,revision,schema_version,created_at,updated_at,trashed_at from note_pages where id='duplicate-review-core-note'")?,
        "noteBlocks":rows(&db,"select id,page_id,parent_block_id,position,block_type,props_json,content_json,plain_text,revision,status,created_at,updated_at from note_blocks where page_id='duplicate-review-core-note' order by id")?,
        "noteRevisions":rows(&db,"select id,page_id,page_revision,schema_version,document_json,reason,created_at from note_revisions where page_id='duplicate-review-core-note' order by page_revision")?,
        "syntheticTriggers":rows(&db,"select name,sql from sqlite_schema where type='trigger' and name in ('synthetic_duplicate_review_ignore','synthetic_duplicate_review_fail') order by name")?
    }))
}

pub(super) fn assert_note(store: &AppStore, expected: &Value) -> Result<(), Error> {
    if fixture::note(store)? != *expected || !store.connection()?.is_autocommit() {
        return Err(
            "duplicate review changed the Core note or left its SQL connection unsettled".into(),
        );
    }
    Ok(())
}

pub(super) fn decision(store: &AppStore, id: &str) -> Result<Value, Error> {
    let db = store.connection()?;
    let (decision, timestamp, effects): (Option<String>, Option<String>, u64) = db.query_row(
        "select decision,decided_at,(select count(*) from synthetic_duplicate_review_effects where candidate_id=?1)
         from duplicate_candidates where id=?1", [id],
        |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)),
    )?;
    Ok(json!({"decision":decision,"decidedAt":timestamp,"effects":effects}))
}

pub(super) fn candidates(items: &[DuplicateCandidate]) -> Result<Value, Error> {
    // Never deserialize the public numeric JSON to recover confidence: the
    // deployed parser can round next_down(1.0) to 1.0. Keep that JSON as text.
    Ok(json!({
        "publicJson":serde_json::to_string(items)?,
        "ids":items.iter().map(|item|item.id.as_str()).collect::<Vec<_>>(),
        "confidenceBits":items.iter().map(|item|format!("{:016x}",item.confidence.to_bits())).collect::<Vec<_>>()
    }))
}

pub(super) fn assert_candidates(
    items: &[DuplicateCandidate],
    expected: &[(&str, f64)],
) -> Result<(), Error> {
    if items.len() != expected.len() {
        return Err("synthetic candidate result count changed".into());
    }
    for (item, (id, confidence)) in items.iter().zip(expected) {
        if item.id != *id
            || item.left_item_id != "duplicate-left"
            || item.left_title != "Synthetic duplicate left"
            || item.right_item_id != "duplicate-right"
            || item.right_title != "Synthetic duplicate right"
            || item.reason != "doi"
            || item.confidence.to_bits() != confidence.to_bits()
        {
            return Err(
                format!("synthetic candidate full DTO or f64 bits changed for {id}").into(),
            );
        }
    }
    Ok(())
}

pub(super) fn assert_history(items: &[MergeOperation], ids: &[&str]) -> Result<(), Error> {
    if items
        .iter()
        .map(|item| item.id.as_str())
        .collect::<Vec<_>>()
        != ids
    {
        return Err("applied/archived/trash history order changed".into());
    }
    for item in items {
        let (right, title, time) = match item.id.as_str() {
            "history-active" => (
                "duplicate-right",
                "Synthetic duplicate right",
                "2026-11-05T00:00:03Z",
            ),
            "history-archived" => (
                "duplicate-archived",
                "Synthetic archived endpoint",
                "2026-11-05T00:00:04Z",
            ),
            "history-trashed" => (
                "duplicate-trashed",
                "Synthetic trashed endpoint",
                "2026-11-05T00:00:05Z",
            ),
            _ => return Err("unexpected synthetic merge history identity".into()),
        };
        if item.survivor_item_id != "duplicate-left"
            || item.survivor_title != "Synthetic duplicate left"
            || item.merged_item_id != right
            || item.merged_title != title
            || item.state != "applied"
            || item.created_at != time
        {
            return Err("synthetic merge history lost one of its original seven fields".into());
        }
    }
    Ok(())
}
