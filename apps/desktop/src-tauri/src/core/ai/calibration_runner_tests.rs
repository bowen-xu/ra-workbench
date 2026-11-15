use super::*;

fn scenario() -> Scenario {
    scenario_with_id("case-1")
}

fn scenario_with_id(id: &str) -> Scenario {
    Scenario {
        id: id.to_owned(),
        language: "en".to_owned(),
        category: "selection_explanation".to_owned(),
        source: "Calibration compares predicted probabilities with observed frequencies."
            .to_owned(),
        query: "Explain calibration.".to_owned(),
        _expected: serde_json::json!({ "requiredSignals": ["hidden expected signal"] }),
    }
}

#[test]
fn case_selector_runs_exactly_one_known_case_and_rejects_unknown_before_provider() {
    let fixture = ScenarioFixture {
        fixture_version: 1,
        license: "synthetic".to_owned(),
        privacy: "synthetic".to_owned(),
        annotation_status: "pending_human_review".to_owned(),
        cases: vec![
            scenario_with_id("selection-zh"),
            scenario_with_id("selection-en"),
        ],
    };
    let selected = select_scenarios(&fixture, Some("selection-zh")).expect("known case");
    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].id, "selection-zh");
    assert!(select_scenarios(&fixture, Some("missing-case")).is_err());
    assert_eq!(select_scenarios(&fixture, None).expect("full set").len(), 2);
}

fn valid_response() -> serde_json::Value {
    serde_json::json!({
        "kind": "calibration_case_v2",
        "schemaVersion": 2,
        "observedAnswerability": "answerable",
        "statements": [{
            "text": "Calibration compares predictions and outcomes.",
            "isSourceStatement": true,
            "evidenceIds": ["source-01"]
        }]
    })
}

#[test]
fn prompt_stays_blind_to_expected_labels() {
    let messages = calibration_messages(&scenario());
    let prompt = messages
        .iter()
        .map(|message| message.content.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(prompt.contains("Explain calibration."));
    assert!(prompt.contains("source-01"));
    assert!(prompt.contains("Cite only provided evidence IDs"));
    assert!(!prompt.contains("evidenceQuotes"));
    assert!(!prompt.contains("hidden expected signal"));
}

#[test]
fn response_decoder_maps_known_evidence_ids_to_verbatim_source_and_rejects_unknown_fields() {
    let valid = valid_response();
    let run_case = build_run_case(
        &scenario(),
        &valid.to_string(),
        15,
        &ProviderUsage::default(),
        None,
    )
    .expect("decode valid calibration response");
    assert_eq!(
        run_case.statements[0].evidence[0].verifier_support,
        "not_run"
    );
    assert_eq!(
        run_case.statements[0].evidence[0].quote,
        "Calibration compares predicted probabilities with observed frequencies."
    );

    let mut unsupported = valid.clone();
    unsupported["statements"][0]["evidenceIds"] = serde_json::json!(["source-99"]);
    assert!(
        build_run_case(
            &scenario(),
            &unsupported.to_string(),
            15,
            &ProviderUsage::default(),
            None,
        )
        .is_err()
    );

    let mut duplicate = valid.clone();
    duplicate["statements"][0]["evidenceIds"] = serde_json::json!(["source-01", "source-01"]);
    assert!(
        build_run_case(
            &scenario(),
            &duplicate.to_string(),
            15,
            &ProviderUsage::default(),
            None,
        )
        .is_err()
    );

    let mut unknown = valid;
    unknown["rawReasoning"] = serde_json::json!("must be rejected");
    assert!(
        build_run_case(
            &scenario(),
            &unknown.to_string(),
            15,
            &ProviderUsage::default(),
            None,
        )
        .is_err()
    );
}

#[test]
fn frozen_v2_fixture_has_a_bounded_exact_evidence_inventory() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../tests/fixtures/ai/cases/m0-cases-v2.json");

    let fixture = read_fixture(&path).expect("read the frozen v2 fixture");

    assert_eq!(fixture.cases.len(), 48);
    for scenario in &fixture.cases {
        let evidence = evidence_items(scenario);
        assert!(!evidence.is_empty());
        assert!(evidence.len() <= 64);
        assert!(evidence.iter().all(|item| {
            !item.quote.is_empty()
                && item.quote.chars().count() <= 1_500
                && scenario.source.contains(&item.quote)
        }));
    }
}

#[test]
fn evidence_items_split_sentences_without_splitting_decimal_values() {
    let mut input = scenario();
    input.source = "The threshold is 0.4. 第二句说明证据；Third sentence!".to_owned();

    let evidence = evidence_items(&input);

    assert_eq!(evidence.len(), 3);
    assert_eq!(evidence[0].id, "source-01");
    assert_eq!(evidence[0].quote, "The threshold is 0.4.");
    assert_eq!(evidence[1].quote, "第二句说明证据；");
    assert_eq!(evidence[2].quote, "Third sentence!");
    assert!(
        evidence
            .iter()
            .all(|item| input.source.contains(&item.quote))
    );
}

#[test]
fn pricing_is_explicit_overflow_checked_and_rounded_up_to_microusd() {
    let usage = ProviderUsage {
        input_tokens: Some(30),
        output_tokens: Some(10),
        cache_hit_tokens: Some(20),
        cache_miss_tokens: Some(10),
    };
    let pricing = CalibrationPricing {
        input_microusd_per_million: 100_000,
        cache_hit_microusd_per_million: 10_000,
        cache_miss_microusd_per_million: 100_000,
        output_microusd_per_million: 200_000,
        cost_basis: "synthetic pricing snapshot".to_owned(),
    };
    let measured = measured_usage(&usage, Some(&pricing));
    assert_eq!(measured.estimated_cost_microusd, Some(4));
    assert_eq!(
        measured.cost_basis.as_deref(),
        Some("synthetic pricing snapshot")
    );

    let missing = measured_usage(&ProviderUsage::default(), Some(&pricing));
    assert_eq!(missing.estimated_cost_microusd, None);
    assert_eq!(missing.cost_basis, None);
}
