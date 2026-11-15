use super::*;
#[path = "tests/authority.rs"]
mod authority;
#[path = "tests/phases.rs"]
mod phases;

#[test]
fn unknown_authoritative_output_is_not_reported_as_a_request_capability() {
    for item in [
        json!({"type":"computer_call","id":"unapproved"}),
        json!({"type":"message","role":"assistant","content":[{"type":"output_audio"}]}),
    ] {
        assert_eq!(
            project_output(&[item]),
            Err(ModelError::UnsupportedResponseType)
        );
    }
}

#[test]
fn empty_window_is_bound_and_still_requires_its_account_and_model() {
    let account = Uuid::from_u128(1);
    let state = ResponsesContinuation::new(Uuid::new_v4(), model(), 16384).unwrap();
    state.bind_window_sync(account, &model(), &[]).unwrap();
    assert_eq!(
        state.prepare_sync(account, model(), input()).unwrap(),
        input()
    );
    assert_eq!(
        state.bind_window_sync(Uuid::from_u128(2), &model(), &[]),
        Err(ModelError::Authentication)
    );
    let mut other = model();
    other.model_id = "different-model".into();
    assert_eq!(
        state.bind_window_sync(account, &other, &[]),
        Err(ModelError::UnsupportedCapability)
    );
}

#[tokio::test]
async fn text_candidate_shrinking_keeps_regular_replay_strict_and_account_bound() {
    let state = ResponsesContinuation::new(Uuid::new_v4(), model(), 16384).unwrap();
    let account = Uuid::from_u128(1);
    state.prepare_sync(account, model(), input()).unwrap();
    let output = vec![
        json!({"type":"message","role":"assistant","content":[{"type":"output_text","text":"done"}]}),
    ];
    state
        .preserve(receipt(input(), input(), output))
        .await
        .unwrap();
    let before = state.encode_checkpoint().unwrap();
    let reduced = vec![json!({"role":"user","content":"handoff"})];
    assert_eq!(
        state
            .prepare_text_compaction_input(account, model(), reduced.clone())
            .await
            .unwrap(),
        reduced
    );
    assert_eq!(
        state.prepare_sync(account, model(), reduced.clone()),
        Err(ModelError::InvalidRequest)
    );
    assert_eq!(
        state.prepare_text_sync(Uuid::from_u128(2), model(), reduced.clone()),
        Err(ModelError::Authentication)
    );
    let mut other = model();
    other.model_id = "other".into();
    assert_eq!(
        state.prepare_text_sync(account, other, reduced),
        Err(ModelError::InvalidRequest)
    );
    assert_eq!(state.encode_checkpoint().unwrap(), before);
}

#[tokio::test]
async fn inherited_turn_rebinds_only_host_authority_and_preserves_provider_items() {
    let account = Uuid::from_u128(1);
    let state = ResponsesContinuation::new(Uuid::new_v4(), model(), 16384).unwrap();
    let authority = json!({"role":"developer","content":"old authority"});
    // A full provider window may itself begin with a developer item. Its content
    // is opaque to the replay owner and must not be removed with host authority.
    let window = vec![
        json!({"role":"developer","content":"provider window item"}),
        json!({"type":"compaction","encrypted_content":"opaque fixture"}),
    ];
    state.bind_window_sync(account, &model(), &window).unwrap();
    let base = [vec![authority.clone()], input()].concat();
    let wire = state.prepare_sync(account, model(), base.clone()).unwrap();
    let output = vec![
        json!({"type":"message","role":"assistant","content":[{"type":"output_text","text":"done"}]}),
    ];
    state
        .preserve_sync(receipt(base.clone(), wire, output.clone()))
        .unwrap();
    let historical = [input(), project_output(&output).unwrap()].concat();
    let mut changed = vec![json!({"role":"developer","content":"new authority"})];
    changed.extend(historical.clone());
    changed.push(json!({"role":"user","content":"next"}));
    assert!(matches!(
        state.prepare_sync(account, model(), changed.clone()),
        Err(ModelError::InvalidRequest)
    ));
    let inherited = state
        .inherit_for_turn(Uuid::new_v4(), &std::collections::HashMap::new())
        .unwrap();
    let expanded = inherited
        .prepare_sync(account, model(), changed.clone())
        .unwrap();
    assert_eq!(expanded[0], changed[0]);
    assert_eq!(&expanded[1..3], window.as_slice());
    assert!(!expanded.contains(&authority));
    let mut edited_history = changed.clone();
    edited_history[1] = json!({"role":"user","content":"rewritten history"});
    assert!(matches!(
        inherited.prepare_sync(account, model(), edited_history),
        Err(ModelError::InvalidRequest)
    ));
    let mut two = vec![
        changed[0].clone(),
        json!({"role":"system","content":"second current rule"}),
    ];
    two.extend_from_slice(&changed[1..]);
    let expanded = inherited
        .prepare_sync(account, model(), two.clone())
        .unwrap();
    assert_eq!(&expanded[..2], &two[..2]);
    assert_eq!(&expanded[2..4], window.as_slice());
}

