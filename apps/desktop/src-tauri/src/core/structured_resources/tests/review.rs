use super::*;

#[test]
fn clear_saves_empty_baseline_and_identical_regeneration_is_pending_after_reopen() {
    let mut db = database();
    let target = paper(&db);
    let generated = crate::hbs_storage::test_sources::write(
        &mut db,
        draft(&target.id, &reading("相同内容。"), "generate"),
    )
    .unwrap()
    .resource;
    let accepted = decision(&mut db, &generated, "keep", Some("all"), None);
    let verified = decision(
        &mut db,
        &accepted,
        "verify",
        None,
        Some(vec!["research_question"]),
    );
    let cleared = decision(&mut db, &verified, "clear", None, None);
    let reopened = get(&db, &target.id).unwrap();
    assert_eq!(reopened.revision, cleared.revision);
    assert!(reopened.content.is_none());
    assert!(reopened.review_baseline.is_none());
    assert!(reopened.pending_field_ids.is_empty());
    assert!(reopened.reviewed_field_ids.is_empty());
    assert_eq!(
        reopened
            .template_ref
            .as_ref()
            .map(|v| (&v.resource_id, v.revision)),
        verified
            .template_ref
            .as_ref()
            .map(|v| (&v.resource_id, v.revision))
    );
    assert_eq!(
        repository::get_version(&db, &target.id, verified.revision)
            .unwrap()
            .content,
        verified.content
    );
    let mut input = draft(&target.id, &reading("相同内容。"), "regenerate");
    input.expected_revision = reopened.revision;
    let regenerated = crate::hbs_storage::test_sources::write(&mut db, input)
        .unwrap()
        .resource;
    assert!(regenerated.review_baseline.is_none());
    assert!(
        regenerated
            .pending_field_ids
            .contains(&"research_question".into())
    );
    let kept = decision(&mut db, &regenerated, "keep", Some("unreviewed"), None);
    assert!(kept.pending_field_ids.is_empty());
}

#[test]
fn clean_human_edit_clears_old_verification_protection_before_the_next_agent_change() {
    let mut db = database();
    let target = paper(&db);
    let mut input = draft(&target.id, &reading("人工第一版。"), "human-first");
    input.editor_kind = "human".into();
    let clean = crate::hbs_storage::test_sources::write(&mut db, input)
        .unwrap()
        .resource;
    let verified = decision(
        &mut db,
        &clean,
        "verify",
        None,
        Some(vec!["research_question"]),
    );
    let mut input = draft(
        &target.id,
        &reading("人工修改并取消校验。"),
        "human-clean-edit",
    );
    input.editor_kind = "human".into();
    input.expected_revision = verified.revision;
    let edited = crate::hbs_storage::test_sources::write(&mut db, input)
        .unwrap()
        .resource;
    assert!(edited.pending_field_ids.is_empty());
    assert!(edited.reviewed_field_ids.is_empty());
    let mut input = draft(&target.id, &reading("Agent新内容。"), "agent-next");
    input.expected_revision = edited.revision;
    let pending = crate::hbs_storage::test_sources::write(&mut db, input)
        .unwrap()
        .resource;
    let kept = decision(&mut db, &pending, "keep", Some("unreviewed"), None);
    assert!(kept.pending_field_ids.is_empty());
}

#[test]
fn invalid_partial_first_generation_undo_is_rejected_and_whole_generation_undo_remains_available() {
    let mut db = database();
    let target = paper(&db);
    let generated = crate::hbs_storage::test_sources::write(
        &mut db,
        draft(&target.id, &reading("初次生成。"), "initial"),
    )
    .unwrap()
    .resource;
    let kept = decision(
        &mut db,
        &generated,
        "keep",
        None,
        Some(vec!["research_question"]),
    );
    let error = crate::structured_resources::review_with(
        &mut crate::hbs_storage::test_sources::Sources::rejecting_at(
            json!({}),
            "$",
            "declared incomplete partial-undo response",
        ),
        &mut db,
        ReviewDraft {
            resource_id: target.id.clone(),
            expected_revision: kept.revision,
            request_id: "bad-partial".into(),
            action: "undo".into(),
            field_ids: None,
            scope: Some("all".into()),
        },
        crate::structured_resources::ReviewRoute::Static,
    )
    .map_err(crate::hbs_storage::boundary::BoundaryError::into_static)
    .unwrap_err();
    assert!(error.to_string().contains("撤销快捷键"));
    assert_eq!(get(&db, &target.id).unwrap().revision, kept.revision);
    let undone = decision(&mut db, &kept, "undoGeneration", None, None);
    assert!(undone.content.is_none());
    assert!(undone.review_baseline.is_none());
}

#[test]
fn resolving_one_field_preserves_the_independent_order_diff() {
    let original = serde_json::json!({"fields":[{"field":"a","text":"A"},{"field":"b","text":"B"}],"kind":"test"});
    let changed = serde_json::json!({"fields":[{"field":"b","text":"B2"},{"field":"a","text":"A"}],"kind":"test"});
    let mut baseline = Some(original.clone());
    super::super::content::replace_fields(&mut baseline, Some(&changed), &["b".into()], true);
    assert_eq!(
        super::super::content::pending(Some(&changed), baseline.as_ref()),
        vec!["$order"]
    );
    let mut restored = Some(changed);
    super::super::content::replace_fields(&mut restored, Some(&original), &["b".into()], false);
    assert_eq!(
        super::super::content::pending(restored.as_ref(), Some(&original)),
        vec!["$order"]
    );
    assert_eq!(restored.unwrap()["fields"][0]["field"], "b");
}

