use super::*;
fn structured(db: &mut Connection) -> (String, Value) {
    let fields: Vec<Value> = [
        "research_question",
        "approach",
        "key_result",
        "conclusion",
        "limitation",
        "open_question",
    ]
    .iter()
    .map(|id| json!({"field":id,"status":"not_found","statements":[]}))
    .collect();
    seed_structured(db,"synthetic-paper",json!({"kind":"paper_overview_unit_v1","schemaVersion":1,"profileId":"generic_overview_v1","fields":fields})).unwrap()
}

#[test]
fn structured_agent_repair_draft_cannot_be_overwritten_but_recovery_can_be_preserved() {
    let mut db = database();
    let (id, before) = structured(&mut db);
    let staged = write_structured(
        &mut db,
        crate::structured_resources::WriteDraft {
            resource_id: id.clone(),
            expected_revision: before["version"].as_i64().unwrap(),
            content: "{invalid Agent draft".into(),
            request_id: "stage-invalid".into(),
            conversation_id: Some("agent".into()),
            expected_draft_revision: None,
            source_snapshot_id: None,
            submission_id: None,
            editor_kind: "agent".into(),
        },
    )
    .unwrap();
    assert_eq!(staged.status, "invalid");
    let result = commit(
        &mut db,
        "structured",
        &id,
        request(
            "do-not-overwrite-draft",
            before["version"].as_i64().unwrap(),
            before["saved"].clone(),
            before["saved"].clone(),
        ),
    );
    assert!(result.unwrap_err().starts_with("MODEL_DRAFT_PENDING"));
    assert_eq!(read(&db, "structured", &id).unwrap(), before);
    let raw: String = db
        .query_row(
            "select content from structured_resource_drafts where resource_id=?1",
            [&id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(raw, "{invalid Agent draft");
    assert_eq!(
        db.query_row(
            "select count(*) from hbs_commit_receipts where request_id='do-not-overwrite-draft'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    let checkpoint = json!({"format":1,"kind":"structured","observed":before,"content":before["saved"],"ancestor":before["saved"],"baselineAncestor":before["baseline"],"ordinary":[]});
    preservation_save(&mut db, "structured", &id, checkpoint.clone()).unwrap();
    assert_eq!(
        preservation_read(&db, "structured", &id).unwrap(),
        checkpoint
    );
}

#[test]
fn structured_hbs_save_cancels_changed_verification_and_keep_does_not_verify() {
    let mut db = database();
    let (id, before) = structured(&mut db);
    let verified = review_structured(
        &mut db,
        crate::structured_resources::ReviewDraft {
            resource_id: id.clone(),
            expected_revision: before["version"].as_i64().unwrap(),
            request_id: "verify".into(),
            action: "verify".into(),
            scope: None,
            field_ids: Some(vec!["research_question".into()]),
        },
    )
    .unwrap();
    assert!(
        verified
            .reviewed_field_ids
            .contains(&"research_question".into())
    );
    let mut saved = before["saved"].clone();
    saved["fields"][0] = json!({"field":"research_question","status":"filled","statements":[{"id":"q","text":"edited candidate","epistemicRole":"uncertain","evidenceItemIds":[]}]});
    let result = commit(
        &mut db,
        "structured",
        &id,
        request(
            "save",
            verified.revision,
            saved.clone(),
            before["baseline"].clone(),
        ),
    )
    .unwrap();
    let resource = crate::structured_resources::get(&db, &id).unwrap();
    assert!(
        !resource
            .reviewed_field_ids
            .contains(&"research_question".into())
    );
    assert!(
        resource
            .pending_field_ids
            .contains(&"research_question".into())
    );
    let protected: String = db
        .query_row(
            "select protected_fields_json from structured_resources where id=?1",
            [&id],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        serde_json::from_str::<Vec<String>>(&protected)
            .unwrap()
            .contains(&"research_question".into())
    );
    let kept = commit(
        &mut db,
        "structured",
        &id,
        request(
            "keep",
            result["snapshot"]["version"].as_i64().unwrap(),
            saved.clone(),
            saved,
        ),
    )
    .unwrap();
    assert_eq!(kept["snapshot"]["saved"], kept["snapshot"]["baseline"]);
    let resource = crate::structured_resources::get(&db, &id).unwrap();
    assert!(resource.reviewed_field_ids.is_empty());
    let protected: String = db
        .query_row(
            "select protected_fields_json from structured_resources where id=?1",
            [&id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(protected, "[]");
}

#[test]
fn inactive_note_cannot_use_a_baseline_only_write_to_bypass_lifecycle() {
    let mut db = database();
    let id = "00000000-0000-4000-8000-000000000186";
    let before =
        crate::notes::hbs::seed(&mut db, id, serde_json::from_value(note()).unwrap()).unwrap();
    db.execute(
        "update note_pages set status='trashed',trashed_at='now' where id=?1",
        [id],
    )
    .unwrap();
    let result = commit(
        &mut db,
        "note",
        id,
        request(
            "inactive",
            before["version"].as_i64().unwrap(),
            before["saved"].clone(),
            before["baseline"].clone(),
        ),
    );
    assert!(result.unwrap_err().starts_with("MODEL_RESOURCE_INACTIVE"));
    assert_eq!(read(&db, "note", id).unwrap(), before);
}

#[test]
fn navigation_never_accepts_an_outstanding_hbs_body_candidate() {
    let mut db = database();
    let id = "00000000-0000-4000-8000-000000000186";
    let initial =
        crate::notes::hbs::seed(&mut db, id, serde_json::from_value(note()).unwrap()).unwrap();
    let mut candidate = initial["saved"].clone();
    candidate["blocks"][0]["content"]["items"][0]["text"] = json!("unreviewed body");
    let pending = commit(
        &mut db,
        "note",
        id,
        request(
            "generate-body",
            initial["version"].as_i64().unwrap(),
            candidate,
            initial["baseline"].clone(),
        ),
    )
    .unwrap()["snapshot"]
        .clone();
    let before = crate::notes::text_change_set::read(&db, id).unwrap();
    let mut after = before.clone();
    after.document.blocks[0].props["navigation"] = json!("changed");
    let tx = db.transaction().unwrap();
    crate::notes::hbs::synchronize_navigation_if_settled(&tx, &before, &after).unwrap();
    tx.commit().unwrap();
    assert_eq!(
        read(&db, "note", id).unwrap()["baseline"],
        initial["baseline"]
    );
    commit(
        &mut db,
        "note",
        id,
        request(
            "keep-body",
            pending["version"].as_i64().unwrap(),
            pending["saved"].clone(),
            pending["saved"].clone(),
        ),
    )
    .unwrap();
    let before = crate::notes::text_change_set::read(&db, id).unwrap();
    let tx = db.transaction().unwrap();
    crate::notes::hbs::synchronize_navigation_if_settled(&tx, &before, &after).unwrap();
    tx.commit().unwrap();
    after.document.title = Some(after.title.clone());
    assert_eq!(
        read(&db, "note", id).unwrap()["baseline"],
        serde_json::to_value(after.document).unwrap()
    );
}
