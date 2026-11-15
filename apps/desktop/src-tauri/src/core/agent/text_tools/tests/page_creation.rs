use super::*;
use crate::agent::text_tools::note_review;

async fn create(f: &Fixture) -> decisions::Review {
    let source = path(f.parent);
    let patch = format!(
        "*** Begin Patch\n*** Add File: {source}\n+---\n+title: Generated page\n+---\n+\n+Generated content\n*** End Patch"
    );
    let result = f.tools.apply("create-page", &patch).await.unwrap();
    assert_eq!(result["targets"][0]["status"], "applied", "{result}");
    review(f)
}
#[tokio::test]
async fn llm_creation_is_independent_with_an_established_entrance_and_only_body_review() {
    let f = fixture(true);
    let shown = create(&f).await;
    let child = shown.pages.iter().find(|p| p.kind == "create").unwrap();
    assert!(!child.metadata_pending);
    assert!(!child.structural_metadata_pending);
    assert!(!child.chunks.is_empty());
    let db = f.tools.store.connection().unwrap();
    let created = note::load(&db, &child.page_id).unwrap();
    assert!(created.parent_id.is_none());
    assert_eq!(created.status, "active");
    let source = crate::notes::hbs::read(&db, &f.parent.to_string()).unwrap();
    let link = source["saved"]["blocks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["content"]["targetId"] == child.page_id)
        .unwrap();
    assert_eq!(link["type"], "researchObjectReference");
    assert_eq!(link["props"]["pageEntry"], true);
    assert!(
        source["baseline"]["blocks"]
            .as_array()
            .unwrap()
            .contains(link),
        "creation entrance is already established"
    );
    drop(db);
    let request = note_review::Request {
        conversation_id: f.tools.record.conversation_id,
        proposal_id: shown.proposal_id.clone(),
        generation: shown.generation,
        request_id: Uuid::new_v4(),
        page_id: child.page_id.clone(),
        action: "undo".into(),
        chunk_id: None,
        document: None,
        history_request_id: None,
        direction: None,
        expected_revision: None,
    };
    note_review::resolve_in(&f.tools.store, request).unwrap();
    let db = f.tools.store.connection().unwrap();
    let child = note::load(&db, &child.page_id).unwrap();
    assert_eq!(child.status, "active");
    assert!(child.document.blocks.is_empty());
    let source = crate::notes::hbs::read(&db, &f.parent.to_string()).unwrap();
    assert!(source["saved"]["blocks"].as_array().unwrap().contains(link));
}
#[tokio::test]
async fn creating_an_entrance_does_not_accept_existing_unrelated_source_content() {
    let f = fixture(true);
    {
        let mut db = f.tools.store.connection().unwrap();
        let initial = crate::hbs_storage::read(&db, "note", &f.parent.to_string()).unwrap();
        let mut candidate = initial["saved"].clone();
        candidate["blocks"][0]["content"]["items"][0]["text"] =
            json!("Unaccepted existing source content");
        crate::hbs_storage::test_sources::commit(
            &mut db,
            "note",
            &f.parent.to_string(),
            crate::hbs_storage::Commit {
                id: "existing-candidate".into(),
                expected_version: initial["version"].as_i64().unwrap(),
                saved: candidate,
                baseline: initial["baseline"].clone(),
            },
        )
        .unwrap();
    }
    create(&f).await;
    let db = f.tools.store.connection().unwrap();
    let source = crate::notes::hbs::read(&db, &f.parent.to_string()).unwrap();
    assert_eq!(
        source["saved"]["blocks"][0]["content"]["items"][0]["text"],
        "Unaccepted existing source content"
    );
    assert_eq!(
        source["baseline"]["blocks"][0]["content"]["items"][0]["text"],
        "parent"
    );
}
#[tokio::test]
async fn obsolete_creation_metadata_retry_gets_a_definite_noop_without_flushing_or_restoring_a_cut_page()
 {
    let f = fixture(true);
    let shown = create(&f).await;
    let child = shown.pages.iter().find(|p| p.kind == "create").unwrap();
    {
        let mut db = f.tools.store.connection().unwrap();
        let point = crate::hbs_storage::read(&db, "note", &child.page_id).unwrap();
        crate::hbs_storage::test_sources::commit(
            &mut db,
            "note",
            &child.page_id,
            crate::hbs_storage::Commit {
                id: "keep-body".into(),
                expected_version: point["version"].as_i64().unwrap(),
                saved: point["saved"].clone(),
                baseline: point["saved"].clone(),
            },
        )
        .unwrap();
        // Reproduce the deployed stale metadata record, without importing private data.
        let old: String = db
            .query_row(
                "select candidate_json from agent_note_patch_candidates where page_id=?1",
                [&child.page_id],
                |r| r.get(0),
            )
            .unwrap();
        let source = crate::hbs_storage::read(&db, "note", &f.parent.to_string()).unwrap();
        let mut cut = source["saved"].clone();
        cut["blocks"]
            .as_array_mut()
            .unwrap()
            .retain(|b| b["content"]["targetId"] != child.page_id);
        crate::hbs_storage::test_sources::commit(
            &mut db,
            "note",
            &f.parent.to_string(),
            crate::hbs_storage::Commit {
                id: "cut".into(),
                expected_version: source["version"].as_i64().unwrap(),
                saved: cut.clone(),
                baseline: cut,
            },
        )
        .unwrap();
        db.execute("update agent_note_patch_candidates set candidate_json=?2,state='pending' where page_id=?1",params![child.page_id,old]).unwrap();
    }
    // This registered owner has no UI event transport: any normal acquisition
    // would fail. A fully settled, obsolete metadata decision needs no fence.
    let session = Uuid::new_v4();
    f.tools
        .store
        .text_editors()
        .register(session, Some(child.page_id.clone()))
        .unwrap();
    let request = note_review::Request {
        conversation_id: f.tools.record.conversation_id,
        proposal_id: shown.proposal_id.clone(),
        generation: shown.generation,
        request_id: Uuid::new_v4(),
        page_id: child.page_id.clone(),
        action: "keep".into(),
        chunk_id: None,
        document: None,
        history_request_id: None,
        direction: None,
        expected_revision: None,
    };
    let before = note::load(&f.tools.store.connection().unwrap(), &child.page_id).unwrap();
    assert_eq!(before.status, "trashed");
    let receipt = note_review::resolve(f.tools.store.clone(), request.clone())
        .await
        .unwrap();
    assert!(receipt.pages.is_empty());
    assert_eq!(
        serde_json::to_value(
            note_review::resolve(f.tools.store.clone(), request)
                .await
                .unwrap()
        )
        .unwrap(),
        serde_json::to_value(receipt).unwrap()
    );
    let after = note::load(&f.tools.store.connection().unwrap(), &child.page_id).unwrap();
    assert_eq!(
        serde_json::to_value(before).unwrap(),
        serde_json::to_value(after).unwrap()
    );
}