#[test]
fn keep_unreviewed_preserves_verified_diff_and_undo_restores_exact_review_state() {
    let mut db = database();
    let target = paper(&db);
    let original = crate::hbs_storage::test_sources::write(
        &mut db,
        draft(
            &target.id,
            &reading("材料不足，尚不确定研究问题。"),
            "initial",
        ),
    )
    .unwrap()
    .resource;
    let kept = decision(&mut db, &original, "keep", None, None);
    let verified = decision(
        &mut db,
        &kept,
        "verify",
        None,
        Some(vec!["research_question"]),
    );
    let mut input = draft(&target.id, &reading("新的不确定判断。"), "edit");
    input.expected_revision = verified.revision;
    let edited = crate::hbs_storage::test_sources::write(&mut db, input)
        .unwrap()
        .resource;
    assert!(
        !edited
            .reviewed_field_ids
            .contains(&"research_question".into())
    );
    let partial = decision(&mut db, &edited, "keep", Some("unreviewed"), None);
    assert!(
        partial
            .pending_field_ids
            .contains(&"research_question".into())
    );
    let restored = decision(
        &mut db,
        &partial,
        "undo",
        None,
        Some(vec!["research_question"]),
    );
    assert!(
        restored
            .reviewed_field_ids
            .contains(&"research_question".into())
    );
    assert_eq!(restored.content, kept.content);
    assert!(restored.pending_field_ids.is_empty());
}

#[test]
fn human_edits_synchronize_clean_baseline_but_preserve_an_existing_agent_diff() {
    let mut db = database();
    let target = paper(&db);
    let mut input = draft(&target.id, &reading("原判断。"), "human-initial");
    input.editor_kind = "human".into();
    let clean = crate::hbs_storage::test_sources::write(&mut db, input)
        .unwrap()
        .resource;
    assert!(clean.pending_field_ids.is_empty());
    let mut input = draft(&target.id, &reading("Agent的新判断。"), "agent-edit");
    input.expected_revision = clean.revision;
    let pending = crate::hbs_storage::test_sources::write(&mut db, input)
        .unwrap()
        .resource;
    let mut input = draft(
        &target.id,
        &reading("人工继续修改同一待审字段。"),
        "human-edit",
    );
    input.expected_revision = pending.revision;
    input.editor_kind = "human".into();
    let edited = crate::hbs_storage::test_sources::write(&mut db, input)
        .unwrap()
        .resource;
    assert_eq!(edited.review_baseline, clean.content);
    assert_eq!(edited.pending_field_ids, vec!["research_question"]);
    let restored = decision(&mut db, &edited, "undo", None, None);
    assert_eq!(restored.content, clean.content);
}

#[test]
fn first_generation_undo_all_restores_absent_content_while_retaining_identity() {
    let mut db = database();
    let target = paper(&db);
    let generated = crate::hbs_storage::test_sources::write(
        &mut db,
        draft(&target.id, &reading("尚不确定。"), "initial"),
    )
    .unwrap()
    .resource;
    let mut ids = generated
        .pending_field_ids
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    ids.reverse();
    let restored = decision(&mut db, &generated, "undo", None, Some(ids));
    assert_eq!(restored.id, target.id);
    assert!(restored.content.is_none());
    assert!(restored.pending_field_ids.is_empty());
}

#[test]
fn coverage_and_order_changes_have_independent_diff_chunks() {
    let mut db = database();
    paper(&db);
    let target = find_by_paper(&db, "paper", "portrait").unwrap();
    let mut value = json!({"kind":"paper_portrait_v1","schemaVersion":1,"coverage":"初始覆盖范围。","claims":[
        {"id":"genre","dimension":"genre","label":"不确定","description":"证据不足。","certainty":"uncertain","evidence":[]},
        {"id":"mode","dimension":"mode","label":"不确定","description":"证据不足。","certainty":"uncertain","evidence":[]},
        {"id":"paradigm","dimension":"paradigm","label":"不确定","description":"证据不足。","certainty":"uncertain","evidence":[]}
    ],"template":[{"id":"question","title":"问题","question":"所回应的问题。","claimIds":[]}]});
    let original = crate::hbs_storage::test_sources::write(
        &mut db,
        draft(&target.id, &value.to_string(), "portrait-initial"),
    )
    .unwrap()
    .resource;
    let kept = decision(&mut db, &original, "keep", None, None);
    value["coverage"] = json!("新的覆盖范围。");
    let mut input = draft(&target.id, &value.to_string(), "coverage");
    input.expected_revision = kept.revision;
    let changed = crate::hbs_storage::test_sources::write(&mut db, input)
        .unwrap()
        .resource;
    assert_eq!(changed.pending_field_ids, vec!["$metadata"]);
    let restored = decision(&mut db, &changed, "undo", None, Some(vec!["$metadata"]));
    assert_eq!(restored.content, kept.content);
    let mut value = kept.content.unwrap();
    value["claims"].as_array_mut().unwrap().reverse();
    let mut input = draft(&target.id, &value.to_string(), "order");
    input.expected_revision = restored.revision;
    let changed = crate::hbs_storage::test_sources::write(&mut db, input)
        .unwrap()
        .resource;
    assert_eq!(changed.pending_field_ids, vec!["$order"]);
    let restored = decision(&mut db, &changed, "undo", None, Some(vec!["$order"]));
    assert!(restored.pending_field_ids.is_empty());
}
