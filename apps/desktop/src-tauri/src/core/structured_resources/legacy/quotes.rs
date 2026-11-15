//! Canonical quotes are resolved only through each imported field's frozen sources.
use crate::structured_resources::Error;
use rusqlite::Connection;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

type Quotes = BTreeMap<String, BTreeSet<String>>;

pub(super) fn fill(
    reading_source: &dyn crate::ai::structured_resource_validation::ExplicitReadingEvidence,
    db: &Connection,
    paper: &str,
    content: &mut Value,
    field_sources: &BTreeMap<String, Vec<String>>,
) -> Result<(), Error> {
    fill_with(content, field_sources, |source| {
        crate::ai::structured_resource_validation::explicit_evidence(
            reading_source,
            db,
            paper,
            source,
        )
        .map(|bundle| {
            bundle
                .items
                .into_iter()
                .map(|item| (item.citation_key, item.quote_exact))
                .collect()
        })
        .map_err(Error::from)
    })
}

fn fill_with(
    content: &mut Value,
    field_sources: &BTreeMap<String, Vec<String>>,
    mut load: impl FnMut(&str) -> Result<Vec<(String, String)>, Error>,
) -> Result<(), Error> {
    // Never leave a partially canonicalized body when one later field fails.
    let mut candidate = content.clone();
    let mut cache = BTreeMap::<String, Vec<(String, String)>>::new();
    for property in ["fields", "retainedFields"] {
        let Some(fields) = candidate.get_mut(property) else {
            continue;
        };
        for field in fields.as_array_mut().ok_or_else(invalid)? {
            let id = field["field"].as_str().ok_or_else(invalid)?.to_owned();
            let retained = format!("$retained:{id}");
            let keys = if property == "retainedFields" {
                [retained.as_str(), id.as_str()]
            } else {
                [id.as_str(), retained.as_str()]
            };
            let sources = keys.into_iter().find_map(|key| field_sources.get(key));
            let mut quotes = None;
            for statement in field["statements"].as_array_mut().ok_or_else(invalid)? {
                let ids = statement["evidenceItemIds"]
                    .as_array()
                    .ok_or_else(invalid)?
                    .iter()
                    .map(|id| id.as_str().map(str::to_owned).ok_or_else(invalid))
                    .collect::<Result<Vec<_>, _>>()?;
                if ids.is_empty() {
                    continue;
                }
                if quotes.is_none() {
                    let sources =
                        sources
                            .filter(|sources| !sources.is_empty())
                            .ok_or_else(|| {
                                Error::Invalid(format!(
                                    "字段{id}缺少可恢复引用的冻结来源；旧数据未改动"
                                ))
                            })?;
                    let mut scoped = Quotes::new();
                    for source in sources {
                        if !cache.contains_key(source) {
                            cache.insert(source.clone(), load(source)?);
                        }
                        for (citation, quote) in &cache[source] {
                            scoped
                                .entry(citation.clone())
                                .or_default()
                                .insert(quote.clone());
                        }
                    }
                    quotes = Some(scoped);
                }
                fill_statement(statement, &ids, quotes.as_ref().ok_or_else(invalid)?)?;
            }
        }
    }
    *content = candidate;
    Ok(())
}

fn fill_statement(statement: &mut Value, ids: &[String], scoped: &Quotes) -> Result<(), Error> {
    let mut exact = BTreeMap::new();
    for id in ids {
        let quotes = scoped.get(id).ok_or_else(|| {
            Error::Invalid(format!("引用{id}不在本字段的冻结来源中；旧数据未改动"))
        })?;
        let Some(quote) = quotes
            .iter()
            .next()
            .filter(|quote| quotes.len() == 1 && !quote.trim().is_empty())
        else {
            return Err(Error::Invalid(format!(
                "引用{id}的冻结原文缺失或存在歧义；旧数据未改动"
            )));
        };
        if exact.insert(id.as_str(), quote.as_str()).is_some() {
            return Err(invalid());
        }
    }
    let mut supplied = match statement.get("evidenceQuotes") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(quotes)) => quotes.clone(),
        _ => return Err(invalid()),
    };
    let mut seen = BTreeSet::new();
    for entry in &supplied {
        let id = entry["evidenceItemId"].as_str().ok_or_else(invalid)?;
        let quote = entry["quote"].as_str().ok_or_else(invalid)?;
        if !seen.insert(id.to_owned())
            || quote.trim().is_empty()
            || exact.get(id).is_none_or(|source| !source.contains(quote))
        {
            return Err(Error::Invalid(format!(
                "引用{id}的摘录不是对应冻结原文的连续片段；旧数据未改动"
            )));
        }
    }
    for id in ids {
        if !seen.contains(id) {
            supplied.push(json!({"evidenceItemId":id,"quote":exact[id.as_str()]}));
        }
    }
    statement
        .as_object_mut()
        .ok_or_else(invalid)?
        .insert("evidenceQuotes".into(), json!(supplied));
    Ok(())
}

