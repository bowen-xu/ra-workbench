use super::*;
use crate::structured_resources::ReviewPolicy;
use serde_json::json;

fn evidence() -> AiEvidenceBundle {
    AiEvidenceBundle {
        id: String::new(),
        run_id: String::new(),
        scope_hash: String::new(),
        retrieval_config_version: String::new(),
        validation_status: String::new(),
        created_at: String::new(),
        items: vec![],
    }
}
fn origin(sequence: usize) -> DecisionOrigin {
    DecisionOrigin {
        package_key: "a".repeat(64),
        instance_id: "native-instance".into(),
        native_request_id: format!("native:{sequence}"),
    }
}
fn need(policy: &mut PrefixPolicy, value: Value) -> (usize, StructuredValidationInputV1) {
    match policy.validate("portrait", value, &evidence()) {
        Err(Flow::Need(HbsNeed::Structured { slot, input })) => (slot, input),
        _ => panic!("first unmatched slot must request its own exact input"),
    }
}

#[test]
fn prefix_replay_progresses_beyond_checkpoint_length_without_lru_or_slot_limit() {
    let mut policy = PrefixPolicy::default();
    for new_slot in 0..128 {
        policy.restart();
        for old_slot in 0..new_slot {
            assert_eq!(
                policy
                    .validate("portrait", json!({"slot":old_slot}), &evidence())
                    .unwrap(),
                json!({"canonical":old_slot})
            );
        }
        let (slot, input) = need(&mut policy, json!({"slot":new_slot}));
        assert_eq!(slot, new_slot);
        assert!(policy.store(slot, input, Ok(json!({"canonical":slot})), origin(slot)));
    }
    assert_eq!(policy.slots.len(), 128);
}

#[test]
fn first_changed_input_discards_only_the_suffix_and_preserves_rejected_origin() {
    let mut policy = PrefixPolicy::default();
    let (slot, input) = need(&mut policy, json!({"number":-0.0}));
    assert!(policy.store(
        slot,
        input,
        Err(CandidateValidationError::Validation {
            path: "$.fields[0]".into(),
            message: "original rejection".into()
        }),
        origin(7)
    ));
    policy.restart();
    match policy
        .validate("portrait", json!({"number":-0.0}), &evidence())
        .unwrap_err()
    {
        Flow::Fail(Failure {
            error: AiError::Validation { path, message },
            cause: FailureCause::NativeRejection(actual),
        }) => {
            assert_eq!(path, "$.fields[0]");
            assert_eq!(message, "original rejection");
            assert_eq!(actual.native_request_id, "native:7");
        }
        _ => panic!("same original error and matched request identity"),
    }
    policy.restart();
    let (slot, _) = need(&mut policy, json!({"number":0.0}));
    assert_eq!(slot, 0);
    assert!(policy.slots.is_empty());
}

#[test]
fn sources_requirement_is_a_real_pause_without_a_fake_full_provider() {
    let mut sources = RuntimeHbsSources::default();
    assert!(matches!(
        sources.require(),
        Err(Flow::Need(HbsNeed::Sources))
    ));
}

#[test]
fn editable_template_uses_its_own_exact_slot_and_keeps_the_rejected_origin() {
    let mut sources = RuntimeHbsSources::default();
    let original = json!({"kind":"reading_template_v1","name":"Original","fields":[]});
    let policy = sources.policy_only().unwrap();
    let (slot, input) = match policy.validate_editable_template(original.clone()) {
        Err(Flow::Need(HbsNeed::EditableTemplate { slot, input })) => (slot, input),
        _ => panic!("template has no reading-evidence dependency"),
    };
    assert_eq!(input.candidate_json, original.to_string());
    assert!(policy.store_template(
        slot,
        input,
        Err("模板类型、名称或字段数量无效".into()),
        origin(17)
    ));
    policy.restart();
    match policy
        .validate_editable_template(original.clone())
        .unwrap_err()
    {
        Flow::Fail(Failure {
            error: AiError::InvalidInput(message),
            cause: FailureCause::NativeRejection(actual),
        }) => {
            assert_eq!(message, "模板类型、名称或字段数量无效");
            assert_eq!(actual.native_request_id, "native:17");
        }
        _ => panic!("same normal issue and exact Native request identity"),
    }
    let (slot, input) = need(policy, portrait_for_prefix());
    assert_eq!(slot, 1);
    assert!(policy.store(slot, input, Ok(Value::Null), origin(18)));
    policy.restart();
    let mut changed = original;
    changed["name"] = json!("Changed");
    assert!(matches!(
        policy.validate_editable_template(changed),
        Err(Flow::Need(HbsNeed::EditableTemplate { slot: 0, .. }))
    ));
    assert!(policy.slots.is_empty());
    assert!(sources.reading.is_none());
}
fn portrait_for_prefix() -> Value {
    json!({"coverage":"subsequent distinct operation"})
}

