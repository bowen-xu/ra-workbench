use super::*;
use rusqlite::{Connection, types::ValueRef};

const TABLES: &[(&str, &str)] = &[
    ("nal_state", "id"),
    ("terms", "id"),
    ("concepts", "id"),
    ("concept_terms", "concept_id,term_id"),
    ("research_object_revisions", "id"),
    ("semantic_representations", "id"),
    ("embedding_spaces", "id"),
    ("embedding_index_generations", "id"),
    ("embedding_records", "id"),
    ("embedding_outbox_jobs", "id"),
    ("nal_statements", "id"),
    ("nal_annotations", "id"),
    ("nal_concept_kinds", "concept_id"),
    ("nal_relation_rules", "relation_concept_id"),
    ("note_pages", "id"),
    ("note_blocks", "id"),
    ("note_revisions", "id"),
    ("concept_search", "concept_id"),
];

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
                ValueRef::Real(value) => Value::Number(
                    serde_json::Number::from_f64(value).ok_or("nonfinite synthetic SQL value")?,
                ),
                ValueRef::Text(value) => json!(std::str::from_utf8(value)?),
                ValueRef::Blob(value) => {
                    json!({"blobHex":value.iter().map(|byte|format!("{byte:02x}")).collect::<String>()})
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
    let mut output = serde_json::Map::new();
    for (table, order) in TABLES {
        // Both SQL identifiers come solely from this fixed acceptance table.
        output.insert(
            (*table).into(),
            rows(&db, &format!("select * from {table} order by {order}"))?,
        );
    }
    output.insert("syntheticEmbeddingConfiguration".into(),rows(&db,"select provider_id,enabled,region,model_id from ai_provider_settings where provider_id='qwen-embedding'")?);
    Ok(Value::Object(output))
}

pub(super) fn scalar(store: &AppStore, sql: &str) -> Result<i64, Error> {
    let db = store.connection()?;
    scalar_in(&db, sql)
}

fn scalar_in(db: &Connection, sql: &str) -> Result<i64, Error> {
    Ok(db.query_row(sql, [], |row| row.get(0))?)
}

pub(super) fn metadata(store: &AppStore) -> Result<Value, Error> {
    let db = store.connection()?;
    let revisions = rows(
        &db,
        "select object_kind,object_id,snapshot_json from research_object_revisions where object_kind in ('term','concept') order by id",
    )?;
    let mut term_count = 0;
    let mut concept_count = 0;
    for revision in revisions.as_array().ok_or("revision array missing")? {
        let id = revision["object_id"]
            .as_str()
            .ok_or("revision id missing")?;
        let actual = if revision["object_kind"] == "term" {
            term_count += 1;
            term(&db, id)?
        } else {
            concept_count += 1;
            concept(&db, id)?
        };
        if revision["snapshot_json"].as_str() != Some(serde_json::to_string(&actual)?.as_str()) {
            return Err("revision snapshot differs from actual SQL timestamps/counts and original Value serialization".into());
        }
    }
    if term_count == 0 || concept_count == 0 {
        return Err("bootstrap did not create actual term/concept revisions".into());
    }
    let bad_representations:i64=db.query_row("select count(*) from semantic_representations where subject_kind='concept' and representation_kind='concept_identity' and (subject_revision<>1 or namespace<>'accepted_concepts' or render_version<>'concept-identity-v1')",[],|row|row.get(0))?;
    if bad_representations != 0 {
        return Err("bootstrap representation metadata differs from original shape".into());
    }
    for row in rows(
        &db,
        "select rendered_text,input_sha256 from semantic_representations order by id",
    )?
    .as_array()
    .ok_or("representation array missing")?
    {
        let rendered = row["rendered_text"]
            .as_str()
            .ok_or("rendered text missing")?;
        if row["input_sha256"] != format!("{:x}", Sha256::digest(rendered.as_bytes())) {
            return Err("representation hash differs from complete rendered text".into());
        }
    }
    let jobs = scalar_in(
        &db,
        "select count(*) from embedding_outbox_jobs where status='queued'",
    )?;
    if jobs == 0
        || scalar_in(
            &db,
            "select count(*) from embedding_records where status='queued'",
        )? != jobs
        || scalar_in(
            &db,
            "select count(*) from embedding_index_generations where status='active' and namespace='accepted_concepts'",
        )? != 1
    {
        return Err(
            "synthetic bootstrap did not preserve the complete embedding queue/generation".into(),
        );
    }
    let reused = term(&db, "bootstrap-existing-term")?;
    if reused["createdAt"] != "1999-01-02T03:04:05.000Z"
        || reused["updatedAt"] != "2000-02-03T04:05:06.000Z"
        || scalar_in(
            &db,
            "select count(*) from concept_terms where term_id='bootstrap-existing-term'",
        )? != 1
    {
        return Err("existing term identity/metadata was replaced during import".into());
    }
    Ok(
        json!({"termRevisions":term_count,"conceptRevisions":concept_count,"queuedJobs":jobs,
        "actualSqlSnapshotsCompared":true,"existingTermMetadataPreserved":true,"representationHashesCompared":true}),
    )
}

fn term(db: &Connection, id: &str) -> Result<Value, Error> {
    Ok(db.query_row("select id,text,normalized_text,language,provenance_kind,revision,created_at,updated_at from terms where id=?1",[id],|row| {
        Ok(json!({"id":row.get::<_,String>(0)?,"text":row.get::<_,String>(1)?,"normalizedText":row.get::<_,String>(2)?,
            "language":row.get::<_,String>(3)?,"provenanceKind":row.get::<_,String>(4)?,"revision":row.get::<_,i64>(5)?,
            "createdAt":row.get::<_,String>(6)?,"updatedAt":row.get::<_,String>(7)?}))
    })?)
}

fn concept(db: &Connection, id: &str) -> Result<Value, Error> {
    let preferred: String = db.query_row(
        "select term_id from concept_terms where concept_id=?1 and relation_kind='preferred'",
        [id],
        |row| row.get(0),
    )?;
    let preferred = term(db, &preferred)?;
    let terms = {
        let mut query=db.prepare("select term_id from concept_terms where concept_id=?1 order by case relation_kind when 'preferred' then 0 else 1 end,created_at,term_id")?;
        query
            .query_map([id], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?
    };
    let terms = terms
        .iter()
        .map(|id| term(db, id))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(db.query_row("select id,label,explanation,disambiguation,domain,tradition,period,provenance_kind,revision,
        (select count(*) from definition_claims where concept_id=c.id),(select count(distinct anchor_id) from term_usages where concept_id=c.id),created_at,updated_at from concepts c where id=?1",[id],|row| {
        Ok(json!({"id":row.get::<_,String>(0)?,"preferredTerm":preferred,"terms":terms,"label":row.get::<_,String>(1)?,
            "explanation":row.get::<_,String>(2)?,"disambiguation":row.get::<_,String>(3)?,"domain":row.get::<_,String>(4)?,
            "tradition":row.get::<_,String>(5)?,"period":row.get::<_,String>(6)?,"provenanceKind":row.get::<_,String>(7)?,
            "revision":row.get::<_,i64>(8)?,"definitionCount":row.get::<_,i64>(9)?,"sourceCount":row.get::<_,i64>(10)?,
            "createdAt":row.get::<_,String>(11)?,"updatedAt":row.get::<_,String>(12)?}))
    })?)
}