fn invalid() -> Error {
    Error::Invalid("旧结构化阅读引用格式无效；旧数据未改动".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(id: &str) -> Value {
        json!({"field":id,"status":"filled","statements":[{"id":id,"epistemicRole":"source_paraphrase","text":"保存正文","evidenceItemIds":["P1-T1"]}]})
    }

    #[test]
    fn same_citation_uses_each_field_source_and_retained_alias() {
        let mut content =
            json!({"fields":[field("question"),field("result")],"retainedFields":[field("old")]});
        let sources = BTreeMap::from([
            ("question".into(), vec!["a".into()]),
            ("result".into(), vec!["b".into()]),
            ("$retained:old".into(), vec!["c".into()]),
        ]);
        fill_with(&mut content, &sources, |source| {
            Ok(vec![("P1-T1".into(), format!("Exact {source}."))])
        })
        .unwrap();
        assert_eq!(
            content["fields"][0]["statements"][0]["evidenceQuotes"][0]["quote"],
            "Exact a."
        );
        assert_eq!(
            content["fields"][1]["statements"][0]["evidenceQuotes"][0]["quote"],
            "Exact b."
        );
        assert_eq!(
            content["retainedFields"][0]["statements"][0]["evidenceQuotes"][0]["quote"],
            "Exact c."
        );
    }

    #[test]
    fn contiguous_existing_quote_is_preserved_and_invalid_quote_changes_nothing() {
        let sources = BTreeMap::from([("question".into(), vec!["a".into()])]);
        let mut content = json!({"fields":[field("question")]});
        content["fields"][0]["statements"][0]["evidenceQuotes"] =
            json!([{"evidenceItemId":"P1-T1","quote":"Exact"}]);
        fill_with(&mut content, &sources, |_| {
            Ok(vec![("P1-T1".into(), "Exact original.".into())])
        })
        .unwrap();
        assert_eq!(
            content["fields"][0]["statements"][0]["evidenceQuotes"][0]["quote"],
            "Exact"
        );
        content["fields"][0]["statements"][0]["evidenceQuotes"][0]["quote"] = json!("Invented");
        let before = content.clone();
        assert!(
            fill_with(&mut content, &sources, |_| Ok(vec![(
                "P1-T1".into(),
                "Exact original.".into()
            )]))
            .is_err()
        );
        assert_eq!(content, before);
    }

    #[test]
    fn missing_or_ambiguous_citations_fail_without_partial_changes() {
        let sources = BTreeMap::from([("question".into(), vec!["a".into(), "b".into()])]);
        for missing in [true, false] {
            let mut content = json!({"fields":[field("question")]});
            let before = content.clone();
            assert!(
                fill_with(&mut content, &sources, |source| {
                    Ok(if missing {
                        vec![]
                    } else {
                        vec![("P1-T1".into(), source.into())]
                    })
                })
                .is_err()
            );
            assert_eq!(content, before);
        }
    }

    #[test]
    fn empty_manual_sources_do_not_invent_quotes_or_load_a_broader_scope() {
        let mut content = json!({"fields":[{"field":"note","statements":[{"id":"manual","epistemicRole":"working_note","text":"人工正文","evidenceItemIds":[]}]}]});
        let before = content.clone();
        fill_with(&mut content, &BTreeMap::new(), |_| {
            panic!("empty IDs must not load evidence")
        })
        .unwrap();
        assert_eq!(content, before);
    }
}
