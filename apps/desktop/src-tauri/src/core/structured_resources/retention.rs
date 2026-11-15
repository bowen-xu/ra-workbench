//! Template structure changes preserve field bodies and their independent review state.
use super::{Error, Resource, content};
use crate::{
    ai::structured_resource_validation::{Flow, StructuredPolicy},
    hbs_storage::boundary::HbsSources,
};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashSet};

fn field_id(value: &Value) -> Result<&str, String> {
    value["field"]
        .as_str()
        .filter(|id| !id.is_empty())
        .ok_or_else(|| "保留字段缺少有效field身份".into())
}

fn retained(value: Option<&Value>) -> Result<Vec<Value>, String> {
    match value.and_then(|value| value.get("retainedFields")) {
        None => Ok(vec![]),
        Some(Value::Array(values)) => Ok(values.clone()),
        _ => Err("retainedFields必须为字段数组".into()),
    }
}

/// Regeneration may omit inactive fields, but it cannot silently erase them.
pub(super) fn preserve(previous: Option<&Value>, proposed: &mut Value) -> Result<(), String> {
    let previous = retained(previous)?;
    let existing = previous
        .iter()
        .map(|value| field_id(value).map(str::to_owned))
        .collect::<Result<HashSet<_>, _>>()?;
    let mut values = retained(Some(proposed))?;
    let mut ids = HashSet::new();
    for value in &values {
        let id = field_id(value)?;
        if !existing.contains(id) {
            return Err("不能新建保留字段；字段结构变更必须通过模板确认".into());
        }
        if !ids.insert(id.to_owned())
            || !["filled", "partial", "not_found"].contains(&value["status"].as_str().unwrap_or(""))
            || !value["statements"].is_array()
        {
            return Err("保留字段身份重复或正文结构无效".into());
        }
    }
    for value in previous {
        if ids.insert(field_id(&value)?.to_owned()) {
            values.push(value);
        }
    }
    let active = proposed["fields"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|field| field["field"].as_str())
        .collect::<HashSet<_>>();
    if values
        .iter()
        .any(|field| field_id(field).is_ok_and(|id| active.contains(id)))
    {
        return Err("现行字段与保留字段不能使用相同身份".into());
    }
    if !values.is_empty() || proposed.get("retainedFields").is_some() {
        proposed
            .as_object_mut()
            .ok_or("结构化阅读必须是JSON对象")?
            .insert("retainedFields".into(), Value::Array(values));
    }
    Ok(())
}

pub(super) fn reframe(resource: &mut Resource, template: &Resource) -> Result<(), Error> {
    let definitions = template
        .content
        .as_ref()
        .and_then(|value| value["fields"].as_array())
        .ok_or_else(|| Error::Invalid("采用的模板缺少字段定义".into()))?;
    let mut snapshot_fields = definitions.clone();
    for field in &mut snapshot_fields {
        field["claimIds"] = json!([]);
    }
    let snapshot = json!({"templateResourceId":template.id,"templateResourceRevision":template.revision,"fields":snapshot_fields});
    for value in [&mut resource.content, &mut resource.review_baseline]
        .into_iter()
        .flatten()
    {
        let active = value["fields"]
            .as_array()
            .cloned()
            .ok_or_else(|| Error::Invalid("原结构化阅读字段无效".into()))?;
        let archived = retained(Some(value)).map_err(Error::Invalid)?;
        let mut by_id = BTreeMap::new();
        for field in archived.iter().chain(&active) {
            by_id.insert(
                field_id(field).map_err(Error::Invalid)?.to_owned(),
                field.clone(),
            );
        }
        let mut next_active = vec![];
        for definition in definitions {
            let id = definition["id"]
                .as_str()
                .ok_or_else(|| Error::Invalid("采用的模板字段身份无效".into()))?;
            next_active.push(
                by_id
                    .remove(id)
                    .unwrap_or_else(|| json!({"field":id,"status":"not_found","statements":[]})),
            );
        }
        let mut next_retained = vec![];
        for field in archived.iter().chain(&active) {
            let id = field_id(field).map_err(Error::Invalid)?;
            if let Some(value) = by_id.remove(id) {
                next_retained.push(value);
            }
        }
        let object = value
            .as_object_mut()
            .ok_or_else(|| Error::Invalid("原结构化阅读必须是JSON对象".into()))?;
        object.insert("profileId".into(), json!("paper_template_v1"));
        object.remove("profileRefs");
        object.insert("paperTemplate".into(), snapshot.clone());
        object.insert("fields".into(), Value::Array(next_active));
        if !next_retained.is_empty() {
            object.insert("retainedFields".into(), Value::Array(next_retained));
        } else {
            object.remove("retainedFields");
        }
    }
    resource.reviewed_field_ids = remap_keys(resource, &resource.reviewed_field_ids);
    resource.pending_field_ids =
        content::pending(resource.content.as_ref(), resource.review_baseline.as_ref());
    Ok(())
}

