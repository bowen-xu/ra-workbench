use super::*;
use rusqlite::types::ValueRef;

pub(super) fn rows(store: &AppStore, sql: &str) -> Result<Value, Error> {
    let db = store.connection()?;
    let mut statement = db.prepare(sql)?;
    let names = statement
        .column_names()
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let mut cursor = statement.query([])?;
    let mut rows = vec![];
    while let Some(row) = cursor.next()? {
        let mut values = serde_json::Map::new();
        for (index, name) in names.iter().enumerate() {
            let value = match row.get_ref(index)? {
                ValueRef::Null => Value::Null,
                ValueRef::Integer(value) => json!(value),
                ValueRef::Real(value) => json!(format!("{:016x}", value.to_bits())),
                ValueRef::Text(value) => json!(std::str::from_utf8(value)?),
                ValueRef::Blob(_) => {
                    return Err("unexpected synthetic attachment blob column".into());
                }
            };
            values.insert(name.clone(), value);
        }
        rows.push(Value::Object(values));
    }
    Ok(Value::Array(rows))
}

pub(super) fn snapshot(store: &AppStore) -> Result<Value, Error> {
    let mut values = serde_json::Map::new();
    for (key, sql) in [
        (
            "items",
            "select id,item_type,title,abstract_text,issued_year,container_title,conference_name,proceedings_title,place,volume,issue,pages,url,language,bibtex_key,raw_bibtex,reading_status,archived_at,trashed_at,created_at,updated_at from bibliographic_items order by id",
        ),
        (
            "attachments",
            "select id,item_id,content_sha256,display_name,source_path,created_at from attachments order by id",
        ),
        (
            "content",
            "select sha256,relative_path,mime_type,byte_size,created_at from content_objects order by sha256",
        ),
        (
            "duplicates",
            "select id,left_item_id,right_item_id,reason,confidence,decision,created_at from duplicate_candidates order by id",
        ),
        (
            "positions",
            "select item_id,attachment_id,page_index,zoom,view_state_json,updated_at from reading_states order by attachment_id",
        ),
        (
            "pages",
            "select attachment_id,page_index,page_text,indexed_at from attachment_page_texts order by attachment_id,page_index",
        ),
        (
            "indexJobs",
            "select attachment_id,state,page_count,error_message,updated_at from full_text_index_jobs order by attachment_id",
        ),
        (
            "fts",
            "select item_id,title,creators,identifiers,tags,full_text from literature_search order by item_id",
        ),
        (
            "anchors",
            "select id,item_id,attachment_id,content_sha256,selector_version,page_index,page_label,rects_json,quote_exact,quote_prefix,quote_suffix,selector_json,created_at from source_anchors order by id",
        ),
        (
            "contexts",
            "select id,paper_id,context_hash,context_schema_version,created_at from paper_context_snapshots order by id",
        ),
        (
            "contextAttachments",
            "select snapshot_id,ordinal,attachment_id,content_sha256,parse_run_id from paper_context_attachments order by snapshot_id,ordinal",
        ),
        (
            "parses",
            "select id,item_id,attachment_id,content_sha256,service_base_url,docling_status,parse_status,raw_response,created_at,finished_at from docling_parse_runs order by id",
        ),
        (
            "jobs",
            "select id,state,source_kind,total_count,completed_count,cancellation_requested,created_at,updated_at from import_jobs order by id",
        ),
        (
            "results",
            "select id,job_id,input_label,state,item_id,error_code,error_message,raw_input,created_at,updated_at from import_results order by job_id,id",
        ),
        (
            "creators",
            "select id,family_name,given_name,literal_name from creators order by id",
        ),
        (
            "creatorLinks",
            "select item_id,creator_id,role,position from item_creators order by item_id,role,position",
        ),
        (
            "identifiers",
            "select id,item_id,kind,normalized_value,display_value from identifiers order by id",
        ),
        ("tags", "select id,name from tags order by id"),
        (
            "tagLinks",
            "select item_id,tag_id from item_tags order by item_id,tag_id",
        ),
        (
            "provenance",
            "select item_id,field_name,source_kind,source_reference,confirmed_by_user,updated_at from metadata_provenance order by item_id,field_name",
        ),
        (
            "notePages",
            "select id,title,parent_page_id,revision,created_at,updated_at from note_pages where id='attachment-cli-core-note'",
        ),
        (
            "noteRevisions",
            "select id,page_id,page_revision,schema_version,document_json,reason,created_at from note_revisions where page_id='attachment-cli-core-note' order by page_revision",
        ),
        (
            "noteLinks",
            "select id,source_page_id,target_kind,target_id,relation_type,target_snapshot_json,created_at from note_links where source_page_id='attachment-cli-core-note' order by id",
        ),
        (
            "noteBlocks",
            "select id,scope_kind,page_id,parent_block_id,position,block_type,props_json,content_json,plain_text,schema_version,revision,status,is_favorite,created_at,updated_at from note_blocks where page_id='attachment-cli-core-note' order by id",
        ),
    ] {
        values.insert(key.into(), rows(store, sql)?);
    }
    Ok(Value::Object(values))
}

pub(super) fn protected(value: &Value) -> Result<Value, Error> {
    let mut out = serde_json::Map::new();
    for key in ["notePages", "noteRevisions", "noteLinks", "noteBlocks"] {
        out.insert(
            key.into(),
            value
                .get(key)
                .ok_or("missing protected attachment projection")?
                .clone(),
        );
    }
    Ok(Value::Object(out))
}

pub(super) fn files(store: &AppStore) -> Result<Value, Error> {
    let db = store.connection()?;
    let mut statement =
        db.prepare("select sha256,relative_path,byte_size from content_objects order by sha256")?;
    let objects = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, u64>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(statement);
    drop(db);
    let mut out = vec![];
    for (sha, relative, size) in objects {
        let bytes =
            crate::content::StoredObjectRead::resolve(store.content_directory(), &relative)?
                .read()?;
        let actual = format!("{:x}", Sha256::digest(&bytes));
        if actual != sha || bytes.len() as u64 != size {
            return Err("actual object bytes differ from original Core identity".into());
        }
        out.push(
            json!({"sha256":sha,"relativePath":relative,"byteSize":size,"actualSha256":actual}),
        );
    }
    Ok(json!(out))
}
