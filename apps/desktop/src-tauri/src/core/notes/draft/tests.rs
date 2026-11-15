use super::*;

fn request(id: &str, source: Option<Value>) -> DraftCommit {
    let document = json!({"schemaVersion":1,"pageId":id,"title":"First title","blocks":[]});
    serde_json::from_value(json!({"commit":{"id":"first-publication","expectedVersion":0,"saved":document,"baseline":document},"favorite":true,"source":source})).unwrap()
}
fn parent(db: &mut Connection) -> super::super::model::NoteDocumentSnapshot {
    super::super::page_creation::create_page_from(
        db,
        super::super::NotePageDraft {
            title: "Parent".into(),
            parent_page_id: None,
            parent_block_position: None,
        },
    )
    .unwrap()
}
fn source(page: &super::super::model::NoteDocumentSnapshot, target: &str, revision: i64) -> Value {
    let mut document = serde_json::to_value(&page.document).unwrap();
    document["blocks"].as_array_mut().unwrap().push(json!({"id":"source-link","type":"researchObjectReference","props":{},"children":[],"content":{"kind":"reference","targetKind":"note_page","targetId":target,"labelSnapshot":"First title","quoteSnapshot":""}}));
    json!({"document":document,"expectedRevision":revision,"saveReason":"structure"})
}

#[test]
fn first_publication_commits_content_baseline_favorite_and_source_link_once() {
    let mut db = super::super::tests::migrated_connection();
    let parent = parent(&mut db);
    let input = source(&parent, "draft-page", parent.page.revision);
    assert!(super::super::document_repository::load_document_from(&db, "draft-page").is_err());
    let first = publish(
        &mut db,
        "draft-page",
        request("draft-page", Some(input.clone())),
    )
    .unwrap();
    assert_eq!(first["page"]["page"]["title"], "First title");
    assert_eq!(first["page"]["page"]["isFavorite"], true);
    assert_eq!(first["snapshot"]["version"], 1);
    assert_eq!(first["snapshot"]["saved"], first["snapshot"]["baseline"]);
    let second = publish(&mut db, "draft-page", request("draft-page", Some(input))).unwrap();
    assert_eq!(first, second);
    let saved =
        super::super::document_repository::load_document_from(&db, &parent.page.id).unwrap();
    assert_eq!(saved.page.revision, parent.page.revision + 1);
    assert_eq!(
        saved
            .document
            .blocks
            .iter()
            .filter(|block| block.id == "source-link")
            .count(),
        1
    );
}

#[test]
fn stale_source_rolls_back_page_first_save_favorite_and_receipt() {
    let mut db = super::super::tests::migrated_connection();
    let parent = parent(&mut db);
    let input = source(&parent, "rejected-draft", 99);
    assert!(
        publish(
            &mut db,
            "rejected-draft",
            request("rejected-draft", Some(input))
        )
        .is_err()
    );
    assert!(super::super::document_repository::load_document_from(&db, "rejected-draft").is_err());
    let count: i64 = db
        .query_row(
            "select count(*) from hbs_commit_receipts where resource_id='rejected-draft'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
    assert_eq!(
        super::super::document_repository::load_document_from(&db, &parent.page.id)
            .unwrap()
            .page
            .revision,
        parent.page.revision
    );
}

#[test]
fn first_publication_never_overwrites_an_existing_page_or_reuses_a_request_for_other_options() {
    let mut db = super::super::tests::migrated_connection();
    let parent = parent(&mut db);
    assert!(publish(&mut db, &parent.page.id, request(&parent.page.id, None)).is_err());
    publish(&mut db, "new-draft", request("new-draft", None)).unwrap();
    let mut reused = request("new-draft", None);
    reused.favorite = false;
    assert!(
        publish(&mut db, "new-draft", reused)
            .unwrap_err()
            .contains("MODEL_REQUEST_ID_REUSED")
    );
    assert!(
        super::super::document_repository::load_document_from(&db, "new-draft")
            .unwrap()
            .page
            .is_favorite
    );
}

#[test]
fn empty_creation_establishes_a_canonical_page_and_baseline_before_input() {
    let mut db = super::super::tests::migrated_connection();
    let mut input = request("empty-draft", None);
    input.commit.saved["title"] = json!("");
    input.commit.baseline["title"] = json!("");
    let first = publish(&mut db, "empty-draft", input).unwrap();
    assert_eq!(first["page"]["page"]["title"], "无标题");
    assert_eq!(first["page"]["page"]["isFavorite"], true);
    assert_eq!(first["snapshot"]["version"], 1);
    assert_eq!(first["snapshot"]["saved"], first["snapshot"]["baseline"]);
    let loaded = super::super::document_repository::load_document_from(&db, "empty-draft").unwrap();
    assert_eq!(loaded.page.title, "无标题");
}