fn quote(text: &str) -> crate::ai::candidate_contracts::AiEvidenceItem {
    crate::ai::candidate_contracts::AiEvidenceItem {
        id: String::new(),
        citation_key: "duplicate".into(),
        ordinal: 0,
        anchor_id: String::new(),
        paper_id: String::new(),
        paper_title: String::new(),
        attachment_id: String::new(),
        content_sha256: String::new(),
        page_index: 0,
        page_label: None,
        quote_exact: text.into(),
        quote_prefix: String::new(),
        quote_suffix: String::new(),
        context_text: String::new(),
        relation_kind: String::new(),
        validation_status: String::new(),
        current_anchor_status: String::new(),
        created_at: String::new(),
    }
}
#[test]
fn full_input_identity_keeps_duplicate_quotes_and_their_original_order() {
    let mut policy = PrefixPolicy::default();
    let mut bundle = evidence();
    bundle.items = vec![quote("original first"), quote("original second")];
    let (slot, input) = match policy
        .validate("portrait", json!({"same":"candidate"}), &bundle)
        .unwrap_err()
    {
        Flow::Need(HbsNeed::Structured { slot, input }) => (slot, input),
        _ => panic!("first exact ordered input"),
    };
    assert_eq!(input.evidence.len(), 2);
    assert_eq!(
        input.evidence[0].citation_key,
        input.evidence[1].citation_key
    );
    assert_eq!(input.evidence[0].quote_exact, "original first");
    assert!(policy.store(slot, input, Ok(Value::Null), origin(1)));
    policy.restart();
    bundle.items.reverse();
    assert!(matches!(
        policy.validate("portrait", json!({"same":"candidate"}), &bundle),
        Err(Flow::Need(HbsNeed::Structured { slot: 0, .. }))
    ));
    assert!(policy.slots.is_empty());
}

#[test]
fn selection_and_validation_share_one_typed_prefix_without_loading_unused_sources() {
    let mut sources = RuntimeHbsSources::default();
    let selection = UnreviewedSelectionInputV1 {
        resource_id: "resource-1".into(),
        resource_revision: 7,
        pending_field_ids: vec!["unreviewed".into(), "protected".into(), "reviewed".into()],
        protected_field_ids: vec!["protected".into()],
        reviewed_field_ids: vec!["reviewed".into()],
    };
    let policy = sources.policy_only().unwrap();
    match policy.select_unreviewed(selection.clone()) {
        Err(HbsNeed::Unreviewed { slot: 0, input }) => assert_eq!(input, selection),
        _ => panic!("first selection needs its exact input"),
    }
    assert!(policy.store_selection(0, selection.clone(), vec!["unreviewed".into()]));
    policy.restart();
    assert_eq!(
        policy.select_unreviewed(selection.clone()).unwrap(),
        vec!["unreviewed"]
    );
    let (slot, input) = need(policy, json!({"next":"validation"}));
    assert_eq!(slot, 1);
    assert!(policy.store(slot, input, Ok(Value::Null), origin(9)));
    policy.restart();
    assert_eq!(
        policy.select_unreviewed(selection.clone()).unwrap(),
        vec!["unreviewed"]
    );
    assert!(
        policy
            .validate("portrait", json!({"next":"validation"}), &evidence())
            .unwrap()
            .is_null()
    );
    policy.restart();
    let mut changed = selection;
    changed.resource_revision += 1;
    assert!(matches!(
        policy.select_unreviewed(changed),
        Err(HbsNeed::Unreviewed { slot: 0, .. })
    ));
    assert!(policy.slots.is_empty());
    assert!(sources.reading.is_none());
    assert!(matches!(
        sources.require(),
        Err(Flow::Need(HbsNeed::Sources))
    ));
}
