use super::*;
use crate::agent::context_session::{configuration, structured::TEMPLATE_CATALOG_ID};
#[path = "structured/receipt_faults.rs"]
mod receipt_faults;

fn template_fixture(write: bool) -> Fixture {
    let mut fixture = fixture(true);
    let db = fixture.tools.store.connection().unwrap();
    let resource = Resource::TemplateCatalog {
        resource_id: TEMPLATE_CATALOG_ID.into(),
        access: Some(if write {
            Access::ReadWrite
        } else {
            Access::ReadOnly
        }),
    };
    fixture.tools.record.input.context = resource_grants::grants(
        &db,
        std::slice::from_ref(&resource),
        Some(crate::agent::writing_resources::test_sources::source()),
    )
    .unwrap();
    configuration::seed_in(
        &db,
        fixture.tools.record.conversation_id,
        &[resource],
        Some(crate::agent::writing_resources::test_sources::source()),
    )
    .unwrap();
    drop(db);
    fixture
}
fn add(body: &str) -> String {
    format!(
        "*** Begin Patch\n*** Add File: source://agents/skills/edit-structured-resources/assets/reading\n{}*** End Patch",
        body.lines()
            .map(|line| format!("+{line}\n"))
            .collect::<String>()
    )
}
const TEMPLATE: &str = r#"{"kind":"reading_template_v1","schemaVersion":1,"name":"Synthetic template","fields":[{"id":"question","title":"Question","question":"What is the research question?"}]}"#;

#[tokio::test]
async fn invalid_template_keeps_real_identity_and_repairs_to_saved_diff() {
    let f = template_fixture(true);
    let result = f.tools.apply("create-template", &add("{")).await.unwrap();
    assert_eq!(result["status"], "declined", "{result}");
    assert_eq!(result["targets"][0]["status"], "failed", "{result}");
    let receipt = &result["targets"][0]["structuredResource"];
    assert_eq!(receipt["status"], "invalid");
    let uri = receipt["sourceUrl"].as_str().unwrap();
    let id = receipt["resourceId"].as_str().unwrap();
    assert!(uri.starts_with("source://agents/skills/edit-structured-resources/assets/reading/"));
    let shown = read(&f, uri).await;
    assert_eq!(shown["text"], "{\n");
    assert_eq!(shown["draft"]["revision"], 1);
    // A later draft revision must not invalidate a patch whose context still matches.
    f.tools
        .store
        .connection()
        .unwrap()
        .execute(
            "update structured_resource_drafts set revision=revision+1 where resource_id=?1",
            [id],
        )
        .unwrap();
    let fixed =
        format!("*** Begin Patch\n*** Update File: {uri}\n@@\n-{{\n+{TEMPLATE}\n*** End Patch");
    let changed = f.tools.apply("repair-template", &fixed).await.unwrap();
    assert_eq!(changed["targets"][0]["status"], "applied", "{changed}");
    assert_eq!(changed["fileDiffs"][0]["comparison"], "saved");
    assert!(changed["fileDiffs"][0].get("beforeSaved").is_some());
    assert!(changed["fileDiffs"][0]["beforeSaved"].is_null());
    assert_eq!(
        changed["fileDiffs"][0]["afterSaved"],
        serde_json::from_str::<Value>(TEMPLATE).unwrap()
    );
    assert!(result.get("fileDiffs").is_none());
    assert!(
        changed["targets"][0]["structuredResource"]
            .get("savedDiff")
            .is_none()
    );
    let target = &changed["targets"][0];
    assert!(target["changeId"].is_string());
    assert!(target.get("proposalId").is_none());
    assert_eq!(
        target["changeId"],
        target["structuredResource"]["receipt"]["requestId"]
    );
    assert!(target["structuredResource"]["draftId"].is_null());
    assert!(result["targets"][0]["changeId"].is_null());
    let replayed = f.tools.apply("repair-template", &fixed).await.unwrap();
    assert_eq!(
        replayed, changed,
        "successful changes retain identity on replay"
    );
    {
        let db = f.tools.store.connection().unwrap();
        let saved = crate::structured_resources::get(&db, id).unwrap();
        assert_eq!(saved.revision, 1);
        assert!(saved.content.is_some());
        assert!(!saved.pending_field_ids.is_empty());
        let candidate = crate::structured_resources::read_text(
            &db,
            id,
            Some(&f.tools.record.conversation_id.to_string()),
            false,
        )
        .unwrap();
        assert!(candidate.draft.is_none());
        let config = configuration::read_in(&db, f.tools.record.conversation_id)
            .unwrap()
            .unwrap();
        assert!(
            config.state["persistent"]
                .as_array()
                .unwrap()
                .iter()
                .any(|selection| selection["resource"]["resourceId"] == id)
        );
    }
    let formal = f
        .tools
        .read(Read {
            path: uri.into(),
            start_char: 0,
            max_chars: 8000,
            view: Some("saved".into()),
        })
        .await
        .unwrap();
    assert!(
        formal["text"]
            .as_str()
            .unwrap()
            .contains("Synthetic template")
    );
    let before_text = formal["text"].as_str().unwrap();
    let after_text = before_text.replace("Synthetic template", "Synthetic template v2");
    let next_patch = format!(
        "*** Begin Patch\n*** Update File: {uri}\n@@\n{}{}*** End Patch",
        before_text
            .lines()
            .map(|line| format!("-{line}\n"))
            .collect::<String>(),
        after_text
            .lines()
            .map(|line| format!("+{line}\n"))
            .collect::<String>()
    );
    let next = f
        .tools
        .apply("later-saved-template", &next_patch)
        .await
        .unwrap();
    assert_eq!(next["status"], "complete", "{next}");
    assert_eq!(
        next["fileDiffs"][0]["beforeSaved"],
        changed["fileDiffs"][0]["afterSaved"]
    );
    assert_eq!(
        f.tools.apply("repair-template", &fixed).await.unwrap(),
        changed
    );
}

