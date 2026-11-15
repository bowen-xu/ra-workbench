use super::*;
use rusqlite::{Connection, types::ValueRef};

fn rows(db: &Connection, sql: &str) -> Result<Value, Error> {
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
                ValueRef::Real(value) => json!(format!("{:016x}", value.to_bits())),
                ValueRef::Text(value) => json!(std::str::from_utf8(value)?),
                ValueRef::Blob(_) => return Err("unexpected synthetic metadata blob column".into()),
            };
            fields.insert(name.clone(), value);
        }
        values.push(Value::Object(fields));
    }
    Ok(Value::Array(values))
}

pub(super) fn snapshot(store: &AppStore) -> Result<Value, Error> {
    let db = store.connection()?;
    let projections = [
        (
            "items",
            "select id,item_type,title,abstract_text,issued_year,container_title,conference_name,proceedings_title,place,volume,issue,pages,url,language,bibtex_key,raw_bibtex,reading_status,archived_at,trashed_at,created_at,updated_at from bibliographic_items order by id",
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
            "candidates",
            "select id,item_id,source_kind,source_reference,draft_json,raw_payload,import_result_id,state,created_at,decided_at from metadata_candidates order by id",
        ),
        (
            "results",
            "select id,job_id,input_label,state,item_id,error_code,error_message,raw_input,created_at,updated_at from import_results order by id",
        ),
        (
            "jobs",
            "select id,state,source_kind,total_count,completed_count,cancellation_requested,created_at,updated_at from import_jobs order by id",
        ),
        (
            "fts",
            "select item_id,title,creators,identifiers,tags,full_text from literature_search order by item_id",
        ),
        (
            "duplicates",
            "select id,left_item_id,right_item_id,reason,confidence,decision,created_at from duplicate_candidates order by id",
        ),
        (
            "events",
            "select seq,phase from synthetic_metadata_cli_events order by seq",
        ),
        (
            "notePages",
            "select id,title,parent_page_id,revision,created_at,updated_at from note_pages where id='metadata-cli-core-note'",
        ),
        (
            "noteRevisions",
            "select id,page_id,page_revision,schema_version,document_json,reason,created_at from note_revisions where page_id='metadata-cli-core-note' order by page_revision",
        ),
        (
            "noteLinks",
            "select id,source_page_id,target_kind,target_id,relation_type,target_snapshot_json,created_at from note_links where source_page_id='metadata-cli-core-note' order by id",
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
        return Err("metadata changed the Core note or left its SQL connection unsettled".into());
    }
    Ok(())
}

pub(super) fn value<T: rusqlite::types::FromSql>(store: &AppStore, sql: &str) -> Result<T, Error> {
    Ok(store.connection()?.query_row(sql, [], |row| row.get(0))?)
}

pub(super) fn provenance(
    store: &AppStore,
    id: &str,
    field: &str,
) -> Result<(String, Option<String>, bool), Error> {
    Ok(store.connection()?.query_row("select source_kind,source_reference,confirmed_by_user from metadata_provenance where item_id=?1 and field_name=?2",[id,field],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?)))?)
}

pub(super) fn candidate_state(store: &AppStore, id: &str) -> Result<String, Error> {
    Ok(store.connection()?.query_row(
        "select state from metadata_candidates where id=?1",
        [id],
        |row| row.get(0),
    )?)
}