#[tokio::test]
async fn installed_compaction_requires_all_retained_user_inputs() {
    for retained in [
        vec![],
        vec![json!({"role":"assistant","content":"wrong prefix"})],
    ] {
        let state = ResponsesContinuation::new(Uuid::new_v4(), model(), 16384).unwrap();
        state
            .bind_compaction_sync(
                Uuid::from_u128(1),
                &model(),
                &json!({"type":"compaction","encrypted_content":"fixture"}),
                1,
            )
            .unwrap();
        let mut normalized = vec![json!({"role":"developer","content":"authority"})];
        normalized.extend(retained);
        assert!(matches!(
            state
                .prepare_input(Uuid::from_u128(1), model(), normalized)
                .await,
            Err(ModelError::InvalidRequest)
        ));
    }
}
fn model() -> ModelIdentity {
    ModelIdentity {
        provider_id: "fixture".into(),
        model_id: "model".into(),
        revision: None,
    }
}

#[tokio::test]
async fn full_window_binding_rejects_malformed_replacement_aliases_and_budget_overflow() {
    let account = Uuid::from_u128(1);
    let items = vec![json!({"type":"unknown_provider_memory","data":"retained"})];
    let state = ResponsesContinuation::new(Uuid::new_v4(), model(), 16384).unwrap();
    for malformed in [vec![json!({})], vec![json!("not an item")]] {
        assert_eq!(
            state.bind_window_sync(account, &model(), &malformed),
            Err(ModelError::InvalidStream)
        );
    }
    let tiny = ResponsesContinuation::new(Uuid::new_v4(), model(), 1).unwrap();
    assert_eq!(
        tiny.bind_window_sync(account, &model(), &items),
        Err(ModelError::BudgetExceeded)
    );
    state.bind_window_sync(account, &model(), &items).unwrap();
    let wire = state
        .prepare_input(account, model(), input())
        .await
        .unwrap();
    let mut received = receipt(
        input(),
        wire,
        vec![
            json!({"type":"message","role":"assistant","content":[{"type":"output_text","text":"done"}]}),
        ],
    );
    received.model.model_id = "alias".into();
    assert_eq!(
        state.preserve(received).await,
        Err(ModelError::UnsupportedCapability)
    );
    state.preserve(receipt(input(), [items.clone(), input()].concat(), vec![json!({"type":"message","role":"assistant","content":[{"type":"output_text","text":"done"}]})])).await.unwrap();
    assert_eq!(
        state.bind_window_sync(account, &model(), &[json!({"type":"changed_window"})]),
        Err(ModelError::InvalidStream)
    );
    assert_eq!(
        state.bind_window_sync(Uuid::from_u128(2), &model(), &items),
        Err(ModelError::Authentication)
    );
    state.bind_window_sync(account, &model(), &items).unwrap();
}
fn input() -> Vec<Value> {
    vec![json!({"role":"user","content":[{"type":"input_text","text":"read"}]})]
}
fn output() -> Vec<Value> {
    vec![
        json!({"type":"reasoning","id":"r1","encrypted_content":"opaque-fixture","summary":[]}),
        json!({"type":"function_call","id":"f1","call_id":"call1","name":"read","arguments":"{ \"path\": \"main.tex\" }"}),
    ]
}
fn receipt(input: Vec<Value>, wire: Vec<Value>, output: Vec<Value>) -> ResponsesReceipt {
    ResponsesReceipt {
        request_id: Uuid::new_v4(),
        credential_scope_id: Uuid::from_u128(1),
        requested_model: model(),
        model: model(),
        normalized_input: input,
        authority_items: None,
        input: wire,
        output,
    }
}

