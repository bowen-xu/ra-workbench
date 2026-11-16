use super::*;

fn published(db: &mut Connection, fields: &[&str], request: &str) -> Resource {
    let target = create_template(db, "字段模板", &format!("{request}-create"), None).unwrap();
    let body = json!({"kind":"reading_template_v1","schemaVersion":1,"name":"字段模板","fields":fields.iter().map(|id|json!({"id":id,"title":id,"question":format!("{id}的内容。") })).collect::<Vec<_>>()});
    let proposed =
        crate::hbs_storage::test_sources::write(db, draft(&target.id, &body.to_string(), request))
            .unwrap();
    assert_eq!(proposed.status, "applied");
    proposed.resource
}

fn adoption(
    target: &Resource,
    template: &Resource,
    confirmed: bool,
    request: &str,
) -> AdoptionDraft {
    AdoptionDraft {
        paper_id: target.paper_id.clone().unwrap(),
        template_id: template.id.clone(),
        template_revision: template.revision,
        expected_revision: target.revision,
        confirmed,
        request_id: request.into(),
    }
}

#[test]
fn first_empty_adoption_is_automatic_but_existing_binding_or_body_requires_confirmation() {
    let mut db = database();
    let target = paper(&db);
    let first = published(&mut db, &["research_question"], "first-template");
    let second = published(&mut db, &["key_result"], "second-template");
    let adopted = crate::hbs_storage::test_sources::adopt(
        &mut db,
        adoption(&target, &first, false, "auto-first"),
    )
    .unwrap();
    assert!(adopted.content.is_none());
    assert!(adopted.review_baseline.is_none());
    assert_eq!(adopted.template_ref.as_ref().unwrap().resource_id, first.id);
    assert!(
        crate::hbs_storage::test_sources::adopt(
            &mut db,
            adoption(&adopted, &second, false, "no-silent-switch")
        )
        .is_err()
    );
    let switched = crate::hbs_storage::test_sources::adopt(
        &mut db,
        adoption(&adopted, &second, true, "confirmed-switch"),
    )
    .unwrap();
    assert_eq!(
        switched.template_ref.as_ref().unwrap().resource_id,
        second.id
    );

    db.execute("insert into bibliographic_items(id,item_type,title,created_at,updated_at) values('populated-paper','journalArticle','Populated','now','now')",[]).unwrap();
    let populated = find_by_paper(&db, "populated-paper", "structuredReading").unwrap();
    let saved = crate::hbs_storage::test_sources::write(
        &mut db,
        draft(&populated.id, &reading("已有正文。"), "existing-body"),
    )
    .unwrap()
    .resource;
    assert!(
        crate::hbs_storage::test_sources::adopt(
            &mut db,
            adoption(&saved, &first, false, "no-silent-body-change")
        )
        .is_err()
    );
}

#[test]
fn paper_owned_template_cannot_be_bound_to_another_paper() {
    let mut db = database();
    let target = paper(&db);
    db.execute("insert into bibliographic_items(id,item_type,title,created_at,updated_at) values('other-paper','journalArticle','Other','now','now')",[]).unwrap();
    let own = ensure_paper_template(&db, "other-paper").unwrap();
    let published = crate::hbs_storage::test_sources::write(
        &mut db,
        draft(&own.id, &template(), "own-template-write"),
    )
    .unwrap()
    .resource;
    assert!(
        crate::hbs_storage::test_sources::adopt(
            &mut db,
            adoption(&target, &published, true, "wrong-paper")
        )
        .is_err()
    );
    assert!(get(&db, &target.id).unwrap().template_ref.is_none());
}

#[test]
fn template_switch_retains_old_field_bodies_diff_and_verification_and_can_reactivate_them() {
    let mut db = database();
    let target = paper(&db);
    let generated = crate::hbs_storage::test_sources::write(
        &mut db,
        draft(&target.id, &reading("原判断。"), "generate"),
    )
    .unwrap()
    .resource;
    let kept = decision(&mut db, &generated, "keep", None, None);
    let verified = decision(
        &mut db,
        &kept,
        "verify",
        None,
        Some(vec!["research_question"]),
    );
    let mut update = draft(&target.id, &reading("新的待审判断。"), "edit");
    update.expected_revision = verified.revision;
    let changed = crate::hbs_storage::test_sources::write(&mut db, update)
        .unwrap()
        .resource;
    let fewer = published(&mut db, &["key_result"], "fewer-fields");
    let adopted = crate::hbs_storage::test_sources::adopt(
        &mut db,
        adoption(&changed, &fewer, true, "remove-field"),
    )
    .unwrap();
    let content = adopted.content.as_ref().unwrap();
    assert_eq!(content["fields"].as_array().unwrap().len(), 1);
    let retained = content["retainedFields"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["field"] == "research_question")
        .unwrap();
    assert_eq!(retained["statements"][0]["text"], "新的待审判断。");
    assert!(
        adopted
            .pending_field_ids
            .contains(&"$retained:research_question".into())
    );
    let baseline = adopted.review_baseline.as_ref().unwrap()["retainedFields"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["field"] == "research_question")
        .unwrap();
    assert_eq!(baseline["statements"][0]["text"], "原判断。");
    let mut regenerated = content.clone();
    regenerated
        .as_object_mut()
        .unwrap()
        .remove("retainedFields");
    let mut input = draft(
        &target.id,
        &regenerated.to_string(),
        "regenerate-without-old-fields",
    );
    input.expected_revision = adopted.revision;
    let regenerated = crate::hbs_storage::test_sources::write(&mut db, input).unwrap();
    assert_eq!(regenerated.status, "applied", "{:?}", regenerated.errors);
    assert_eq!(
        regenerated.resource.content.as_ref().unwrap()["retainedFields"],
        content["retainedFields"]
    );

    let restored = decision(
        &mut db,
        &regenerated.resource,
        "undo",
        None,
        Some(vec!["$retained:research_question"]),
    );
    assert!(
        restored
            .reviewed_field_ids
            .contains(&"$retained:research_question".into())
    );
    let reactivated_template = published(
        &mut db,
        &["research_question", "key_result"],
        "restore-fields",
    );
    let reactivated = crate::hbs_storage::test_sources::adopt(
        &mut db,
        adoption(&restored, &reactivated_template, true, "restore-field"),
    )
    .unwrap();
    let active = reactivated.content.as_ref().unwrap()["fields"]
        .as_array()
        .unwrap();
    assert_eq!(active[0]["field"], "research_question");
    assert_eq!(active[0]["statements"][0]["text"], "原判断。");
    assert!(
        reactivated
            .reviewed_field_ids
            .contains(&"research_question".into())
    );
    assert!(
        !reactivated.content.as_ref().unwrap()["retainedFields"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v["field"] == "research_question")
    );
}

