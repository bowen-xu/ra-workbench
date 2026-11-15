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
                ValueRef::Real(value) => json!({"f64Bits":format!("{:016x}",value.to_bits())}),
                ValueRef::Text(value) => json!(std::str::from_utf8(value)?),
                ValueRef::Blob(_) => return Err("unexpected blob in fixed undo projection".into()),
            };
            object.insert(name.clone(), value);
        }
        output.push(Value::Object(object));
    }
    Ok(Value::Array(output))
}

pub(super) fn snapshot(store: &AppStore) -> Result<Value, Error> {
    let db = store.connection()?;
    let mut result = serde_json::Map::new();
    for (name, sql) in [
        (
            "papers",
            "select id,title,archived_at,trashed_at,created_at,updated_at from bibliographic_items where id like 'undo-%' order by id",
        ),
        (
            "anchors",
            "select id,item_id,attachment_id,content_sha256,page_index,rects_json,quote_exact,selector_json from source_anchors where id like 'undo-%' order by id",
        ),
        (
            "attachments",
            "select id,item_id,content_sha256,display_name from attachments where id like 'undo-%' order by id",
        ),
        (
            "readingStates",
            "select item_id,attachment_id,page_index,zoom,updated_at,view_state_json from reading_states where item_id like 'undo-%' order by item_id",
        ),
        (
            "identifiers",
            "select id,item_id,kind,normalized_value,display_value from identifiers where id like 'undo-%' order by id",
        ),
        (
            "projects",
            "select project_id,item_id,added_at from project_items where project_id like 'undo-%' order by project_id,item_id",
        ),
        (
            "tags",
            "select item_id,tag_id from item_tags where tag_id like 'undo-%' order by item_id,tag_id",
        ),
        (
            "provenance",
            "select item_id,field_name,source_kind,source_reference,confirmed_by_user,updated_at from metadata_provenance where item_id like 'undo-%' order by item_id,field_name",
        ),
        (
            "candidates",
            "select id,left_item_id,right_item_id,reason,confidence,decision,created_at,decided_at from duplicate_candidates where id like 'undo-%' order by id",
        ),
        (
            "operations",
            "select id,survivor_item_id,merged_item_id,snapshot_json,state,created_at,undone_at from merge_operations where id like 'undo-%' order by id",
        ),
        (
            "collections",
            "select id,name,description,concept_id from collections where id like 'undo-%' order by id",
        ),
        (
            "relations",
            "select id,source_id,target_id,type_id from collection_relations where id like 'undo-%' order by id",
        ),
        (
            "memberships",
            "select item_id,collection_id from collection_memberships where item_id like 'undo-%' order by item_id,collection_id",
        ),
        (
            "assignments",
            "select item_id,relation_id from item_relation_assignments where item_id like 'undo-%' order by item_id,relation_id",
        ),
        (
            "collectionRevision",
            "select revision from collection_state where id=1",
        ),
        ("nalRevision", "select revision from nal_state where id=1"),
        (
            "statements",
            "select id,subject,predicate,origin from nal_statements where id like 'undo-%' order by rowid",
        ),
        (
            "annotations",
            "select id,concept_id,statement_id,note_page_id from nal_annotations where id like 'undo-%' order by rowid",
        ),
        (
            "literatureSearch",
            "select item_id,title,creators,identifiers,tags,full_text from literature_search where item_id like 'undo-%' order by item_id,rowid",
        ),
        (
            "readingSources",
            "select block_id,page_id,current_anchor_id,paper_id_snapshot,paper_title_snapshot,attachment_id_snapshot,content_sha256,quote_exact,created_at,updated_at from reading_note_sources where page_id='duplicate-undo-core-note'",
        ),
        (
            "notePage",
            "select id,title,document_title,status,revision,schema_version,created_at,updated_at,trashed_at from note_pages where id='duplicate-undo-core-note'",
        ),
        (
            "noteBlocks",
            "select id,page_id,parent_block_id,position,block_type,props_json,content_json,plain_text,revision,status,created_at,updated_at from note_blocks where page_id='duplicate-undo-core-note' order by id",
        ),
        (
            "noteRevisions",
            "select id,page_id,page_revision,schema_version,document_json,reason,created_at from note_revisions where page_id='duplicate-undo-core-note' order by page_revision",
        ),
        (
            "noteCatalog",
            "select id,title,category,source_id,source_label,updated_at,status,is_favorite,favorite_position,revision from note_page_catalog where id='duplicate-undo-core-note'",
        ),
        (
            "noteTitleCache",
            "select page_id,body_title from note_catalog_title_cache where page_id='duplicate-undo-core-note'",
        ),
        (
            "noteMetadata",
            "select page_id,search_text,folded_text from note_catalog_metadata_text where page_id='duplicate-undo-core-note'",
        ),
        (
            "noteMetadataSearch",
            "select rowid,folded_text from note_catalog_metadata_search where rowid in (select rowid from note_catalog_metadata_text where page_id='duplicate-undo-core-note')",
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

pub(super) fn assert_note(store: &AppStore, expected: &Value) -> Result<(), Error> {
    if fixture::note(store)? != *expected || !store.connection()?.is_autocommit() {
        return Err("undo changed the Core note document or left SQL unsettled".into());
    }
    Ok(())
}

pub(super) fn assert_success(
    store: &AppStore,
    case: &fixture::Case,
    before: &Value,
) -> Result<Value, Error> {
    let db = store.connection()?;
    for (table, id_column, id) in [
        ("source_anchors", "id", case.identity("anchor")),
        ("attachments", "id", case.identity("attachment")),
        (
            "reading_states",
            "attachment_id",
            case.identity("attachment"),
        ),
        ("identifiers", "id", case.identity("identifier")),
    ] {
        // Table/column names are fixed harness literals above, never native input.
        let sql = format!("select item_id from {table} where {id_column}=?1");
        let item: String = db.query_row(&sql, [id], |r| r.get(0))?;
        if item != case.source {
            return Err(format!("undo did not restore {table}").into());
        }
    }
    let(state,undone,candidate,decided,archived):(String,Option<String>,Option<String>,Option<String>,Option<String>)=db.query_row(
        "select o.state,o.undone_at,c.decision,c.decided_at,b.archived_at from merge_operations o join duplicate_candidates c on c.id=?2 join bibliographic_items b on b.id=?3 where o.id=?1",
        rusqlite::params![case.id,case.snapshot.candidate_id,case.source],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)))?;
    if state != "undone"
        || undone.as_deref().is_none_or(str::is_empty)
        || candidate.is_some()
        || decided.is_some()
        || archived.is_some()
    {
        return Err("undo did not settle history/candidate/archive at original values".into());
    }
    for (table, column, id) in [
        ("project_items", "project_id", case.identity("project")),
        ("item_tags", "tag_id", case.identity("tag")),
        ("metadata_provenance", "item_id", case.survivor.clone()),
    ] {
        let count: u64 = db.query_row(
            &format!("select count(*) from {table} where {column}=?1"),
            [id],
            |r| r.get(0),
        )?;
        if count != 0 {
            return Err("undo retained an added project/tag/provenance row".into());
        }
    }
    let members: u64 = db.query_row(
        "select count(*) from collection_memberships where item_id=?1",
        [&case.survivor],
        |r| r.get(0),
    )?;
    let assignments: u64 = db.query_row(
        "select count(*) from item_relation_assignments where item_id=?1",
        [&case.survivor],
        |r| r.get(0),
    )?;
    let statement: u64 = db.query_row(
        "select count(*) from nal_statements where id=?1",
        [case.identity("statement-added")],
        |r| r.get(0),
    )?;
    let annotation: u64 = db.query_row(
        "select count(*) from nal_annotations where id=?1",
        [case.identity("annotation")],
        |r| r.get(0),
    )?;
    let class_changed = !matches!(case.mode, fixture::Mode::NoClassification);
    let nal_changed = matches!(case.mode, fixture::Mode::Full);
    if members != if class_changed { 1 } else { 2 }
        || assignments != u64::from(!class_changed)
        || statement != u64::from(!nal_changed)
        || annotation != u64::from(!nal_changed)
    {
        return Err(
            "undo violated optional classification/NAL branches or their full-row deltas".into(),
        );
    }
    let source: String = db.query_row(
        "select paper_id_snapshot from reading_note_sources where page_id=?1",
        [fixture::NOTE],
        |r| r.get(0),
    )?;
    let title: String = db.query_row(
        "select title from bibliographic_items where id=?1",
        [&case.source],
        |r| r.get(0),
    )?;
    let label: String = db.query_row(
        "select source_label from note_page_catalog where id=?1",
        [fixture::NOTE],
        |r| r.get(0),
    )?;
    let metadata: String = db.query_row(
        "select search_text from note_catalog_metadata_text where page_id=?1",
        [fixture::NOTE],
        |r| r.get(0),
    )?;
    if source != case.source || label != title || !metadata.contains(&title) {
        return Err("undo did not update the original Core note derived source projections".into());
    }
    for id in [&case.survivor, &case.source] {
        let count:u64=db.query_row("select count(*) from literature_search s join bibliographic_items b on b.id=s.item_id where s.item_id=?1 and s.title=b.title",[id],|r|r.get(0))?;
        if count != 1 {
            return Err("undo did not rebuild each original literature search row".into());
        }
    }
    drop(db);
    let after = snapshot(store)?;
    for (name, changed) in [
        ("collectionRevision", class_changed),
        ("nalRevision", nal_changed),
    ] {
        let old = before[name][0]["revision"]
            .as_i64()
            .ok_or("missing before revision")?;
        if after[name][0]["revision"].as_i64() != Some(old + i64::from(changed)) {
            return Err("undo optional state revision increment changed".into());
        }
    }
    Ok(after)
}
