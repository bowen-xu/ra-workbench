use super::*;
use rusqlite::{Connection, types::ValueRef};

fn rows(db: &Connection, sql: &str) -> Result<Value, Error> {
    let mut statement = db.prepare(sql)?;
    let names = statement
        .column_names()
        .iter()
        .map(|s| (*s).to_owned())
        .collect::<Vec<_>>();
    let mut cursor = statement.query([])?;
    let mut result = vec![];
    while let Some(row) = cursor.next()? {
        let mut object = serde_json::Map::new();
        for (index, name) in names.iter().enumerate() {
            let value = match row.get_ref(index)? {
                ValueRef::Null => Value::Null,
                ValueRef::Integer(value) => json!(value),
                ValueRef::Real(value) => json!({"f64Bits":format!("{:016x}",value.to_bits())}),
                ValueRef::Text(value) => json!(std::str::from_utf8(value)?),
                ValueRef::Blob(value) => {
                    json!({"blobHex":value.iter().map(|v|format!("{v:02x}")).collect::<String>()})
                }
            };
            object.insert(name.clone(), value);
        }
        result.push(Value::Object(object));
    }
    Ok(json!(result))
}

pub(super) fn snapshot(store: &AppStore) -> Result<Value, Error> {
    let db = store.connection()?;
    let mut result = serde_json::Map::new();
    // Fixed synthetic rows only, with every selected column named explicitly.
    for (name, sql) in [
        (
            "items",
            "select id,item_type,title,abstract_text,issued_year,archived_at,trashed_at,created_at,updated_at from bibliographic_items where id like 'cleanup-%' order by id",
        ),
        (
            "attachments",
            "select id,item_id,content_sha256,display_name,attachment_kind,source_path,created_at from attachments where id like 'cleanup-%' order by id",
        ),
        (
            "anchors",
            "select id,item_id,attachment_id,content_sha256,selector_version,page_index,page_label,rects_json,quote_exact,quote_prefix,quote_suffix,selector_json,created_at from source_anchors where id='cleanup-anchor'",
        ),
        (
            "readingStates",
            "select attachment_id,item_id,page_index,zoom,updated_at,view_state_json from reading_states where item_id like 'cleanup-%' order by item_id",
        ),
        (
            "contexts",
            "select id,paper_id,context_hash,context_schema_version,created_at from paper_context_snapshots where id like 'cleanup-%' order by id",
        ),
        (
            "history",
            "select id,paper_id,revision,profile_refs_json,created_at from paper_reading_profile_selection_revisions where id like 'cleanup-%' order by id",
        ),
        (
            "content",
            "select sha256,relative_path,mime_type,byte_size,created_at from content_objects where sha256='eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee'",
        ),
        (
            "gc",
            "select content_sha256,unreferenced_since,not_before,reason,attempt_count,last_attempt_at,last_error_code from content_object_gc_candidates where content_sha256='eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee'",
        ),
        (
            "scope",
            "select item_id,table_name,row_key from literature_cleanup_rows where item_id like 'cleanup-%' order by item_id,table_name,row_key",
        ),
        (
            "readingSources",
            "select block_id,page_id,note_type,color,current_anchor_id,paper_id_snapshot,paper_title_snapshot,attachment_id_snapshot,attachment_name_snapshot,content_sha256,page_index,page_label,rects_json,quote_exact,quote_prefix,quote_suffix,locator_version,created_at,updated_at,selector_json from reading_note_sources where page_id='cleanup-reading-note'",
        ),
        (
            "notePages",
            "select id,title,parent_page_id,position,status,is_favorite,revision,schema_version,created_at,updated_at,trashed_at,document_title from note_pages where id in('cleanup-independent-note','cleanup-reading-note') order by id",
        ),
        (
            "noteBlocks",
            "select id,scope_kind,page_id,parent_block_id,position,block_type,props_json,content_json,plain_text,schema_version,revision,status,is_favorite,created_at,updated_at,trashed_at from note_blocks where page_id in('cleanup-independent-note','cleanup-reading-note') order by id",
        ),
        (
            "noteRevisions",
            "select id,page_id,page_revision,schema_version,document_json,reason,created_at,is_pinned from note_revisions where page_id in('cleanup-independent-note','cleanup-reading-note') order by page_id,page_revision",
        ),
        (
            "noteLinks",
            "select id,source_page_id,target_kind,target_id,relation_type,target_snapshot_json,created_at from note_links where source_page_id in('cleanup-independent-note','cleanup-reading-note') order by id",
        ),
        (
            "readingMap",
            "select legacy_id,page_id from reading_note_page_map where page_id='cleanup-reading-note'",
        ),
        (
            "search",
            "select item_id,title,creators,identifiers,tags,full_text from literature_search where item_id like 'cleanup-%' order by item_id,rowid",
        ),
        (
            "noteRevisionAssets",
            "select revision_id,content_sha256 from note_revision_assets where revision_id in(select id from note_revisions where page_id in('cleanup-independent-note','cleanup-reading-note')) order by revision_id,content_sha256",
        ),
        (
            "noteIndex",
            "select id,schema_version,state,last_rebuilt_at,last_error_code from note_index_state where id=1",
        ),
        (
            "noteFts",
            "select object_id,object_kind,title,body,citation_text,paper_title,creators,tags,project_titles,source_state,updated_at from note_search where object_id in('cleanup-independent-note','cleanup-reading-note','cleanup-reading-alias') order by object_kind,object_id,rowid",
        ),
        (
            "noteCatalog",
            "select id,title,category,source_id,source_label,updated_at,status,is_favorite,favorite_position,revision,catalog_title_key,catalog_title_down,catalog_category_key,catalog_category_down,catalog_source_key,catalog_source_down,catalog_updated_ms from note_page_catalog where id in('cleanup-independent-note','cleanup-reading-note') order by id",
        ),
        (
            "noteTitleCache",
            "select page_id,body_title from note_catalog_title_cache where page_id in('cleanup-independent-note','cleanup-reading-note') order by page_id",
        ),
        (
            "noteMetadata",
            "select page_id,search_text,folded_text from note_catalog_metadata_text where page_id in('cleanup-independent-note','cleanup-reading-note') order by page_id",
        ),
        (
            "noteMetadataSearch",
            "select rowid,folded_text from note_catalog_metadata_search where rowid in(select rowid from note_catalog_metadata_text where page_id in('cleanup-independent-note','cleanup-reading-note')) order by rowid",
        ),
        (
            "noteCatalogState",
            "select generation from note_page_catalog_state where id=1",
        ),
        (
            "noteSearchState",
            "select generation from note_catalog_search_state where id=1",
        ),
    ] {
        result.insert(name.into(), rows(&db, sql)?);
    }
    Ok(Value::Object(result))
}

