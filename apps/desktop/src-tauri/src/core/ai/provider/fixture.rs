use std::time::Duration;

use super::{
    GenerationProvider,
    cancellation::CancellationSignal,
    model::{
        GenerationRequest, GenerationResponse, ProviderCapabilities, ProviderError,
        ProviderErrorKind, ProviderUsage,
    },
};

#[derive(Clone)]
#[cfg(test)]
pub(crate) enum FixtureBehavior {
    Delayed { content: String, delay: Duration },
    Failure(ProviderErrorKind),
}

#[derive(Clone)]
pub(crate) struct FixtureGenerationProvider {
    content: String,
    delay: Option<Duration>,
    failure: Option<ProviderErrorKind>,
}

impl Default for FixtureGenerationProvider {
    fn default() -> Self {
        Self {
            content: r#"{"kind":"provider_diagnostic_v1","ok":true}"#.to_owned(),
            delay: None,
            failure: None,
        }
    }
}

impl FixtureGenerationProvider {
    #[cfg(test)]
    pub(crate) fn with_behavior(behavior: FixtureBehavior) -> Self {
        match behavior {
            FixtureBehavior::Delayed { content, delay } => Self {
                content,
                delay: Some(delay),
                failure: None,
            },
            FixtureBehavior::Failure(kind) => Self {
                content: String::new(),
                delay: None,
                failure: Some(kind),
            },
        }
    }
}

impl GenerationProvider for FixtureGenerationProvider {
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            provider_id: "fixture-generation".to_owned(),
            provider_kind: "generation".to_owned(),
            is_cloud: false,
            supports_structured_output: true,
            supports_streaming: false,
            supports_cancellation: true,
            supports_usage: true,
            supports_vision_input: false,
        }
    }

    async fn generate(
        &self,
        request: &GenerationRequest,
        cancellation: &CancellationSignal,
    ) -> Result<GenerationResponse, ProviderError> {
        if cancellation.is_cancelled() {
            return Err(ProviderError::new(ProviderErrorKind::Canceled));
        }
        if let Some(kind) = self.failure {
            return Err(ProviderError::new(kind));
        }
        if let Some(delay) = self.delay {
            cancellation
                .race(tokio::time::sleep(delay))
                .await
                .map_err(|_| ProviderError::new(ProviderErrorKind::Canceled))?;
        }
        let is_selection = request
            .messages
            .iter()
            .any(|message| message.content.contains("selection_explanation_v1"));
        let is_overview = request
            .messages
            .iter()
            .any(|message| message.content.contains("paper_overview_unit_v1"));
        let suggestion = suggestion_content(request);
        let research = research_candidate_content(request);
        let content = if let Some(content) = research
            && self.content.contains("provider_diagnostic_v1")
        {
            content
        } else if let Some(content) = suggestion
            && self.content.contains("provider_diagnostic_v1")
        {
            content
        } else if is_overview && self.content.contains("provider_diagnostic_v1") {
            profiled_overview_content(request).unwrap_or_else(|| {
            r#"{"kind":"paper_overview_unit_v1","schemaVersion":1,"profileId":"generic_overview_v1","fields":[{"field":"research_question","status":"filled","statements":[{"id":"q1","text":"论文提出了一个需要从原文核对的研究问题。","epistemicRole":"source_paraphrase","evidenceItemIds":["ev-page-1"]}]},{"field":"approach","status":"filled","statements":[{"id":"a1","text":"论文描述了用于回答研究问题的方法。","epistemicRole":"source_paraphrase","evidenceItemIds":["ev-page-1"]}]},{"field":"key_result","status":"partial","statements":[{"id":"r1","text":"有限证据片段报告了主要结果。","epistemicRole":"source_paraphrase","evidenceItemIds":["ev-page-1"]}]},{"field":"conclusion","status":"partial","statements":[{"id":"c1","text":"作者据此给出了受当前证据范围限制的结论。","epistemicRole":"source_paraphrase","evidenceItemIds":["ev-page-1"]}]},{"field":"limitation","status":"not_found","statements":[]},{"field":"open_question","status":"not_found","statements":[]}]}"#.to_owned()
            })
        } else if is_selection && self.content.contains("provider_diagnostic_v1") {
            r#"{"kind":"selection_explanation_v1","schemaVersion":1,"statements":[{"id":"s1","text":"这段选区的核心含义由精确引文直接支持。","epistemicRole":"source_paraphrase","evidenceItemIds":["ev-quote"]},{"id":"s2","text":"结合选区上下文，可以形成一条仍需人工核对的综合解释。","epistemicRole":"synthesis","evidenceItemIds":["ev-quote","ev-context"]}],"uncertainties":["该局部选区可能不足以覆盖作者在全文中的限定条件。"]}"#.to_owned()
        } else {
            self.content.clone()
        };
        Ok(GenerationResponse {
            model_id: request.model_id.clone(),
            content,
            usage: ProviderUsage {
                input_tokens: Some(12),
                output_tokens: Some(8),
                cache_hit_tokens: Some(0),
                cache_miss_tokens: Some(12),
            },
        })
    }
}

