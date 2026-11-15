use serde_json::{Map, Value};
use std::collections::BTreeMap;

fn array(content: &Value) -> Option<&Vec<Value>> {
    content
        .get("fields")
        .or_else(|| content.get("claims"))
        .and_then(Value::as_array)
}
fn id(value: &Value) -> Option<&str> {
    value
        .get("field")
        .or_else(|| value.get("id"))
        .and_then(Value::as_str)
}

pub(super) fn fields(content: Option<&Value>) -> BTreeMap<String, Value> {
    let Some(content) = content else {
        return BTreeMap::new();
    };
    let Some(entries) = array(content) else {
        return BTreeMap::from([("content".into(), content.clone())]);
    };
    let mut result = entries
        .iter()
        .filter_map(|v| id(v).map(|id| (id.into(), v.clone())))
        .collect::<BTreeMap<_, _>>();
    for entry in content
        .get("retainedFields")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if let Some(id) = id(entry) {
            result.insert(format!("$retained:{id}"), entry.clone());
        }
    }
    let mut metadata = content.clone();
    if let Some(object) = metadata.as_object_mut() {
        object.remove("fields");
        object.remove("claims");
        object.remove("retainedFields");
    }
    result.insert("$metadata".into(), metadata);
    result.insert(
        "$order".into(),
        Value::Array(
            entries
                .iter()
                .filter_map(|v| id(v).map(|id| Value::String(id.into())))
                .collect(),
        ),
    );
    result
}

pub(super) fn pending(current: Option<&Value>, baseline: Option<&Value>) -> Vec<String> {
    let current = fields(current);
    let baseline = fields(baseline);
    let ids = current
        .keys()
        .chain(baseline.keys())
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    ids.into_iter()
        .filter(|id| current.get(id) != baseline.get(id))
        .collect()
}

pub(super) fn replace_fields(
    target: &mut Option<Value>,
    source: Option<&Value>,
    ids: &[String],
    baseline: bool,
) {
    if ids.iter().any(|id| id == "content") {
        *target = source.cloned();
        return;
    }
    if target.is_none() {
        let Some(source) = source else { return };
        let key = if source.get("claims").is_some() {
            "claims"
        } else {
            "fields"
        };
        *target = Some(Value::Object(Map::from_iter([(
            key.into(),
            Value::Array(vec![]),
        )])));
    }
    let Some(target) = target.as_mut() else {
        return;
    };
    let key = if target.get("claims").is_some() {
        "claims"
    } else {
        "fields"
    };
    if ids.iter().any(|id| id == "$metadata") {
        let entries = target.get(key).cloned().unwrap_or(Value::Array(vec![]));
        let retained = target.get("retainedFields").cloned();
        let mut metadata = source.cloned().unwrap_or_else(|| Value::Object(Map::new()));
        if let Some(object) = metadata.as_object_mut() {
            object.remove("claims");
            object.remove("fields");
            object.remove("retainedFields");
            object.insert(key.into(), entries);
            if let Some(retained) = retained {
                object.insert("retainedFields".into(), retained);
            }
        }
        *target = metadata;
    }
    let source_fields = fields(source);
    for field in ids.iter().filter_map(|id| id.strip_prefix("$retained:")) {
        let entries = target
            .as_object_mut()
            .unwrap()
            .entry("retainedFields")
            .or_insert_with(|| Value::Array(vec![]))
            .as_array_mut()
            .unwrap();
        let position = entries.iter().position(|entry| id(entry) == Some(field));
        match (position, source_fields.get(&format!("$retained:{field}"))) {
            (Some(index), Some(value)) => entries[index] = value.clone(),
            (Some(index), None) => {
                entries.remove(index);
            }
            (None, Some(value)) => entries.push(value.clone()),
            (None, None) => {}
        }
    }
    let order = source_fields
        .get("$order")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let source_key = source
        .map(|s| {
            if s.get("claims").is_some() {
                "id"
            } else {
                "field"
            }
        })
        .unwrap_or(if key == "claims" { "id" } else { "field" });
    let Some(entries) = target.get_mut(key).and_then(Value::as_array_mut) else {
        return;
    };
    if ids.iter().any(|id| id == "$order") {
        // Baselines may retain identity-only placeholders: order can be kept
        // independently while every field body remains an unresolved Diff.
        for field in &order {
            if let Some(field) = field.as_str()
                && entries.iter().all(|entry| id(entry) != Some(field))
                && baseline
            {
                entries.push(Value::Object(Map::from_iter([(
                    source_key.into(),
                    Value::String(field.into()),
                )])));
            }
        }
        entries.sort_by_key(|entry| {
            order
                .iter()
                .position(|v| v.as_str() == id(entry))
                .unwrap_or(usize::MAX)
        });
    }
    for field in ids.iter().filter(|field| !field.starts_with('$')) {
        let position = entries.iter().position(|entry| id(entry) == Some(field));
        match (position, source_fields.get(field)) {
            (Some(index), Some(value)) => entries[index] = value.clone(),
            (Some(index), None) => {
                entries.remove(index);
            }
            (None, Some(value)) => {
                let source_position = order.iter().position(|v| v.as_str() == Some(field));
                let index = source_position
                    .and_then(|position| {
                        order[position + 1..].iter().find_map(|following| {
                            entries
                                .iter()
                                .position(|entry| id(entry) == following.as_str())
                        })
                    })
                    .unwrap_or(entries.len());
                entries.insert(index, value.clone());
            }
            (None, None) => {}
        }
    }
}