#[test]
fn adopting_a_template_rejects_invalid_derived_fact_body_until_exact_quotes_are_supplied() {
    let mut db = database();
    let target = paper(&db);
    let sha = "a".repeat(64);
    db.execute("insert into content_objects(sha256,relative_path,mime_type,byte_size,created_at) values(?1,'paper.pdf','application/pdf',1,'now')",[&sha]).unwrap();
    db.execute("insert into attachments(id,item_id,content_sha256,display_name,created_at) values('attachment','paper',?1,'paper.pdf','now')",[&sha]).unwrap();
    db.execute("insert into source_anchors(id,item_id,attachment_id,content_sha256,page_index,rects_json,quote_exact,quote_prefix,quote_suffix,created_at) values('anchor','paper','attachment',?1,0,'[]','A factual source sentence.','','','now')",[&sha]).unwrap();
    let mut body: serde_json::Value =
        serde_json::from_str(&reading("A factual source sentence.")).unwrap();
    body["fields"][0]["statements"][0]["epistemicRole"] = json!("source_paraphrase");
    body["fields"][0]["statements"][0]["evidenceItemIds"] = json!(["anchor"]);
    let mut input = draft(&target.id, &body.to_string(), "generic-fact");
    input.source_snapshot_id = Some("anchor:anchor".into());
    let original = crate::hbs_storage::test_sources::write(&mut db, input)
        .unwrap()
        .resource;
    assert!(original.template_ref.is_none());
    assert!(original.content.is_some());
    let global = published(&mut db, &["research_question"], "fact-template");
    let failed = crate::structured_resources::adopt_template_with(
        &mut crate::hbs_storage::test_sources::Sources::rejecting_at(
            json!({"kind":"paper_overview_unit_v1"}),
            "$",
            "陈述 question-1 必须为每个引用来源提供一条连续原文摘录",
        ),
        &mut db,
        adoption(&original, &global, true, "invalid-derived"),
    )
    .map_err(crate::hbs_storage::boundary::BoundaryError::into_static);
    assert!(matches!(failed, Err(Error::Invalid(_))));
    let unchanged = get(&db, &target.id).unwrap();
    assert_eq!(unchanged.revision, original.revision);
    assert_eq!(unchanged.content, original.content);
    assert!(unchanged.template_ref.is_none());
    let own = ensure_paper_template(&db, "paper").unwrap();
    let proposal_body = json!({"kind":"reading_template_v1","schemaVersion":1,"name":"本文事实模板","fields":[{"id":"research_question","title":"问题","question":"研究问题是什么？"}]});
    let saved_template = crate::hbs_storage::test_sources::write(
        &mut db,
        draft(&own.id, &proposal_body.to_string(), "own-fact-template"),
    )
    .unwrap()
    .resource;
    let failed = crate::structured_resources::adopt_template_with(
        &mut crate::hbs_storage::test_sources::Sources::rejecting_at(
            json!({"kind":"paper_overview_unit_v1"}),
            "$",
            "陈述 question-1 必须为每个引用来源提供一条连续原文摘录",
        ),
        &mut db,
        adoption(&original, &saved_template, true, "invalid-own-derived"),
    )
    .map_err(crate::hbs_storage::boundary::BoundaryError::into_static);
    assert!(matches!(failed, Err(Error::Invalid(_))));
    assert_eq!(get(&db, &target.id).unwrap().content, original.content);
    body["fields"][0]["statements"][0]["evidenceQuotes"] =
        json!([{"evidenceItemId":"anchor","quote":"A factual source sentence."}]);
    let mut repair = draft(&target.id, &body.to_string(), "fact-quotes");
    repair.expected_revision = original.revision;
    repair.source_snapshot_id = Some("anchor:anchor".into());
    let repaired = crate::hbs_storage::test_sources::write(&mut db, repair)
        .unwrap()
        .resource;
    let adopted = crate::hbs_storage::test_sources::adopt(
        &mut db,
        adoption(&repaired, &global, true, "valid-derived"),
    )
    .unwrap();
    assert_eq!(
        adopted.template_ref.as_ref().unwrap().resource_id,
        global.id
    );
    assert_eq!(
        adopted.content.as_ref().unwrap()["fields"][0]["statements"][0]["evidenceQuotes"][0]["quote"],
        "A factual source sentence."
    );
}