#[tokio::test]
async fn read_only_catalog_cannot_create_objects_and_wrong_parent_never_grants_access() {
    let f = template_fixture(false);
    let result = f.tools.apply("forbidden", &add(TEMPLATE)).await.unwrap();
    assert_eq!(result["status"], "failed");
    assert_eq!(result["targets"][0]["status"], "failed");
    let db = f.tools.store.connection().unwrap();
    assert_eq!(
        crate::structured_resources::list_headers(&db, None)
            .unwrap()
            .len(),
        0
    );
}

#[tokio::test]
async fn unknown_tool_receipt_recovers_the_original_invalid_draft_without_replaying() {
    let f = template_fixture(true);
    let patch = add("{");
    let result = f.tools.apply("interrupted-create", &patch).await.unwrap();
    let id = result["targets"][0]["structuredResource"]["resourceId"]
        .as_str()
        .unwrap();
    let operation = format!("{}:interrupted-create", f.tools.record.request_id);
    {
        let db = f.tools.store.connection().unwrap();
        db.execute("update agent_text_patch_calls set state='running',results_json='{\"status\":\"unknown\",\"targets\":[]}' where request_id=?1",[&operation]).unwrap();
        db.execute(
            "delete from agent_text_patch_calls where request_id=?1",
            [format!("{operation}:file:0")],
        )
        .unwrap();
    }
    let restored = f.tools.apply("interrupted-create", &patch).await.unwrap();
    assert_eq!(restored["status"], "declined", "{restored}");
    assert_eq!(
        restored["targets"][0]["structuredResource"]["resourceId"],
        id
    );
    let db = f.tools.store.connection().unwrap();
    let draft = crate::structured_resources::read_text(
        &db,
        id,
        Some(&f.tools.record.conversation_id.to_string()),
        false,
    )
    .unwrap()
    .draft
    .unwrap();
    assert_eq!(draft.revision, 1);
    assert_eq!(
        crate::structured_resources::list_headers(&db, None)
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn configuration_seed_survives_reopen_and_never_overwrites_manual_choices() {
    let f = template_fixture(true);
    let conversation = f.tools.record.conversation_id;
    {
        let db = f.tools.store.connection().unwrap();
        assert!(
            configuration::seed_in(
                &db,
                conversation,
                &[
                    Resource::TemplateCatalog {
                        resource_id: TEMPLATE_CATALOG_ID.into(),
                        access: Some(Access::ReadOnly),
                    },
                    Resource::TemplateCatalog {
                        resource_id: TEMPLATE_CATALOG_ID.into(),
                        access: Some(Access::ReadWrite),
                    },
                ],
                Some(crate::agent::writing_resources::test_sources::source())
            )
            .is_err()
        ); // Permissions do not make a second resource identity.
        db.execute("update agent_context_configurations set revision=2,state_json='{\"schemaVersion\":1,\"persistent\":[],\"temporary\":[]}' where conversation_id=?1",[conversation.to_string()]).unwrap();
        configuration::seed_in(
            &db,
            conversation,
            &[Resource::TemplateCatalog {
                resource_id: TEMPLATE_CATALOG_ID.into(),
                access: Some(Access::ReadWrite),
            }],
            Some(crate::agent::writing_resources::test_sources::source()),
        )
        .unwrap();
    }
    let db = Connection::open(f._root.path().join("fixture.sqlite3")).unwrap();
    let restored = configuration::read_in(&db, conversation).unwrap().unwrap();
    assert_eq!(restored.revision, 2);
    assert!(restored.state["persistent"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn full_context_can_edit_an_existing_draft_but_cannot_allocate_an_orphan() {
    let f = template_fixture(true);
    let uri = {
        let db = f.tools.store.connection().unwrap();
        let config = configuration::read_in(&db, f.tools.record.conversation_id)
            .unwrap()
            .unwrap();
        let mut state = config.state;
        let mut uri = String::new();
        for index in 0..31 {
            let object = crate::structured_resources::create_template(
                &db,
                "Draft",
                &format!("capacity-{index}"),
                Some(&f.tools.record.conversation_id.to_string()),
            )
            .unwrap();
            if index == 0 {
                uri = object.source_uri;
            }
            state["persistent"].as_array_mut().unwrap().push(json!({
                "resource":{"kind":"structuredObject","resourceId":object.id,"access":"readWrite"},
                "label":"Draft"
            }));
        }
        db.execute(
            "update agent_context_configurations set state_json=?2 where conversation_id=?1",
            params![
                f.tools.record.conversation_id.to_string(),
                state.to_string()
            ],
        )
        .unwrap();
        uri
    };
    let denied = f
        .tools
        .apply("capacity-create", &add(TEMPLATE))
        .await
        .unwrap();
    assert_eq!(denied["targets"][0]["status"], "failed");
    read(&f, &uri).await;
    let patch = format!("*** Begin Patch\n*** Update File: {uri}\n@@\n+{TEMPLATE}\n*** End Patch");
    let edited = f.tools.apply("capacity-edit", &patch).await.unwrap();
    assert_eq!(edited["targets"][0]["status"], "applied", "{edited}");
    assert_eq!(
        crate::structured_resources::list_headers(&f.tools.store.connection().unwrap(), None)
            .unwrap()
            .len(),
        31
    );
}

#[tokio::test]
async fn new_evidence_without_a_live_read_proof_is_cached_without_history_fallback() {
    let mut f = fixture(true);
    let uri = {
        let db = f.tools.store.connection().unwrap();
        db.execute("insert into bibliographic_items(id,item_type,title,created_at,updated_at) values('target-paper','document','Target','now','now')",[]).unwrap();
        let resource = crate::structured_resources::ensure_paper_resources(&db, "target-paper")
            .unwrap()
            .into_iter()
            .find(|resource| resource.kind == "portrait")
            .unwrap();
        f.tools.record.input.context = resource_grants::grants(
            &db,
            &[Resource::StructuredObject {
                resource_id: resource.id,
                access: Some(Access::ReadWrite),
            }],
            Some(crate::agent::writing_resources::test_sources::source()),
        )
        .unwrap();
        resource.source_uri
    };
    read(&f, &uri).await;
    let raw = r#"{"kind":"paper_portrait_v1","schemaVersion":1,"claims":[{"evidenceItemIds":["unshared-history-id"]}]}"#;
    let patch = format!("*** Begin Patch\n*** Update File: {uri}\n@@\n+{raw}\n*** End Patch");
    let result = f.tools.apply("unproved-evidence", &patch).await.unwrap();
    assert_eq!(result["status"], "declined", "{result}");
    assert_eq!(result["targets"][0]["status"], "failed", "{result}");
    let db = f.tools.store.connection().unwrap();
    let (source,body):(String,String)=db.query_row("select source_snapshot_id,content from structured_resource_drafts where creator_conversation_id=?1",[f.tools.record.conversation_id.to_string()],|row|Ok((row.get(0)?,row.get(1)?))).unwrap();
    assert!(source.starts_with("unread-source:"));
    assert_eq!(body.trim(), raw);
    let saved =
        crate::structured_resources::find_by_paper(&db, "target-paper", "portrait").unwrap();
    assert_eq!(saved.revision, 0);
    assert!(saved.content.is_none());
}
