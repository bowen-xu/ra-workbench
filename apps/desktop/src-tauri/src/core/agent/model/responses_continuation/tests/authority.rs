use super::*;

#[tokio::test]
async fn explicit_authority_preserves_leading_hook_across_restore_and_rule_refresh() {
    let scope = Uuid::new_v4();
    let account = Uuid::from_u128(1);
    let state = ResponsesContinuation::new(scope, model(), 16384).unwrap();
    let window = vec![json!({"type":"compaction","encrypted_content":"boundary state"})];
    state.bind_window_sync(account, &model(), &window).unwrap();
    let hook = json!({"role":"developer","content":"historical hook"});
    let original = vec![
        json!({"role":"developer","content":"old authority"}),
        hook.clone(),
        input()[0].clone(),
    ];
    let wire = state
        .prepare_scoped_sync(account, model(), original.clone(), Some(1), false)
        .unwrap();
    assert_eq!(wire[1], window[0]);
    assert_eq!(wire[2], hook);
    let output = vec![
        json!({"type":"message","role":"assistant","content":[{"type":"output_text","text":"done"}]}),
    ];
    let mut result = receipt(original.clone(), wire, output.clone());
    result.authority_items = Some(1);
    state.preserve_sync(result).unwrap();
    let encoded = state.encode_checkpoint().unwrap();
    let raw: Value = serde_json::from_str(&encoded).unwrap();
    assert_eq!(raw["schema_version"], 3);
    assert_eq!(raw["checkpoint"]["authority_items"], 1);
    let restored = ResponsesContinuation::restore(scope, model(), 16384, &encoded)
        .unwrap()
        .inherit_for_turn(Uuid::new_v4(), &std::collections::HashMap::new())
        .unwrap();
    let mut next = vec![json!({"role":"developer","content":"new authority"})];
    next.extend_from_slice(&original[1..]);
    next.extend(project_output(&output).unwrap());
    next.push(json!({"role":"user","content":"fresh"}));
    let updated = restored
        .prepare_scoped_sync(account, model(), next.clone(), Some(1), false)
        .unwrap();
    assert_eq!(
        &updated[..3],
        &[next[0].clone(), window[0].clone(), hook.clone()]
    );
    assert_eq!(updated.iter().filter(|item| *item == &hook).count(), 1);
    assert!(
        !updated
            .iter()
            .any(|item| item["content"] == "old authority")
    );
    for boundary in [json!(99), Value::Null] {
        let mut invalid = raw.clone();
        invalid["checkpoint"]["authority_items"] = boundary;
        assert!(
            ResponsesContinuation::restore(scope, model(), 16384, &invalid.to_string()).is_err()
        );
    }
    assert!(
        restored
            .prepare_scoped_sync(account, model(), input(), Some(1), false)
            .is_err()
    );
}
