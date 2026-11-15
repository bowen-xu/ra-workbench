//! Exact field provenance follows content versions, including mixed legacy views.
use super::{Error, Resource, content, decode, encode};
use rusqlite::{Connection, params};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn field_sources(
    db: &Connection,
    id: &str,
    revision: i64,
    field: &str,
) -> Result<Vec<String>, Error> {
    let mut stmt=db.prepare("select source_snapshot_id from structured_resource_field_sources where resource_id=?1 and revision=?2 and field_id=?3 order by source_snapshot_id")?;
    Ok(stmt
        .query_map(params![id, revision, field], |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?)
}
pub(super) fn set_field_sources(
    db: &Connection,
    id: &str,
    revision: i64,
    field: &str,
    sources: &[String],
) -> Result<(), Error> {
    db.execute("delete from structured_resource_field_sources where resource_id=?1 and revision=?2 and field_id=?3",params![id,revision,field])?;
    for source in sources {
        db.execute("insert into structured_resource_field_sources(resource_id,revision,field_id,source_snapshot_id) values(?1,?2,?3,?4)",params![id,revision,field,source])?;
    }
    Ok(())
}
pub(super) fn persist(
    db: &Connection,
    resource: &Resource,
    explicit: Option<&str>,
    fallback: Option<&str>,
) -> Result<(), Error> {
    if resource.kind == "readingTemplate" {
        return Ok(());
    }
    let mut stmt=db.prepare("select revision,content_json,source_snapshot_id from structured_resource_revisions where resource_id=?1 and revision<?2 order by revision desc")?;
    let history = stmt
        .query_map(params![resource.id, resource.revision], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, Option<String>>(2)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    for (field, value) in content::fields(resource.content.as_ref()) {
        let mut sources = vec![];
        for (revision, raw, source) in &history {
            let old: Option<Value> = raw.as_deref().map(decode).transpose()?;
            let fields = content::fields(old.as_ref());
            let old_field = matching_key(&fields, &field, &value);
            let matches = old_field.is_some();
            if explicit.is_some() && !matches {
                break;
            }
            if matches {
                sources = field_sources(db, &resource.id, *revision, old_field.unwrap())?;
                if sources.is_empty() {
                    sources.extend(source.iter().cloned());
                }
                break;
            }
        }
        if sources.is_empty()
            && explicit.is_none()
            && let Some((revision, _, _)) = history.first()
        {
            sources = field_sources(db, &resource.id, *revision, &field)?;
        }
        if sources.is_empty() {
            sources.extend(explicit.or(fallback).map(str::to_owned));
        }
        set_field_sources(db, &resource.id, resource.revision, &field, &sources)?;
    }
    Ok(())
}
pub(super) fn for_validation(
    db: &Connection,
    resource: &Resource,
    value: &Value,
    explicit: Option<&str>,
) -> Result<BTreeMap<String, Vec<String>>, Error> {
    let old = content::fields(resource.content.as_ref());
    let mut output = BTreeMap::new();
    for (field, new) in content::fields(Some(value)) {
        let sources = if explicit.is_some() && old.get(&field) != Some(&new) {
            explicit.map(|s| vec![s.to_owned()]).unwrap_or_default()
        } else {
            let mut sources = field_sources(db, &resource.id, resource.revision, &field)?;
            if old.get(&field) != Some(&new) {
                let mut history=db.prepare("select revision,content_json,source_snapshot_id from structured_resource_revisions where resource_id=?1 order by revision desc")?;
                let versions = history
                    .query_map([&resource.id], |r| {
                        Ok((
                            r.get::<_, i64>(0)?,
                            r.get::<_, Option<String>>(1)?,
                            r.get::<_, Option<String>>(2)?,
                        ))
                    })?
                    .collect::<Result<Vec<_>, _>>()?;
                for (revision, raw, source) in versions {
                    let value: Option<Value> = raw.as_deref().map(decode).transpose()?;
                    if let Some(key) = matching_key(&content::fields(value.as_ref()), &field, &new)
                    {
                        sources = field_sources(db, &resource.id, revision, key)?;
                        if sources.is_empty() {
                            sources.extend(source);
                        }
                        break;
                    }
                }
            }
            if sources.is_empty() {
                sources.extend(explicit.map(str::to_owned));
            }
            sources
        };
        output.insert(field, sources);
    }
    Ok(output)
}
fn matching_key<'a>(
    fields: &'a BTreeMap<String, Value>,
    field: &'a str,
    value: &Value,
) -> Option<&'a str> {
    if fields.get(field) == Some(value) {
        return Some(field);
    }
    let alias = field
        .strip_prefix("$retained:")
        .map(str::to_owned)
        .unwrap_or_else(|| format!("$retained:{field}"));
    fields
        .get_key_value(&alias)
        .filter(|(_, previous)| *previous == value)
        .map(|(key, _)| key.as_str())
}
pub(super) fn sources(
    reading_source: &dyn crate::ai::structured_resource_validation::ExplicitReadingEvidence,
    db: &Connection,
    resource: &Resource,
) -> Result<Value, Error> {
    let Some(paper) = resource.paper_id.as_deref() else {
        return Ok(Value::Array(vec![]));
    };
    let mut output: BTreeMap<String, (Value, BTreeSet<String>)> = BTreeMap::new();
    for (field, value) in content::fields(resource.content.as_ref()) {
        for source in field_sources(db, &resource.id, resource.revision, &field)? {
            let bundle = crate::ai::structured_resource_validation::explicit_evidence(
                reading_source,
                db,
                paper,
                &source,
            )
            .map_err(Error::from)?;
            let raw = encode(&value)?;
            for item in bundle.items {
                if !raw.contains(&format!("\"{}\"", item.citation_key)) {
                    continue;
                }
                let key = format!("{source}:{}", item.id);
                let item = serde_json::to_value(item)
                    .map_err(|_| Error::Invalid("无法读取原文依据".into()))?;
                output
                    .entry(key)
                    .or_insert_with(|| (item, BTreeSet::new()))
                    .1
                    .insert(field.clone());
            }
        }
    }
    Ok(Value::Array(
        output
            .into_values()
            .map(|(mut value, fields)| {
                value["fieldIds"] = serde_json::json!(fields);
                value
            })
            .collect(),
    ))
}
