use std::collections::HashSet;

use research_literature_library_native::analysis_workflow::candidate::decode_candidate_value;

fn evidence() -> HashSet<String> {
    ["ev-1".to_owned(), "ev-2".to_owned()].into_iter().collect()
}

#[test]
fn decodes_the_four_grounded_suggestion_kinds() {
    let cases = [
        serde_json::json!({
            "kind": "concept_suggestion_v1",
            "schemaVersion": 1,
            "suggestions": [{
                "id": "concept_1",
                "term": "稀疏门控",
                "language": "zh",
                "explanation": "只激活一部分专家。",
                "domain": "机器学习",
                "epistemicRole": "source_paraphrase",
                "evidenceItemIds": ["ev-1"]
            }]
        }),
        serde_json::json!({
            "kind": "definition_suggestion_v1",
            "schemaVersion": 1,
            "suggestions": [{
                "id": "definition_1",
                "term": "专家",
                "statement": "专家是被路由器选择的子网络。",
                "epistemicRole": "source_paraphrase",
                "evidenceItemIds": ["ev-1"]
            }]
        }),
        serde_json::json!({
            "kind": "relation_suggestion_v1",
            "schemaVersion": 1,
            "suggestions": [{
                "id": "relation_1",
                "subjectTerm": "稀疏门控",
                "predicateKind": "related_to",
                "objectTerm": "混合专家",
                "epistemicRole": "synthesis",
                "evidenceItemIds": ["ev-1", "ev-2"]
            }]
        }),
        serde_json::json!({
            "kind": "attention_suggestion_v1",
            "schemaVersion": 1,
            "suggestions": [{
                "id": "attention_1",
                "title": "关注负载均衡约束",
                "reason": "它限定了专家是否会过度集中。",
                "priority": "high",
                "evidenceItemIds": ["ev-2"]
            }]
        }),
    ];

    let kinds = cases
        .into_iter()
        .map(|value| {
            decode_candidate_value(value, &evidence())
                .expect("valid suggestion candidate")
                .kind()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        kinds,
        [
            "concept_suggestion_v1",
            "definition_suggestion_v1",
            "relation_suggestion_v1",
            "attention_suggestion_v1",
        ]
    );
}

#[test]
fn rejects_model_owned_identity_unknown_relations_and_out_of_scope_evidence() {
    let concept_with_identity = serde_json::json!({
        "kind": "concept_suggestion_v1",
        "schemaVersion": 1,
        "suggestions": [{
            "id": "concept_1",
            "conceptId": "invented-stable-id",
            "term": "稀疏门控",
            "explanation": "解释",
            "epistemicRole": "source_paraphrase",
            "evidenceItemIds": ["ev-1"]
        }]
    });
    assert!(decode_candidate_value(concept_with_identity, &evidence()).is_err());

    let relation = serde_json::json!({
        "kind": "relation_suggestion_v1",
        "schemaVersion": 1,
        "suggestions": [{
            "id": "relation_1",
            "subjectTerm": "A",
            "predicateKind": "proves",
            "objectTerm": "B",
            "epistemicRole": "source_paraphrase",
            "evidenceItemIds": ["ev-1"]
        }]
    });
    assert!(decode_candidate_value(relation, &evidence()).is_err());

    let attention = serde_json::json!({
        "kind": "attention_suggestion_v1",
        "schemaVersion": 1,
        "suggestions": [{
            "id": "attention_1",
            "title": "关注某处",
            "reason": "理由",
            "priority": "low",
            "evidenceItemIds": ["outside"]
        }]
    });
    assert!(decode_candidate_value(attention, &evidence()).is_err());
}

#[test]
fn synthesis_requires_multiple_evidence_items() {
    let definition = serde_json::json!({
        "kind": "definition_suggestion_v1",
        "schemaVersion": 1,
        "suggestions": [{
            "id": "definition_1",
            "term": "综合定义",
            "statement": "跨片段综合得出的定义。",
            "epistemicRole": "synthesis",
            "evidenceItemIds": ["ev-1"]
        }]
    });
    assert!(matches!(
        decode_candidate_value(
            definition,
            &evidence()
        ),
        Err(research_workbench_native_sdk::candidate_contracts::CandidateValidationError::InvalidInput(_))
    ));
}
