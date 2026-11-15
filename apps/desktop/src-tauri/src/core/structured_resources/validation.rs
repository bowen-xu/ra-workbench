use super::{Resource, ValidationIssue};
use crate::ai::structured_resource_validation::{
    Failure, FailureCause, Flow, StructuredPolicy, ValidationSources,
};
use crate::hbs_storage::boundary::HbsSources;
use rusqlite::Connection;
use serde_json::Value;

type Checked<S> = Result<
    Value,
    Flow<Vec<ValidationIssue>, <<S as HbsSources>::Policy as StructuredPolicy>::Pause>,
>;

#[path = "validation/evidence.rs"]
mod evidence;
use evidence::validate_field_sources_with;

pub(super) fn validate_review_with<P: StructuredPolicy>(
    sources: &mut ValidationSources<'_, P>,
    db: &Connection,
    resource: &Resource,
    raw: &str,
) -> Result<Value, Flow<Vec<ValidationIssue>, P::Pause>> {
    validate_inner_with(sources, db, resource, raw, None, true, false)
}
fn resource_failure(error: super::Error) -> Failure<Vec<ValidationIssue>> {
    let message = error.to_string();
    let cause = match &error {
        super::Error::Database(error) => FailureCause::SourceDatabase(Box::new(
            crate::runtime_plugins::native::SafeDatabaseError::sql("source-read", error),
        )),
        _ => FailureCause::Original,
    };
    Failure {
        error: vec![issue(&message)],
        cause,
    }
}
pub(super) fn validate_write_with<S: HbsSources>(
    sources: &mut S,
    db: &Connection,
    resource: &Resource,
    raw: &str,
    source: Option<&str>,
    human: bool,
) -> Checked<S> {
    validate_inner_with(sources, db, resource, raw, source, human, true)
}
pub(super) fn validate_review_lazy_with<S: HbsSources>(
    sources: &mut S,
    db: &Connection,
    resource: &Resource,
    raw: &str,
) -> Checked<S> {
    validate_inner_with(sources, db, resource, raw, None, true, false)
}
fn validate_inner_with<S: HbsSources>(
    sources: &mut S,
    db: &Connection,
    resource: &Resource,
    raw: &str,
    source: Option<&str>,
    human: bool,
    preserve_inactive: bool,
) -> Checked<S> {
    let mut value: Value = serde_json::from_str(raw)
        .map_err(|error| {
            vec![ValidationIssue {
                path: format!("line {} column {}", error.line(), error.column()),
                code: "invalid_json".into(),
                message: error.to_string(),
            }]
        })
        .map_err(Flow::original)?;
    if resource.kind == "portrait" && !value["coverage"].is_string() {
        return Err(Flow::original(vec![ValidationIssue {
            path: "$.coverage".into(),
            code: "invalid_type".into(),
            message: "coverage 必须是描述实际阅读范围的字符串，不能是对象、数组或 null".into(),
        }]));
    }
    if resource.kind == "structuredReading" {
        if resource.template_ref.is_none()
            && (value["profileId"] == "paper_template_v1" || value.get("paperTemplate").is_some())
        {
            return Err(Flow::original(vec![issue(
                "本文阅读模板尚未确认并采用；不能直接使用画像建议中的模板生成或写入正式阅读记录",
            )]));
        }
        let prior = if preserve_inactive {
            resource.content.clone()
        } else {
            Some(value.clone())
        };
        super::retention::preserve(prior.as_ref(), &mut value)
            .map_err(|message| Flow::original(vec![issue(&message)]))?;
    }
    let validation = if resource.kind == "readingTemplate" {
        sources
            .policy_only()
            .map_err(|error| error.map_error(|message| vec![issue(&message)]))?
            .validate_editable_template(value)
            .map_err(|error| {
                error.map_error(|error| ValidationIssue {
                    path: "$".into(),
                    code: "invalid_content".into(),
                    message: error.to_string(),
                })
            })
    } else {
        let mut validation = sources
            .require()
            .map_err(|error| error.map_error(|message| vec![issue(&message)]))?;
        let sources = &mut validation;
        let field_sources = super::provenance::for_validation(db, resource, &value, source)
            .map_err(|error| Flow::Fail(resource_failure(error)))?;
        validate_field_sources_with(sources.reading, db, resource, &value, &field_sources)
            .map_err(|failure| Flow::Fail(failure.map_error(|issue| vec![issue])))?;
        let source_ids = field_sources
            .values()
            .flatten()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let mut checked = value.clone();
        let retained = checked
            .as_object_mut()
            .and_then(|object| object.remove("retainedFields"));
        if let Some(retained) = retained.as_ref() {
            validate_retained_with(sources, db, resource, retained, &source_ids, human)
                .map_err(|error| error.map_error(|message| vec![issue(&message)]))?;
        }
        let has_manual =
            separate_manual_statements(&mut checked, resource, human).map_err(|message| {
                Flow::original(vec![ValidationIssue {
                    path: "$.fields".into(),
                    code: "invalid_manual_content".into(),
                    message,
                }])
            })?;
        crate::ai::structured_resource_validation::validate_many_with(
            sources,
            db,
            resource.paper_id.as_deref().unwrap_or_default(),
            &resource.kind,
            checked,
            &source_ids,
        )
        .map_err(|error| error.map_error(content_issue))
        .map(|validated| {
            if has_manual || retained.is_some() {
                value
            } else {
                validated
            }
        })
    };
    let value = validation.map_err(|error| error.map_error(|issue| vec![issue]))?;
    if resource.kind == "structuredReading"
        && let Some(reference) = &resource.template_ref
    {
        let expected:Option<String>=db.query_row("select content_json from structured_resource_revisions where resource_id=?1 and revision=?2",rusqlite::params![reference.resource_id,reference.revision],|r|r.get(0)).map_err(|error|Flow::Fail(Failure::sql(vec![issue("无法读取采用的模板版本")],"read",&error)))?;
        if let Some(raw) = expected
            && let Ok(template) = serde_json::from_str::<Value>(&raw)
        {
            let expected = template["fields"]
                .as_array()
                .map(|fields| {
                    fields
                        .iter()
                        .filter_map(|f| f["id"].as_str())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let actual = value["fields"]
                .as_array()
                .map(|fields| {
                    fields
                        .iter()
                        .filter_map(|f| f["field"].as_str())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            if expected != actual {
                return Err(Flow::original(vec![issue(
                    "写入字段或顺序与当前采用的模板不一致",
                )]));
            }
            if let Some(snapshot) = value.get("paperTemplate") {
                if normalized_fields(&snapshot["fields"]) != normalized_fields(&template["fields"])
                {
                    return Err(Flow::original(vec![issue(
                        "本文模板字段定义与采用的准确模板版本不一致",
                    )]));
                }
                if snapshot.get("templateResourceId").and_then(Value::as_str)
                    != Some(reference.resource_id.as_str())
                    || snapshot
                        .get("templateResourceRevision")
                        .and_then(Value::as_i64)
                        != Some(reference.revision)
                {
                    return Err(Flow::original(vec![issue("本文模板身份与采用版本不一致")]));
                }
            } else {
                return Err(Flow::original(vec![issue(
                    "已采用模板的阅读记录必须携带准确模板身份与版本",
                )]));
            }
        }
    }
    Ok(value)
}
fn separate_manual_statements(
    value: &mut Value,
    resource: &Resource,
    human: bool,
) -> Result<bool, String> {
    if resource.kind != "structuredReading" {
        return Ok(false);
    }
    let old = super::content::fields(resource.content.as_ref());
    let mut has_manual = false;
    let Some(fields) = value["fields"].as_array_mut() else {
        return Ok(false);
    };
    for field in fields {
        let field_id = field["field"].as_str().unwrap_or_default().to_owned();
        let status = field["status"].clone();
        let Some(statements) = field["statements"].as_array_mut() else {
            continue;
        };
        let mut retained = vec![];
        for statement in std::mem::take(statements) {
            let role = statement["epistemicRole"].as_str().unwrap_or_default();
            let manual = ["working_note", "hypothesis", "question"].contains(&role);
            let unchanged = old
                .get(&field_id)
                .or_else(|| old.get(&format!("$retained:{field_id}")))
                .and_then(|f| f["statements"].as_array())
                .is_some_and(|values| values.contains(&statement));
            if manual && (human || unchanged) {
                if status != "filled"
                    || statement["id"]
                        .as_str()
                        .is_none_or(|id| id.is_empty() || id.len() > 80)
                    || statement["text"]
                        .as_str()
                        .is_none_or(|text| text.trim().is_empty())
                    || statement.as_object().is_none_or(|object| {
                        object.keys().any(|key| {
                            ![
                                "id",
                                "text",
                                "epistemicRole",
                                "evidenceItemIds",
                                "evidenceQuotes",
                            ]
                            .contains(&key.as_str())
                        })
                    })
                    || statement["evidenceItemIds"].as_array().is_none_or(|ids| {
                        ids.iter()
                            .any(|id| id.as_str().is_none_or(|id| id.is_empty() || id.len() > 80))
                    })
                {
                    return Err(format!("人工字段{field_id}的正文或来源格式无效"));
                }
                has_manual = true;
            } else {
                retained.push(statement);
            }
        }
        *statements = retained;
        if statements.is_empty() {
            field["status"] = serde_json::json!("not_found");
        }
    }
    Ok(has_manual)
}
fn validate_retained_with<P: StructuredPolicy>(
    sources: &mut ValidationSources<'_, P>,
    db: &Connection,
    resource: &Resource,
    retained: &Value,
    source_ids: &[String],
    human: bool,
) -> Result<(), Flow<String, P::Pause>> {
    let mut value = serde_json::json!({"fields":retained});
    separate_manual_statements(&mut value, resource, human).map_err(Flow::original)?;
    for field in value["fields"].as_array().into_iter().flatten() {
        let mut field = field.clone();
        field["field"] = serde_json::json!("research_question");
        let mut fields = vec![field];
        fields.extend(
            [
                "approach",
                "key_result",
                "conclusion",
                "limitation",
                "open_question",
            ]
            .map(|id| serde_json::json!({"field":id,"status":"not_found","statements":[]})),
        );
        crate::ai::structured_resource_validation::validate_many_with(sources,
            db,
            resource.paper_id.as_deref().unwrap_or_default(),
            "structuredReading",
            serde_json::json!({"kind":"paper_overview_unit_v1","schemaVersion":1,"profileId":"generic_overview_v1","fields":fields}),
            source_ids,
        ).map_err(|error| error.map_error(|cause| cause.to_string()))?;
    }
    Ok(())
}
fn content_issue(error: crate::ai::error::AiError) -> ValidationIssue {
    let (path, message) = match error {
        crate::ai::error::AiError::Validation { path, message } => (path, message),
        other => ("$".into(), other.to_string()),
    };
    ValidationIssue {
        path,
        code: "invalid_content".into(),
        message,
    }
}
pub(super) fn normalized_fields(value: &Value) -> Value {
    let mut value = value.clone();
    if let Some(fields) = value.as_array_mut() {
        for field in fields {
            if let Some(field) = field.as_object_mut() {
                field.remove("claimIds");
            }
        }
    }
    value
}
fn issue(message: &str) -> ValidationIssue {
    ValidationIssue {
        path: "$.fields".into(),
        code: "template_mismatch".into(),
        message: message.into(),
    }
}
