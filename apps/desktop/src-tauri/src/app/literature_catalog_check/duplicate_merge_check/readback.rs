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
                ValueRef::Blob(value) => {
                    json!({"blobHex":value.iter().map(|b|format!("{b:02x}")).collect::<String>()})
                }
            };
            object.insert(name.clone(), value);
        }
        output.push(Value::Object(object));
    }
    Ok(json!(output))
}

pub(super) fn snapshot(store: &AppStore) -> Result<Value, Error> {
    let db = store.connection()?;
    let mut result = serde_json::Map::new();
    // Fixed synthetic-table projections; never application settings or credentials.
    for (table, columns, order) in [
        (
            "bibliographic_items",
            "id,item_type,title,abstract_text,issued_year,container_title,volume,issue,pages,url,language,bibtex_key,raw_bibtex,reading_status,archived_at,created_at,updated_at,trashed_at,conference_name,proceedings_title,place",
            "id",
        ),
        (
            "attachments",
            "id,item_id,content_sha256,display_name,attachment_kind,source_path,created_at",
            "id",
        ),
        (
            "source_anchors",
            "id,item_id,attachment_id,content_sha256,selector_version,page_index,page_label,rects_json,quote_exact,quote_prefix,quote_suffix,created_at,selector_json",
            "id",
        ),
        (
            "reading_states",
            "attachment_id,item_id,page_index,zoom,updated_at,view_state_json",
            "item_id",
        ),
        (
            "identifiers",
            "id,item_id,kind,normalized_value,display_value",
            "id",
        ),
        (
            "project_items",
            "project_id,item_id,added_at",
            "project_id,item_id",
        ),
        ("item_tags", "item_id,tag_id", "item_id,tag_id"),
        (
            "metadata_provenance",
            "item_id,field_name,source_kind,source_reference,confirmed_by_user,updated_at",
            "item_id,field_name",
        ),
        (
            "duplicate_candidates",
            "id,left_item_id,right_item_id,reason,confidence,decision,created_at,decided_at",
            "id",
        ),
        (
            "merge_operations",
            "id,survivor_item_id,merged_item_id,snapshot_json,state,created_at,undone_at",
            "id",
        ),
        ("collections", "id,name,description,concept_id", "id"),
        (
            "collection_relations",
            "id,source_id,target_id,type_id",
            "id",
        ),
        (
            "collection_memberships",
            "item_id,collection_id",
            "item_id,collection_id",
        ),
        (
            "item_relation_assignments",
            "item_id,relation_id",
            "item_id,relation_id",
        ),
        ("collection_state", "id,revision", "id"),
        (
            "nal_state",
            "id,revision,legacy_imported,relation_semantics_version",
            "id",
        ),
        ("nal_statements", "id,subject,predicate,origin", "rowid"),
        (
            "nal_annotations",
            "id,concept_id,statement_id,note_page_id",
            "rowid",
        ),
        ("nal_concept_kinds", "concept_id,kind", "concept_id"),
        (
            "nal_relation_rules",
            "relation_concept_id,kind",
            "relation_concept_id",
        ),
        (
            "terms",
            "id,text,normalized_text,language,provenance_kind,revision,created_at,updated_at",
            "id",
        ),
        (
            "concepts",
            "id,label,explanation,disambiguation,domain,tradition,period,provenance_kind,revision,created_at,updated_at,deleted_at",
            "id",
        ),
        (
            "concept_terms",
            "concept_id,term_id,relation_kind,created_at",
            "concept_id,term_id",
        ),
        (
            "research_object_revisions",
            "id,object_kind,object_id,revision,snapshot_json,editor_kind,change_reason,created_at",
            "id",
        ),
        (
            "semantic_representations",
            "id,subject_kind,subject_id,subject_revision,representation_kind,namespace,language,render_version,rendered_text,input_sha256,created_at",
            "id",
        ),
        (
            "embedding_spaces",
            "id,provider_id,region,model_id,model_revision,dimension,scalar_type,distance_metric,normalization,document_input_role,query_input_role,instruction_set_version,created_at,retired_at",
            "id",
        ),
        (
            "embedding_index_generations",
            "id,namespace,embedding_space_id,backend_kind,generation,status,relative_database_path,lance_table,lance_dataset_version,source_high_watermark,index_kind,config_json,member_count,created_at,activated_at,superseded_at",
            "id",
        ),
        (
            "embedding_records",
            "id,representation_id,embedding_space_id,index_generation_id,lance_table,lance_row_id,lance_dataset_version,vector_encoding,vector_norm,generation_run_id,status,error_code,created_at,updated_at,stale_at",
            "id",
        ),
        (
            "embedding_outbox_jobs",
            "id,embedding_record_id,attempt,status,idempotency_key,lease_owner,lease_expires_at,error_code,created_at,started_at,finished_at",
            "id",
        ),
        (
            "concept_search",
            "concept_id,terms,label,explanation,qualifiers",
            "concept_id",
        ),
        (
            "literature_search",
            "item_id,title,creators,identifiers,tags,full_text",
            "item_id,rowid",
        ),
        (
            "reading_note_sources",
            "block_id,page_id,note_type,color,current_anchor_id,paper_id_snapshot,paper_title_snapshot,attachment_id_snapshot,attachment_name_snapshot,content_sha256,page_index,page_label,rects_json,quote_exact,quote_prefix,quote_suffix,locator_version,created_at,updated_at,selector_json",
            "block_id",
        ),
        (
            "note_pages",
            "id,title,parent_page_id,position,status,is_favorite,revision,schema_version,created_at,updated_at,trashed_at,document_title",
            "id",
        ),
        (
            "note_blocks",
            "id,scope_kind,page_id,parent_block_id,position,block_type,props_json,content_json,plain_text,schema_version,revision,status,is_favorite,created_at,updated_at,trashed_at",
            "id",
        ),
        (
            "note_revisions",
            "id,page_id,page_revision,schema_version,document_json,reason,created_at,is_pinned",
            "page_id,page_revision",
        ),
    ] {
        result.insert(
            table.into(),
            rows(
                &db,
                &format!("select {columns} from {table} order by {order}"),
            )?,
        );
    }
    for (name, sql) in [
        (
            "noteCatalog",
            "select id,title,category,source_id,source_label,updated_at,status,is_favorite,favorite_position,revision,catalog_title_key,catalog_title_down,catalog_category_key,catalog_category_down,catalog_source_key,catalog_source_down,catalog_updated_ms from note_page_catalog where id='duplicate-merge-core-note'",
        ),
        (
            "noteTitleCache",
            "select page_id,body_title from note_catalog_title_cache where page_id='duplicate-merge-core-note'",
        ),
        (
            "noteMetadata",
            "select page_id,search_text,folded_text from note_catalog_metadata_text where page_id='duplicate-merge-core-note'",
        ),
        (
            "noteMetadataSearch",
            "select rowid,folded_text from note_catalog_metadata_search where rowid in (select rowid from note_catalog_metadata_text where page_id='duplicate-merge-core-note')",
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
        return Err("merge changed the Core note document or left SQL unsettled".into());
    }
    Ok(())
}

pub(super) fn placement(store: &AppStore, owner: &str) -> Result<(), Error> {
    let case = fixture::case();
    let db = store.connection()?;
    for (table, column, id) in [
        ("attachments", "id", case.id("attachment")),
        ("source_anchors", "id", case.id("anchor")),
        ("reading_states", "attachment_id", case.id("attachment")),
        ("identifiers", "id", case.id("identifier")),
    ] {
        let actual: String = db.query_row(
            &format!("select item_id from {table} where {column}=?1"),
            [id],
            |r| r.get(0),
        )?;
        if actual != owner {
            return Err(format!("actual merge/undo placement differs in {table}").into());
        }
    }
    let (source, label, metadata): (String, String, String) = db.query_row(
        "select paper_id_snapshot,(select source_label from note_page_catalog where id=?1),
         (select search_text from note_catalog_metadata_text where page_id=?1)
         from reading_note_sources where page_id=?1",
        [fixture::NOTE],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    let title: String = db.query_row(
        "select title from bibliographic_items where id=?1",
        [owner],
        |r| r.get(0),
    )?;
    if source != owner || label != title || !metadata.contains(&title) {
        return Err(
            "Core note source/catalog/metadata projections do not follow the actual owner".into(),
        );
    }
    Ok(())
}

pub(super) fn metadata(store: &AppStore) -> Result<Value, Error> {
    let db = store.connection()?;
    let scalar = |sql| -> Result<i64, Error> { Ok(db.query_row(sql, [], |r| r.get(0))?) };
    let terms = scalar(
        "select count(*) from research_object_revisions where object_kind='term' and revision=1 and json_extract(snapshot_json,'$.text')='Dynamic merge source collection' and json_extract(snapshot_json,'$.revision')=8 and json_extract(snapshot_json,'$.provenanceKind')='ai_accepted_after_edit'",
    )?;
    let concepts = scalar(
        "select count(*) from research_object_revisions where object_kind='concept' and revision=1 and json_extract(snapshot_json,'$.label')='Dynamic merge source collection' and json_extract(snapshot_json,'$.revision')=7 and json_extract(snapshot_json,'$.provenanceKind')='ai_accepted_after_edit'",
    )?;
    let representations = rows(
        &db,
        "select r.id,r.rendered_text,r.input_sha256,r.subject_revision from semantic_representations r join concepts c on c.id=r.subject_id where c.label='Dynamic merge source collection' and r.representation_kind='concept_identity'",
    )?;
    if terms != 1 || concepts != 1 || representations.as_array().is_none_or(Vec::is_empty) {
        return Err("actual trigger-mutated term/concept metadata was not read back".into());
    }
    for row in representations
        .as_array()
        .ok_or("representation rows missing")?
    {
        let rendered = row["rendered_text"]
            .as_str()
            .ok_or("rendered text missing")?;
        if row["subject_revision"] != 7
            || row["input_sha256"] != format!("{:x}", Sha256::digest(rendered.as_bytes()))
        {
            return Err("actual semantic revision/hash changed".into());
        }
    }
    let jobs = scalar(
        "select count(*) from embedding_outbox_jobs j join embedding_records e on e.id=j.embedding_record_id join semantic_representations r on r.id=e.representation_id join concepts c on c.id=r.subject_id where c.label='Dynamic merge source collection' and j.status='queued' and e.status='queued' and r.subject_revision=7",
    )?;
    let memberships = scalar(
        "select count(*) from collection_memberships where item_id='merge-main-survivor' and collection_id='merge-dynamic-collection'",
    )?;
    if jobs == 0 || memberships != 1 {
        return Err("actual T1 collection or semantic queue was not retained".into());
    }
    Ok(
        json!({"termRevisionSnapshots":terms,"conceptRevisionSnapshots":concepts,
        "actualRepresentations":representations,"queuedJobs":jobs,"dynamicSurvivorMemberships":memberships,
        "providerCalls":0,"syntheticEmbeddingConfigurationOnly":true}),
    )
}