/// Verification follows the field body when it moves between active and retained sections.
pub(super) fn remap_keys(resource: &Resource, keys: &[String]) -> Vec<String> {
    let active = resource
        .content
        .as_ref()
        .and_then(|v| v["fields"].as_array())
        .into_iter()
        .flatten()
        .filter_map(|field| field["field"].as_str())
        .collect::<HashSet<_>>();
    let inactive = resource
        .content
        .as_ref()
        .and_then(|v| v["retainedFields"].as_array())
        .into_iter()
        .flatten()
        .filter_map(|field| field["field"].as_str())
        .collect::<HashSet<_>>();
    keys.iter()
        .map(|key| {
            let id = key.strip_prefix("$retained:").unwrap_or(key);
            if inactive.contains(id) {
                format!("$retained:{id}")
            } else if active.contains(id) {
                id.to_owned()
            } else {
                key.clone()
            }
        })
        .collect()
}

/// A confirmed structure change must still leave a valid formal document.
pub(super) fn validate_reframed_with<S: HbsSources>(
    sources: &mut S,
    db: &rusqlite::Connection,
    before: &Resource,
    after: &Resource,
) -> Result<(), Flow<Error, <S::Policy as StructuredPolicy>::Pause>> {
    if let Some(content) = after.content.as_ref() {
        let mut validation_base = before.clone();
        validation_base.template_ref = after.template_ref.clone();
        let body = super::encode(content)?;
        let mut validation = sources
            .require()
            .map_err(|error| error.map_error(Error::Invalid))?;
        super::validation::validate_review_with(&mut validation, db, &validation_base, &body)
            .map_err(|error| {
                error.map_error(|issues| {
                    Error::Invalid(format!(
                        "采用该模板后正文不符合约定格式，正式内容和提案均已保留：{}",
                        issues
                            .iter()
                            .map(|issue| issue.message.as_str())
                            .collect::<Vec<_>>()
                            .join("；")
                    ))
                })
            })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regeneration_preserves_omitted_archived_fields_but_cannot_invent_inactive_structure() {
        let previous = json!({"fields":[{"field":"method","status":"not_found","statements":[]}],"retainedFields":[{"field":"question","status":"filled","statements":[{"id":"old","text":"已有正文。"}]}]});
        let mut proposed =
            json!({"fields":[{"field":"method","status":"not_found","statements":[]}]});
        preserve(Some(&previous), &mut proposed).unwrap();
        assert_eq!(proposed["retainedFields"], previous["retainedFields"]);
        proposed["retainedFields"] = json!([]);
        preserve(Some(&previous), &mut proposed).unwrap();
        assert_eq!(proposed["retainedFields"], previous["retainedFields"]);
        proposed["retainedFields"] =
            json!([{"field":"invented","status":"not_found","statements":[]}]);
        assert!(preserve(Some(&previous), &mut proposed).is_err());
    }
}
