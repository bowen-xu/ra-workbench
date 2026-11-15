use rusqlite::{Connection, types::ValueRef};
use serde_json::{Map, Value, json};

/// Fixed queries only: no SQL, table name or path comes from the client.
fn rows(db: &Connection, table: &str, sql: &str, id: &str) -> Result<Value, String> {
    let mut statement = db.prepare(sql).map_err(|error| error.to_string())?;
    let columns: Vec<String> = statement
        .column_names()
        .iter()
        .map(|name| (*name).to_owned())
        .collect();
    let data = statement
        .query_map([id], |row| {
            let mut value = Map::new();
            for (index, column) in columns.iter().enumerate() {
                let cell = match row.get_ref(index)? {
                    ValueRef::Null => Value::Null,
                    ValueRef::Integer(value) => json!(value),
                    ValueRef::Real(value) => json!(value),
                    ValueRef::Text(value) => json!(String::from_utf8_lossy(value)),
                    ValueRef::Blob(value) => json!({"blobBytes":value.len()}),
                };
                value.insert(column.clone(), cell);
            }
            Ok(Value::Object(value))
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(json!({"table":table,"columns":columns,"rows":data}))
}
pub(crate) fn read(db: &mut Connection, kind: &str, id: &str) -> Result<Value, String> {
    let tx = db.transaction().map_err(|error| error.to_string())?;
    let snapshot = super::read(&tx, kind, id)?;
    let (saved, baseline) = match kind {
        "note" => (
            vec![
                rows(
                    &tx,
                    "note_pages",
                    "select * from note_pages where id=?1",
                    id,
                )?,
                rows(
                    &tx,
                    "note_blocks",
                    "select * from note_blocks where page_id=?1 order by position,id",
                    id,
                )?,
            ],
            vec![rows(
                &tx,
                "hbs_note_baselines",
                "select * from hbs_note_baselines where page_id=?1",
                id,
            )?],
        ),
        "structured" => (
            vec![rows(
                &tx,
                "structured_resources",
                "select id,kind,revision,content_json from structured_resources where id=?1",
                id,
            )?],
            vec![rows(
                &tx,
                "structured_resources",
                "select id,revision,baseline_json from structured_resources where id=?1",
                id,
            )?],
        ),
        "text" => (
            vec![rows(
                &tx,
                "hbs_text_documents",
                "select id,revision,content from hbs_text_documents where id=?1",
                id,
            )?],
            vec![rows(
                &tx,
                "hbs_text_documents",
                "select id,revision,baseline from hbs_text_documents where id=?1",
                id,
            )?],
        ),
        _ => return Err("MODEL_INVALID_KIND：未知文档类型".into()),
    };
    let preservation = rows(
        &tx,
        "hbs_preservations",
        "select * from hbs_preservations where kind||':'||resource_id=?1",
        &format!("{kind}:{id}"),
    )?;
    tx.commit().map_err(|error| error.to_string())?;
    Ok(json!({"snapshot":snapshot,"storage":{"S":saved,"B":baseline,"S′":[preservation]}}))
}
