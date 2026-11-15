use super::*;
use research_agent_core::submission::ContextSourceKind;
use serde_json::json;

fn grant(id: &str) -> ContextText {
    ContextText {
        source_id: format!("resource:{id}"),
        kind: ContextSourceKind::Paper,
        resource_id: id.into(),
        revision: "grant-identity".into(),
        anchor: Some(json!({"content":"resourceGrant","resource":{"kind":"paperMetadata","resourceId":id},"contextAccess":"readOnly"}).to_string()),
        text: json!({"label":id,"content":"body not loaded"}).to_string(),
    }
}

#[test]
fn model_receives_object_ids_and_access_without_internal_policy_or_inventory() {
    let mut source = grant("scope");
    source.anchor=Some(json!({"content":"resourceGrant","resource":{"kind":"paperGroup","resourceId":"paper","newMemberAccess":"readWrite","parts":[{"resource":{"kind":"paperNotes","resourceId":"paper","excludedNoteIds":["excluded"]},"label":"Private note name","selected":true,"access":"readWrite"}]},"contextAccess":"readWrite"}).to_string());
    source.text = "Private inventory and display names".into();
    let text = ResourceContext::default()
        .update(&[source])
        .unwrap()
        .unwrap();
    assert!(text.contains("paper") && text.contains("readWrite"));
    for hidden in ["Private", "parts", "excludedNoteIds", "anchor", "revision"] {
        assert!(!text.contains(hidden), "{hidden}");
    }
    assert!(decode(&text).is_some());
    let decoded = decode(&text).unwrap();
    let descriptor = decoded.values().next().unwrap();
    assert!(
        descriptor
            .overrides
            .iter()
            .any(|part| part.resource["resourceId"] == "excluded" && !part.selected)
    );
}

#[test]
fn exceptions_use_exact_ids_and_paths_and_read_only_groups_hide_write_preferences() {
    let project = Uuid::new_v4().to_string();
    let mut source = grant(&project);
    let resource = json!({"kind":"manuscript","resourceId":project,"groupAccess":"readWrite","newMemberAccess":"readWrite","parts":[
        {"resource":{"kind":"manuscriptFile","resourceId":project,"path":"main.tex"},"label":"Private title","access":"readWrite","selected":true},
        {"resource":{"kind":"manuscriptFile","resourceId":project,"path":"sections/fixed.tex"},"label":"Private title","access":"readOnly","selected":true}
    ]});
    source.anchor = Some(
        json!({"content":"resourceGrant","resource":resource,"contextAccess":"readWrite"})
            .to_string(),
    );
    let mut scope = ResourceContext::default();
    let text = scope.update(&[source.clone()]).unwrap().unwrap();
    let descriptor = decode(&text).unwrap().into_values().next().unwrap();
    assert_eq!(descriptor.overrides.len(), 1);
    assert_eq!(
        descriptor.overrides[0].resource,
        json!({"kind":"manuscriptFile","resourceId":project,"path":"sections/fixed.tex"})
    );
    assert!(!text.contains("Private title") && !text.contains("main.tex"));
    let mut anchor: serde_json::Value =
        serde_json::from_str(source.anchor.as_ref().unwrap()).unwrap();
    anchor["resource"]["groupAccess"] = json!("readOnly");
    anchor["contextAccess"] = json!("readOnly");
    source.anchor = Some(anchor.to_string());
    let text = scope.update(&[source]).unwrap().unwrap();
    let descriptor = decode(&text).unwrap().into_values().next().unwrap();
    assert_eq!(descriptor.access, "readOnly");
    assert!(descriptor.overrides.is_empty());
}