#[test]
fn bounded_tool_result_remains_a_strict_continuation_prefix_after_reopen() {
    use research_agent_core::model::tool_output::bounded_tool_output;
    let scope = Uuid::new_v4();
    let account = Uuid::from_u128(1);
    let state = ResponsesContinuation::new(scope, model(), 16384).unwrap();
    let base = input();
    state.prepare_sync(account, model(), base.clone()).unwrap();
    state
        .preserve_sync(receipt(base.clone(), base.clone(), output()))
        .unwrap();
    let full = "🦀result".repeat(100);
    let bounded = bounded_tool_output(&full, 64).unwrap();
    let mut next = base.clone();
    next.extend(project_output(&output()).unwrap());
    next.push(json!({"type":"function_call_output","call_id":"call1","output":bounded}));
    let wire = state.prepare_sync(account, model(), next.clone()).unwrap();
    assert_eq!(wire[1]["encrypted_content"], "opaque-fixture");
    assert_eq!(wire.last().unwrap()["output"], bounded);
    let answer = vec![
        json!({"type":"message","role":"assistant","content":[{"type":"output_text","text":"done"}]}),
    ];
    state
        .preserve_sync(receipt(next.clone(), wire.clone(), answer.clone()))
        .unwrap();
    let restored =
        ResponsesContinuation::restore(scope, model(), 16384, &state.encode_checkpoint().unwrap())
            .unwrap();
    next.extend(project_output(&answer).unwrap());
    next.push(json!({"role":"user","content":[{"type":"input_text","text":"next"}]}));
    let replay = restored
        .prepare_sync(account, model(), next.clone())
        .unwrap();
    assert_eq!(replay[1]["encrypted_content"], "opaque-fixture");
    assert_eq!(replay[3]["output"], bounded);

    // A different frozen budget cannot silently replace a legacy prefix.
    next[2]["output"] = json!(bounded_tool_output(&full, 128).unwrap());
    assert_eq!(
        restored.prepare_sync(account, model(), next),
        Err(ModelError::InvalidRequest)
    );
}
#[tokio::test]
async fn restores_opaque_items_and_original_arguments_in_next_tool_round() {
    let state = ResponsesContinuation::new(Uuid::new_v4(), model(), 16384).unwrap();
    state
        .prepare_input(Uuid::from_u128(1), model(), input())
        .await
        .unwrap();
    let base = input();
    assert_eq!(
        state
            .prepare_input(Uuid::from_u128(1), model(), base.clone())
            .await
            .unwrap(),
        base
    );
    state
        .preserve(receipt(base.clone(), base.clone(), output()))
        .await
        .unwrap();
    let mut normalized = base.clone();
    normalized.extend(project_output(&output()).unwrap());
    let result = json!({"type":"function_call_output","call_id":"call1","output":"source"});
    normalized.push(result.clone());
    let expanded = state
        .prepare_input(Uuid::from_u128(1), model(), normalized.clone())
        .await
        .unwrap();
    let mut expected = base;
    expected.extend(output());
    expected.push(result);
    assert_eq!(expanded, expected);
    let answer = vec![
        json!({"type":"message","id":"m1","role":"assistant","content":[{"type":"output_text","text":"Done"}]}),
    ];
    state
        .preserve(receipt(
            normalized.clone(),
            expanded.clone(),
            answer.clone(),
        ))
        .await
        .unwrap();
    normalized.extend(project_output(&answer).unwrap());
    normalized.push(json!({"role":"user","content":"again"}));
    let final_input = state
        .prepare_input(Uuid::from_u128(1), model(), normalized)
        .await
        .unwrap();
    assert_eq!(final_input[1]["encrypted_content"], "opaque-fixture");
    assert_eq!(final_input.len(), 6);
}
#[tokio::test]
async fn rejects_changed_history_model_and_stale_parallel_receipts() {
    let state = ResponsesContinuation::new(Uuid::new_v4(), model(), 16384).unwrap();
    state
        .prepare_input(Uuid::from_u128(1), model(), input())
        .await
        .unwrap();
    state
        .preserve(receipt(input(), input(), output()))
        .await
        .unwrap();
    assert!(
        state
            .prepare_input(Uuid::from_u128(1), model(), input())
            .await
            .is_err()
    );
    let mut other = model();
    other.model_id = "different".into();
    assert!(
        state
            .prepare_input(Uuid::from_u128(1), other, input())
            .await
            .is_err()
    );
    assert!(
        state
            .preserve(receipt(input(), input(), output()))
            .await
            .is_err()
    );
}
#[tokio::test]
async fn budget_failure_keeps_previous_checkpoint_and_owners_are_isolated() {
    let first = ResponsesContinuation::new(Uuid::new_v4(), model(), 1024).unwrap();
    let second = ResponsesContinuation::new(Uuid::new_v4(), model(), 1024).unwrap();
    first
        .prepare_input(Uuid::from_u128(1), model(), input())
        .await
        .unwrap();
    first
        .preserve(receipt(input(), input(), output()))
        .await
        .unwrap();
    assert_eq!(
        second
            .prepare_input(Uuid::from_u128(1), model(), input())
            .await
            .unwrap(),
        input()
    );
    let mut normalized = input();
    normalized.extend(project_output(&output()).unwrap());
    let wire = first
        .prepare_input(Uuid::from_u128(1), model(), normalized.clone())
        .await
        .unwrap();
    let huge = vec![json!({"type":"reasoning","encrypted_content":"x".repeat(2048)})];
    assert!(matches!(
        first
            .preserve(receipt(normalized.clone(), wire.clone(), huge))
            .await,
        Err(ModelError::BudgetExceeded)
    ));
    assert_eq!(
        first
            .prepare_input(Uuid::from_u128(1), model(), normalized)
            .await
            .unwrap(),
        wire
    );
}

