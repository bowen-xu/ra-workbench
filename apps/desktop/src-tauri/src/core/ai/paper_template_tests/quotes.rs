use research_literature_library_native::{
    analysis_workflow::candidate::decode_candidate_value,
    structured_validation::quotes as validate_quotes,
};
use research_workbench_native_sdk::candidate_contracts::EvidenceQuote;
use serde_json::json;
use std::collections::HashSet;

#[test]
fn uncertainty_can_have_no_citations_but_factual_roles_still_require_evidence() {
    let payload = json!({"kind":"paper_overview_unit_v1","schemaVersion":1,"profileId":"paper_template_v1",
        "paperTemplate":{"templateResourceId":"template","templateResourceRevision":1,"fields":[{"id":"question","title":"问题","question":"研究什么？","claimIds":[]}]},
        "fields":[{"field":"question","status":"filled","statements":[{"id":"s1","text":"原文未能确定。","epistemicRole":"uncertain","evidenceItemIds":[]}]}]});
    decode_candidate_value(payload.clone(), &HashSet::new()).unwrap();
    for role in ["source_paraphrase", "inference", "synthesis"] {
        let mut factual = payload.clone();
        factual["fields"][0]["statements"][0]["epistemicRole"] = role.into();
        assert!(decode_candidate_value(factual, &HashSet::new()).is_err());
    }
    let mut cited = payload;
    cited["fields"][0]["statements"][0]["evidenceItemIds"] = json!(["source"]);
    assert!(decode_candidate_value(cited, &HashSet::from(["source".into()])).is_err());
}

#[test]
fn complete_frozen_paragraphs_are_valid_without_a_1200_character_cutoff() {
    let key = "ev-page-1".to_owned();
    let allowed = HashSet::from([key.clone()]);
    let mut bundle = vec![EvidenceQuote {
        citation_key: key.clone(),
        quote_exact: "An explicit model.".into(),
    }];
    // Docling freezes complete structural blocks, unlike legacy page snippets.
    for quote in ["x".repeat(1224), "原文🙂".repeat(2000)] {
        bundle[0].quote_exact = format!("Prefix. {quote} Suffix.");
        let payload = json!({"kind":"paper_overview_unit_v1","schemaVersion":1,"profileId":"paper_template_v1",
            "paperTemplate":{"portraitCandidateId":"portrait","portraitRevision":1,"fields":[{"id":"question","title":"问题","question":"研究什么？","claimIds":[]}]},
            "fields":[{"field":"question","status":"filled","statements":[{"id":"s1","text":"A supported statement.","epistemicRole":"source_paraphrase","evidenceItemIds":[key],"evidenceQuotes":[{"evidenceItemId":key,"quote":quote}]}]}]});
        let decoded = decode_candidate_value(payload.clone(), &allowed).unwrap();
        validate_quotes(&decoded, &bundle).unwrap();
        let mut forged = payload.clone();
        forged["fields"][0]["statements"][0]["evidenceQuotes"][0]["quote"] =
            format!("{quote} invented").into();
        let decoded = decode_candidate_value(forged, &allowed).unwrap();
        let error = validate_quotes(&decoded, &bundle).unwrap_err().to_string();
        assert!(error.contains("s1") && error.contains(&key));
        for replacement in [
            json!([]),
            json!([{"evidenceItemId":key,"quote":" "}]),
            json!([{"evidenceItemId":key,"quote":"x"},{"evidenceItemId":key,"quote":"x"}]),
        ] {
            let mut invalid = payload.clone();
            invalid["fields"][0]["statements"][0]["evidenceQuotes"] = replacement;
            assert!(decode_candidate_value(invalid, &allowed).is_err());
        }
    }
}
