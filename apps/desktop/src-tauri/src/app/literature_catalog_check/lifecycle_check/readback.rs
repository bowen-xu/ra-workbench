use super::*;
use rusqlite::{Connection, types::ValueRef};

fn rows(db: &Connection, sql: &str) -> Result<Value, Error> {
    let mut statement = db.prepare(sql)?;
    let names = statement
        .column_names()
        .iter()
        .map(|n| (*n).to_owned())
        .collect::<Vec<_>>();
    let mut cursor = statement.query([])?;
    let mut result = vec![];
    while let Some(row) = cursor.next()? {
        let mut object = serde_json::Map::new();
        for (index, name) in names.iter().enumerate() {
            let value = match row.get_ref(index)? {
                ValueRef::Null => Value::Null,
                ValueRef::Integer(value) => json!(value),
                ValueRef::Real(value) => json!(format!("{:016x}", value.to_bits())),
                ValueRef::Text(value) => json!(std::str::from_utf8(value)?),
                _ => return Err("unexpected synthetic lifecycle snapshot column".into()),
            };
            object.insert(name.clone(), value);
        }
        result.push(Value::Object(object));
    }
    Ok(Value::Array(result))
}

pub(super) fn snapshot(store: &AppStore) -> Result<Value, Error> {
    let db = store.connection()?;
    let projections = [
        (
            "items",
            "select id,item_type,title,archived_at,trashed_at,created_at,updated_at from bibliographic_items where id like 'lifecycle-%' order by id",
        ),
        (
            "projects",
            "select id,title,summary,created_at from research_projects where id like 'lifecycle-%' order by id",
        ),
        (
            "projectItems",
            "select project_id,item_id,added_at from project_items where item_id like 'lifecycle-%' order by project_id,item_id",
        ),
        (
            "attachments",
            "select id,item_id,content_sha256,display_name,created_at from attachments where id='lifecycle-attachment'",
        ),
        (
            "anchors",
            "select id,item_id,attachment_id,content_sha256,page_index,quote_exact,created_at from source_anchors where id='lifecycle-anchor'",
        ),
        (
            "readingStates",
            "select item_id,attachment_id,page_index,zoom as zoomBits,updated_at from reading_states where item_id='lifecycle-main'",
        ),
        (
            "identifiers",
            "select id,item_id,kind,normalized_value,display_value from identifiers where id='lifecycle-doi'",
        ),
        (
            "itemTags",
            "select item_id,tag_id from item_tags where item_id='lifecycle-main' order by tag_id",
        ),
        (
            "provenance",
            "select item_id,field_name,source_kind,updated_at from metadata_provenance where item_id='lifecycle-main' order by field_name",
        ),
        (
            "search",
            "select item_id,title from literature_search where item_id like 'lifecycle-%' order by item_id",
        ),
        (
            "notePages",
            "select id,title,document_title,status,revision,schema_version,created_at,updated_at,trashed_at from note_pages where id='lifecycle-core-note'",
        ),
        (
            "noteBlocks",
            "select id,page_id,parent_block_id,position,block_type,props_json,content_json,plain_text,revision,status,created_at,updated_at from note_blocks where page_id='lifecycle-core-note' order by id",
        ),
        (
            "noteRevisions",
            "select id,page_id,page_revision,schema_version,document_json,reason,created_at from note_revisions where page_id='lifecycle-core-note' order by page_revision",
        ),
        (
            "noteLinks",
            "select id,source_page_id,target_kind,target_id,relation_type,target_snapshot_json,created_at from note_links where source_page_id='lifecycle-core-note' order by id",
        ),
        (
            "effects",
            "select rowid,item_id,kind from synthetic_lifecycle_effects order by rowid",
        ),
    ];
    let mut result = serde_json::Map::new();
    for (name, sql) in projections {
        result.insert(name.into(), rows(&db, sql)?);
    }
    Ok(Value::Object(result))
}

pub(super) fn assert_note(store: &AppStore, expected: &Value) -> Result<(), Error> {
    if fixture::note(store)? != *expected || !store.connection()?.is_autocommit() {
        return Err("lifecycle changed the Core note or left SQL unsettled".into());
    }
    Ok(())
}

pub(super) fn item(store: &AppStore, id: &str, archived: bool, trashed: bool) -> Result<(), Error> {
    let (a, t): (Option<String>, Option<String>) = store.connection()?.query_row(
        "select archived_at,trashed_at from bibliographic_items where id=?1",
        [id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    if a.is_some() != archived || t.is_some() != trashed {
        return Err("actual lifecycle nullable state differs from its original operation".into());
    }
    Ok(())
}

pub(super) fn protected(snapshot: &Value) -> Result<Value, Error> {
    let mut result = snapshot
        .as_object()
        .ok_or("lifecycle snapshot is not an object")?
        .clone();
    for key in ["items", "projectItems", "effects"] {
        result.remove(key);
    }
    Ok(Value::Object(result))
}

pub(super) fn project_ids(store: &AppStore) -> Result<Vec<String>, Error> {
    let db = store.connection()?;
    Ok(db
        .prepare("select project_id from project_items where item_id=?1 order by project_id")?
        .query_map([fixture::MAIN], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?)
}