#[test]
fn an_old_group_only_fragment_is_replaced_when_the_same_grant_has_a_writable_notes_exception() {
    let old = format!(
        "{OPEN}{}{CLOSE}",
        json!([{"sourceId":"resource:scope","resource":{"kind":"paperGroup","resourceId":"paper"},"access":"readOnly"}])
    );
    let mut scope = ResourceContext {
        baseline: decode(&old),
    };
    let mut source = grant("scope");
    source.anchor=Some(json!({"content":"resourceGrant","resource":{"kind":"paperGroup","resourceId":"paper","newMemberAccess":"readOnly","parts":[{"resource":{"kind":"paperNotes","resourceId":"paper","excludedNoteIds":[]},"label":"Notes","selected":true,"access":"readWrite"}]},"contextAccess":"readOnly"}).to_string());
    let updated = scope
        .update(std::slice::from_ref(&source))
        .unwrap()
        .unwrap();
    let descriptor = decode(&updated).unwrap().into_values().next().unwrap();
    assert!(descriptor.overrides.iter().any(|part| part.resource
        == json!({"kind":"paperNotes","resourceId":"paper"})
        && part.access == "readWrite"));
    assert!(scope.update(&[source]).unwrap().is_none());
}

// Derived from upstream PermissionsState::render_diff's known-equal/no-fragment
// contract. Ordering, access changes and an empty scope are application adaptations.
#[test]
fn equal_authority_emits_nothing_and_revocation_explicitly_clears_previous_scope() {
    let mut scope = ResourceContext::default();
    assert!(scope.update(&[]).unwrap().is_none());
    let a = grant("a");
    let b = grant("b");
    assert!(scope.update(&[a.clone(), b.clone()]).unwrap().is_some());
    assert!(scope.update(&[b, a.clone()]).unwrap().is_none());
    let mut changed = a.clone();
    let mut anchor: serde_json::Value =
        serde_json::from_str(changed.anchor.as_ref().unwrap()).unwrap();
    anchor["contextAccess"] = json!("readWrite");
    changed.anchor = Some(anchor.to_string());
    assert!(
        scope
            .update(&[changed])
            .unwrap()
            .unwrap()
            .contains("readWrite")
    );
    let revoked = scope.update(&[]).unwrap().unwrap();
    assert_eq!(decode(&revoked), Some(BTreeMap::new()));
    assert!(revoked.contains("replaces earlier resource scopes"));
    assert!(scope.update(&[]).unwrap().is_none());
    assert!(scope.update(&[a]).unwrap().is_some());
}

#[test]
fn compaction_restores_full_current_scope_even_when_ordinary_turn_has_no_update() {
    let input = research_agent_core::submission::SubmissionInput {
        remote_compaction_v2: None,
        text: "Explain".into(),
        user_input: vec![],
        skill_instructions: vec![],
        instructions: vec![],
        instruction_configs: vec![],
        skills: vec![],
        model: ModelIdentity {
            provider_id: "fixture".into(),
            model_id: "fixture".into(),
            revision: None,
        },
        reasoning_effort: None,
        service_tier: None,
        context: vec![grant("a")],
    };
    let record = SubmissionRecord {
        conversation_id: Uuid::new_v4(),
        request_id: Uuid::new_v4(),
        turn_id: Uuid::new_v4(),
        context_snapshot_id: Uuid::new_v4(),
        input,
    };
    let scope = ResourceContext::default()
        .full(&record.input.context)
        .unwrap()
        .unwrap();
    let source = vec![ModelMessage::User {
        content: vec![ContentPart::Text(scope.clone())],
        source_item_id: None,
    }];
    let fresh = vec![ModelMessage::User {
        content: vec![ContentPart::Text("current request".into())],
        source_item_id: None,
    }];
    let rebuilt = for_compaction(&record, None, &source, &fresh, true).unwrap();
    assert!(
        matches!(&rebuilt[0],ModelMessage::User { content: parts, .. } if matches!(&parts[0],ContentPart::Text(text) if text==&scope))
    );
    assert!(
        matches!(&rebuilt[1],ModelMessage::User { content: parts, .. } if matches!(&parts[0],ContentPart::Text(text) if text=="current request"))
    );
    let rebuilt_again = for_compaction(&record, None, &source, &rebuilt, true).unwrap();
    assert!(rebuilt_again == rebuilt);
}