#[tokio::test]
async fn credential_change_cannot_rebind_existing_owner_or_publish_old_output() {
    let state = ResponsesContinuation::new(Uuid::new_v4(), model(), 16384).unwrap();
    assert!(matches!(
        state.preserve(receipt(input(), input(), output())).await,
        Err(ModelError::Authentication)
    ));
    state
        .prepare_input(Uuid::from_u128(1), model(), input())
        .await
        .unwrap();
    assert!(matches!(
        state
            .prepare_input(Uuid::from_u128(2), model(), input())
            .await,
        Err(ModelError::Authentication)
    ));
    assert!(matches!(
        state.prepare_input(Uuid::nil(), model(), input()).await,
        Err(ModelError::Authentication)
    ));
    let mut wrong = receipt(input(), input(), output());
    wrong.credential_scope_id = Uuid::from_u128(2);
    assert!(matches!(
        state.preserve(wrong).await,
        Err(ModelError::Authentication)
    ));
    state
        .preserve(receipt(input(), input(), output()))
        .await
        .unwrap();
}

#[test]
fn mid_turn_window_weaves_canonical_before_last_user_and_keeps_tail_after_window() {
    let account = Uuid::from_u128(1);
    let state = ResponsesContinuation::new(Uuid::new_v4(), model(), 16384).unwrap();
    let window = vec![
        json!({"type":"message","role":"user","content":[{"type":"input_text","text":"old"}]}),
        json!({"type":"message","role":"user","content":[{"type":"input_text","text":"last"}]}),
        json!({"type":"compaction","encrypted_content":"new"}),
    ];
    state
        .bind_window_with_canonical_sync(account, &model(), &window, 1)
        .unwrap();
    let canonical = json!({"role":"user","content":[{"type":"input_text","text":"current rules"}]});
    let tail =
        json!({"role":"user","content":[{"type":"input_text","text":"steer after compact"}]});
    let input = vec![canonical.clone(), tail.clone()];
    assert_eq!(
        state.expand_compaction(&input, Some(0)).unwrap(),
        vec![
            window[0].clone(),
            canonical,
            window[1].clone(),
            window[2].clone(),
            tail
        ]
    );
    assert!(state.expand_compaction(&[], Some(0)).is_err());
}
