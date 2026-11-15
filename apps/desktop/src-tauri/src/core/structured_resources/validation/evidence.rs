//! Bounded evidence diagnostics for the exact field and quote being validated.
use super::*;

pub(super) fn validate_field_sources_with(
    reading_source: &dyn crate::ai::structured_resource_validation::ReadingValidationEvidence,
    db: &Connection,
    resource: &Resource,
    value: &Value,
    sources: &std::collections::BTreeMap<String, Vec<String>>,
) -> Result<(), Failure<ValidationIssue>> {
    let paper = resource.paper_id.as_deref().unwrap_or_default();
    for (field, field_value) in super::super::content::fields(Some(value)) {
        let (collection, identity) = if resource.kind == "portrait" {
            ("claims", "id")
        } else {
            ("fields", "field")
        };
        let index = value[collection]
            .as_array()
            .and_then(|fields| fields.iter().position(|v| v[identity] == field))
            .unwrap_or(0);
        let base_path = format!("$.{collection}[{index}]");
        let value = field_value;
        let mut evidence = vec![];
        for source in sources.get(&field).into_iter().flatten() {
            evidence.extend(
                crate::ai::structured_resource_validation::evidence_with(
                    reading_source,
                    db,
                    paper,
                    Some(source),
                )
                .map_err(|failure| {
                    failure.map_error(|message| ValidationIssue {
                        path: base_path.clone(),
                        code: "invalid_evidence".into(),
                        message,
                    })
                })?
                .items,
            );
        }
        if evidence.is_empty() && sources.get(&field).is_none_or(Vec::is_empty) {
            evidence = crate::ai::structured_resource_validation::evidence_with(
                reading_source,
                db,
                paper,
                None,
            )
            .map_err(|failure| {
                failure.map_error(|message| ValidationIssue {
                    path: base_path.clone(),
                    code: "invalid_evidence".into(),
                    message,
                })
            })?
            .items;
        }
        let statements = value["statements"]
            .as_array()
            .cloned()
            .unwrap_or_else(|| vec![value.clone()]);
        for (statement_index, statement) in statements.into_iter().enumerate() {
            let statement_path = if resource.kind == "portrait" {
                base_path.clone()
            } else {
                format!("{base_path}.statements[{statement_index}]")
            };
            if let Some(ids) = statement["evidenceItemIds"].as_array() {
                for id in ids {
                    if !evidence
                        .iter()
                        .any(|e| Some(e.citation_key.as_str()) == id.as_str())
                    {
                        return Err(Failure::original(ValidationIssue {
                            path: format!("{statement_path}.evidenceItemIds"),
                            code: "invalid_evidence".into(),
                            message: format!("字段{field}引用了本字段允许范围外的来源{id}"),
                        }));
                    }
                }
            }
            let quotes = statement
                .get("evidenceQuotes")
                .or_else(|| statement.get("evidence"))
                .and_then(Value::as_array);
            let quote_key = if statement.get("evidenceQuotes").is_some() {
                "evidenceQuotes"
            } else {
                "evidence"
            };
            for (quote_index, quote) in quotes.into_iter().flatten().enumerate() {
                let originals = evidence
                    .iter()
                    .filter(|e| quote["evidenceItemId"] == e.citation_key)
                    .map(|e| e.quote_exact.as_str())
                    .collect::<Vec<_>>();
                validate_quote(&field, quote, &originals).map_err(|message| {
                    Failure::original(ValidationIssue {
                        path: format!(
                            "{statement_path}.{quote_key}[{quote_index}].{}",
                            if originals.is_empty() {
                                "evidenceItemId"
                            } else {
                                "quote"
                            }
                        ),
                        code: "invalid_evidence".into(),
                        message,
                    })
                })?;
            }
        }
    }
    Ok(())
}
fn validate_quote(field: &str, quote: &Value, originals: &[&str]) -> Result<(), String> {
    let field = diagnostic_excerpt(field, 64);
    let id = diagnostic_excerpt(quote["evidenceItemId"].as_str().unwrap_or("<非字符串>"), 80);
    if originals.is_empty() {
        return Err(format!(
            "字段{field}的证据 ID {id} 不在当前字段允许的原文证据范围内；请重新读取已授权来源，使用工具返回的 evidenceItemId，不要根据页码猜测 ID。"
        ));
    }
    if !quote["quote"]
        .as_str()
        .is_some_and(|q| originals.iter().any(|text| text.contains(q)))
    {
        return Err(format!(
            "字段{field}的证据 ID {id} 有效，但 quote 不是该证据 quoteExact 的逐字连续子串。提交引文片段：{}；对应原文开头片段（不代表最近匹配）：{}。请读取该 ID 对应的 quoteExact 并直接复制，保留空白、换行、断词和字符；勿改写、拼接或只缩成不能支持判断的单词。",
            diagnostic_excerpt(quote["quote"].as_str().unwrap_or("<非字符串>"), 200),
            diagnostic_excerpt(originals[0], 200),
        ));
    }
    Ok(())
}
fn diagnostic_excerpt(text: &str, limit: usize) -> String {
    let encoded = serde_json::to_string(text).unwrap_or_default();
    let prefix: String = encoded.chars().take(limit).collect();
    if encoded.chars().count() > limit {
        format!("{prefix}…（已截断，原行 {} 字符）", text.chars().count())
    } else {
        prefix
    }
}

#[cfg(test)]
mod quote_diagnostic_tests {
    use super::*;
    #[test]
    fn distinguishes_missing_identity_from_mismatched_literal_quote() {
        let quote = serde_json::json!({"evidenceItemId":"ev-page-7","quote":"different"});
        assert!(
            validate_quote("claim_3", &quote, &[])
                .unwrap_err()
                .contains("不在当前字段允许")
        );
        assert!(
            validate_quote("claim_3", &quote, &["Original text."])
                .unwrap_err()
                .contains("ID \"ev-page-7\" 有效")
        );
        let exact = serde_json::json!({"evidenceItemId":"ev-page-7","quote":"Original"});
        assert!(validate_quote("claim_3", &exact, &["Original text."]).is_ok());
        let long = "x".repeat(10000);
        let quote = serde_json::json!({"evidenceItemId":"ev-page-7","quote":long});
        let error = validate_quote("claim_3", &quote, &["y".repeat(10000).as_str()]).unwrap_err();
        assert!(error.contains("已截断"));
        assert!(error.chars().count() < 1000);
        assert!(!error.contains(&long));
    }
}