fn research_candidate_content(request: &GenerationRequest) -> Option<String> {
    let prompt = request.messages.iter().find_map(|message| {
        let value = serde_json::from_str::<serde_json::Value>(&message.content).ok()?;
        matches!(
            value.get("task")?.as_str()?,
            "reference_record_candidate_v1" | "derived_research_view_v1"
        )
        .then_some(value)
    })?;
    let task = prompt.get("task")?.as_str()?;
    let evidence = prompt.get("evidence")?.as_array()?;
    let first = evidence.first()?;
    if task == "reference_record_candidate_v1" {
        let raw_text = first.get("quoteExact")?.as_str()?;
        let citation_key = first.get("citationKey")?.as_str()?;
        return serde_json::to_string(&serde_json::json!({
            "kind": task,
            "schemaVersion": 1,
            "rawText": raw_text,
            "evidenceItemIds": [citation_key],
            "fields": [{
                "name": "title",
                "value": raw_text,
                "evidenceItemIds": [citation_key]
            }],
            "uncertainties": ["Fixture cannot verify bibliographic identity or DOI."]
        }))
        .ok();
    }
    let first_key = first.get("citationKey")?.as_str()?;
    let second_key = evidence.get(1)?.get("citationKey")?.as_str()?;
    let view_kind = prompt.get("viewKind")?.as_str()?;
    let is_hypothesis = matches!(view_kind, "gap_hypothesis" | "innovation_hypothesis");
    serde_json::to_string(&serde_json::json!({
        "kind": task,
        "schemaVersion": 1,
        "viewKind": view_kind,
        "title": prompt.get("title")?.as_str()?,
        "scopeNote": prompt.get("scopeNote")?.as_str()?,
        "items": [{
            "id": "derived_1",
            "text": if is_hypothesis {
                "所选证据提示一个仍需扩大检索和人工核验的工作假设。"
            } else {
                "所选来源形成一条需要人工核对的跨文献综合。"
            },
            "epistemicRole": if is_hypothesis { "hypothesis" } else { "synthesis" },
            "evidenceItemIds": if is_hypothesis {
                vec![first_key]
            } else {
                vec![first_key, second_key]
            },
            "hasConflict": false
        }]
    }))
    .ok()
}