pub(super) fn assert_partial(store: &AppStore, notes: &Value) -> Result<(), Error> {
    if fixture::notes(store)? != *notes {
        return Err("cleanup changed a retained Core Note document".into());
    }
    let db = store.connection()?;
    let remaining = db
        .prepare("select id from bibliographic_items where id like 'cleanup-%' order by id")?
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if remaining
        != [
            fixture::ACTIVE,
            fixture::RECENT,
            fixture::SECOND,
            fixture::THIRD,
        ]
    {
        return Err(
            "cleanup did not retain exactly the failed/later/recent/independent items".into(),
        );
    }
    let source: (Option<String>,String,String,String) = db.query_row(
        "select current_anchor_id,paper_id_snapshot,content_sha256,(select plain_text from note_blocks where page_id=?1 order by position limit 1) from reading_note_sources where page_id=?1",
        [fixture::READING_NOTE],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))?;
    if source
        != (
            None,
            fixture::FIRST.into(),
            fixture::CONTENT.into(),
            fixture::BODY.into(),
        )
    {
        return Err("cleanup lost the retained reading body or source snapshot".into());
    }
    for (sql, expected) in [
        (
            "select count(*) from attachments where id='cleanup-first-pdf'",
            0,
        ),
        (
            "select count(*) from attachments where id='cleanup-independent-pdf'",
            1,
        ),
        (
            "select count(*) from source_anchors where id='cleanup-anchor'",
            0,
        ),
        (
            "select count(*) from reading_states where item_id='cleanup-first'",
            0,
        ),
        (
            "select count(*) from content_objects where sha256='eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee'",
            1,
        ),
        (
            "select count(*) from content_object_gc_candidates where content_sha256='eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee'",
            0,
        ),
        (
            "select count(*) from literature_cleanup_rows where item_id like 'cleanup-%'",
            0,
        ),
        (
            "select count(*) from paper_context_snapshots where id='cleanup-first-context'",
            0,
        ),
        (
            "select count(*) from paper_reading_profile_selection_revisions where id='cleanup-first-revision'",
            0,
        ),
        (
            "select count(*) from paper_context_snapshots where id in('cleanup-second-context','cleanup-third-context','cleanup-independent-paper-context')",
            3,
        ),
        (
            "select count(*) from paper_reading_profile_selection_revisions where id in('cleanup-second-revision','cleanup-third-revision','cleanup-independent-paper-revision')",
            3,
        ),
    ] {
        if db.query_row(sql, [], |r| r.get::<_, i64>(0))? != expected {
            return Err(
                "cleanup lost scope rollback or independently retained synthetic data".into(),
            );
        }
    }
    if !db.is_autocommit() {
        return Err("cleanup left the SQL owner unsettled".into());
    }
    Ok(())
}

pub(super) fn assert_second_rollback(before: &Value, after: &Value) -> Result<(), Error> {
    for key in ["items", "contexts", "history", "search"] {
        let select = |snapshot: &Value| -> Result<Vec<Value>, Error> {
            Ok(snapshot[key]
                .as_array()
                .ok_or("missing cleanup snapshot rows")?
                .iter()
                .filter(|row| {
                    row["id"].as_str() == Some(fixture::SECOND)
                        || row["paper_id"].as_str() == Some(fixture::SECOND)
                        || row["item_id"].as_str() == Some(fixture::SECOND)
                })
                .cloned()
                .collect())
        };
        if select(before)? != select(after)? {
            return Err("failed second item did not roll back its fixed rows".into());
        }
    }
    for key in [
        "notePages",
        "noteBlocks",
        "noteRevisions",
        "noteLinks",
        "readingMap",
        "content",
        "gc",
    ] {
        if before[key] != after[key] {
            return Err("cleanup changed preserved Core note storage or shared content".into());
        }
    }
    Ok(())
}
