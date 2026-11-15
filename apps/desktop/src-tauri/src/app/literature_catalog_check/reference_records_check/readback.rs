use super::*;
use rusqlite::types::ValueRef;

fn rows(db: &rusqlite::Connection, sql: &str) -> Result<Value, Error> {
    let mut statement = db.prepare(sql)?;
    let names = statement
        .column_names()
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let mut cursor = statement.query([])?;
    let mut values = Vec::new();
    while let Some(row) = cursor.next()? {
        let mut fields = serde_json::Map::new();
        for (index, name) in names.iter().enumerate() {
            let value = match row.get_ref(index)? {
                ValueRef::Null => Value::Null,
                ValueRef::Integer(value) => json!(value),
                ValueRef::Text(value) => json!(std::str::from_utf8(value)?),
                ValueRef::Real(_) | ValueRef::Blob(_) => {
                    return Err("unexpected synthetic reference primitive".into());
                }
            };
            fields.insert(name.clone(), value);
        }
        values.push(Value::Object(fields));
    }
    Ok(Value::Array(values))
}

pub(super) fn snapshot(store: &AppStore) -> Result<Value, Error> {
    let db = store.connection()?;
    let mut result = serde_json::Map::new();
    for (name, sql) in [
        (
            "records",
            "select id,citing_paper_id,source_anchor_id,raw_text,fields_json,parse_state,identity_state,matched_item_id,semantic_state,usage_kind,usage_summary,usage_anchor_id,provenance_kind,current_revision,created_at,updated_at from reference_records order by updated_at desc,id",
        ),
        (
            "revisions",
            "select id,reference_record_id,revision,snapshot_json,editor_kind,change_reason,created_at from reference_record_revisions order by reference_record_id,revision",
        ),
        (
            "items",
            "select id,item_type,title,archived_at,trashed_at,created_at,updated_at from bibliographic_items order by id",
        ),
        (
            "content",
            "select sha256,relative_path,mime_type,byte_size,created_at from content_objects order by sha256",
        ),
        (
            "attachments",
            "select id,item_id,content_sha256,display_name,created_at from attachments order by id",
        ),
        (
            "anchors",
            "select id,item_id,attachment_id,content_sha256,selector_version,page_index,page_label,rects_json,quote_exact,quote_prefix,quote_suffix,selector_json,created_at from source_anchors order by id",
        ),
        (
            "notePages",
            "select id,title,parent_page_id,revision,created_at,updated_at from note_pages where id='reference-cli-core-note'",
        ),
        (
            "noteRevisions",
            "select id,page_id,page_revision,schema_version,document_json,reason,created_at from note_revisions where page_id='reference-cli-core-note' order by page_revision",
        ),
        (
            "noteLinks",
            "select id,source_page_id,target_kind,target_id,relation_type,target_snapshot_json,created_at from note_links where source_page_id='reference-cli-core-note' order by id",
        ),
    ] {
        result.insert(name.into(), rows(&db, sql)?);
    }
    Ok(Value::Object(result))
}

pub(super) fn protected(snapshot: &Value) -> Result<Value, Error> {
    let mut fields = serde_json::Map::new();
    for name in [
        "items",
        "content",
        "attachments",
        "anchors",
        "notePages",
        "noteRevisions",
        "noteLinks",
    ] {
        fields.insert(
            name.into(),
            snapshot
                .get(name)
                .ok_or("missing protected reference projection")?
                .clone(),
        );
    }
    Ok(Value::Object(fields))
}

pub(super) fn history(
    snapshot: &Value,
    id: &str,
    drafts: &[ReferenceRecordDraft],
) -> Result<(), Error> {
    let rows = snapshot["revisions"]
        .as_array()
        .ok_or("missing reference history rows")?;
    if rows.len() != drafts.len() {
        return Err("reference immutable history row count changed".into());
    }
    for (index, (row, draft)) in rows.iter().zip(drafts).enumerate() {
        uuid::Uuid::parse_str(
            row["id"]
                .as_str()
                .ok_or("reference history lost actual UUID")?,
        )?;
        let stored: ReferenceRecordDraft = serde_json::from_str(
            row["snapshot_json"]
                .as_str()
                .ok_or("missing original draft snapshot")?,
        )?;
        if stored != *draft
            || row["reference_record_id"] != id
            || row["revision"] != json!(index + 1)
            || row["editor_kind"] != "human"
            || row["change_reason"] != draft.change_reason.trim()
        {
            return Err(
                "reference immutable history lost original draft, order, identity or reason".into(),
            );
        }
    }
    Ok(())
}