fn suggestion_content(request: &GenerationRequest) -> Option<String> {
    let task = request.messages.iter().find_map(|message| {
        serde_json::from_str::<serde_json::Value>(&message.content)
            .ok()?
            .get("task")?
            .as_str()
            .map(str::to_owned)
    })?;
    let suggestion = match task.as_str() {
        "concept_suggestion_v1" => serde_json::json!({
            "id": "concept_1", "term": "稀疏门控",
            "explanation": "证据描述了每次只激活部分专家的路由机制。",
            "disambiguation": "混合专家模型中的路由机制", "domain": "机器学习",
            "epistemicRole": "source_paraphrase", "evidenceItemIds": ["ev-page-1"]
        }),
        "definition_suggestion_v1" => serde_json::json!({
            "id": "definition_1", "term": "稀疏门控",
            "statement": "稀疏门控在一次计算中选择部分专家。",
            "epistemicRole": "source_paraphrase", "evidenceItemIds": ["ev-page-1"]
        }),
        "relation_suggestion_v1" => serde_json::json!({
            "id": "relation_1", "subjectTerm": "稀疏门控",
            "predicateKind": "related_to", "objectTerm": "混合专家",
            "epistemicRole": "source_paraphrase", "evidenceItemIds": ["ev-page-1"]
        }),
        "attention_suggestion_v1" => serde_json::json!({
            "id": "attention_1", "title": "关注路由与负载均衡",
            "reason": "该约束可能影响专家利用率，值得回到原文核对。", "priority": "high",
            "evidenceItemIds": ["ev-page-1"]
        }),
        _ => return None,
    };
    serde_json::to_string(&serde_json::json!({
        "kind": task,
        "schemaVersion": 1,
        "suggestions": [suggestion]
    }))
    .ok()
}

fn profiled_overview_content(request: &GenerationRequest) -> Option<String> {
    let prompt = request.messages.iter().find_map(|message| {
        let value = serde_json::from_str::<serde_json::Value>(&message.content).ok()?;
        (value.get("profileId")?.as_str()? == "reviewed_profile_composition_v1").then_some(value)
    })?;
    let profile_refs = prompt.get("profileRefs")?.clone();
    let fields = prompt
        .get("requiredFields")?
        .as_array()?
        .iter()
        .enumerate()
        .map(|(index, field)| {
            let field = field.as_str()?;
            Some(if index == 0 {
                serde_json::json!({
                    "field": field,
                    "status": "partial",
                    "statements": [{
                        "id": format!("profile_field_{index}"),
                        "text": "当前冻结证据为该画像字段提供了需要人工核对的线索。",
                        "epistemicRole": "source_paraphrase",
                        "evidenceItemIds": ["ev-page-1"]
                    }]
                })
            } else {
                serde_json::json!({ "field": field, "status": "not_found", "statements": [] })
            })
        })
        .collect::<Option<Vec<_>>>()?;
    serde_json::to_string(&serde_json::json!({
        "kind": "paper_overview_unit_v1",
        "schemaVersion": 1,
        "profileId": "reviewed_profile_composition_v1",
        "profileRefs": profile_refs,
        "fields": fields
    }))
    .ok()
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::super::{
        GenerationMessage, GenerationRequest, GenerationResponseFormat, GenerationRole,
        ThinkingMode,
    };
    use super::profiled_overview_content;

    #[test]
    fn fixture_returns_the_frozen_profile_refs_and_fields() {
        let request = GenerationRequest {
            model_id: "fixture-v1".to_owned(),
            messages: vec![GenerationMessage {
                role: GenerationRole::User,
                content: serde_json::json!({
                    "profileId": "reviewed_profile_composition_v1",
                    "profileRefs": [{
                        "profileId": "philosophical_conceptual",
                        "profileVersion": 1
                    }],
                    "requiredFields": ["research_question", "argument"]
                })
                .to_string(),
            }],
            max_output_tokens: 1_000,
            thinking_mode: ThinkingMode::Disabled,
            response_format: GenerationResponseFormat::JsonObject,
            timeout: Duration::from_secs(5),
        };
        let value: serde_json::Value = serde_json::from_str(
            &profiled_overview_content(&request).expect("profile-aware fixture response"),
        )
        .expect("fixture JSON");
        assert_eq!(value["profileRefs"][0]["profileVersion"], 1);
        assert_eq!(value["fields"][1]["field"], "argument");
    }
}
