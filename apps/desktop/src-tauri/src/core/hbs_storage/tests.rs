use super::test_sources::{
    commit, preservation_read, preservation_save, review as review_structured,
    seed as seed_structured, write as write_structured,
};
use super::*;
#[path = "preservation_tests.rs"]
mod preservation_tests;
#[path = "production_tests.rs"]
mod production_tests;
fn database() -> Connection {
    let mut db = Connection::open_in_memory().unwrap();
    crate::storage::migrate(&mut db).unwrap();
    db
}
fn note() -> Value {
    json!({"schemaVersion":1,"pageId":"00000000-0000-4000-8000-000000000186","blocks":[
        {"id":"block-one","type":"paragraph","props":{},"content":{"kind":"inline","items":[{"type":"text","text":"原文","styles":[]}]},"children":[]}]})
}
fn request(id: &str, version: i64, saved: Value, baseline: Value) -> Commit {
    Commit {
        id: id.into(),
        expected_version: version,
        saved,
        baseline,
    }
}
#[test]
fn notes_use_original_blocks_and_atomic_baseline_with_durable_receipts() {
    let mut db = database();
    let id = "00000000-0000-4000-8000-000000000186";
    let first =
        crate::notes::hbs::seed(&mut db, id, serde_json::from_value(note()).unwrap()).unwrap();
    let baseline = first["baseline"].clone();
    let mut saved = baseline.clone();
    saved["blocks"][0]["content"]["items"][0]["text"] = json!("外部修改");
    let version = first["version"].as_i64().unwrap();
    let result = commit(
        &mut db,
        "note",
        id,
        request("one", version, saved.clone(), baseline.clone()),
    )
    .unwrap();
    assert_eq!(result["snapshot"]["saved"], saved);
    assert_eq!(result["snapshot"]["baseline"], baseline);
    assert_eq!(
        db.query_row(
            "select count(*) from note_blocks where page_id=?1",
            [id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        commit(
            &mut db,
            "note",
            id,
            request("one", version, saved.clone(), baseline.clone())
        )
        .unwrap(),
        result
    );
    assert!(
        commit(
            &mut db,
            "note",
            id,
            request("one", version, baseline.clone(), baseline.clone())
        )
        .unwrap_err()
        .contains("MODEL_REQUEST_ID_REUSED")
    );
    assert_eq!(
        commit(
            &mut db,
            "note",
            id,
            request("stale", version, baseline.clone(), baseline.clone())
        )
        .unwrap()["type"],
        "conflict"
    );
    let block_state: String = db
        .query_row(
            "select group_concat(content_json) from note_blocks where page_id=?1",
            [id],
            |r| r.get(0),
        )
        .unwrap();
    let kept = commit(
        &mut db,
        "note",
        id,
        request("keep", version + 1, saved.clone(), saved.clone()),
    )
    .unwrap();
    assert_eq!(kept["snapshot"]["version"], version + 2);
    assert_eq!(
        db.query_row(
            "select group_concat(content_json) from note_blocks where page_id=?1",
            [id],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        block_state
    );
    let mut bad = saved.clone();
    bad["pageId"] = json!("different");
    assert!(
        commit(
            &mut db,
            "note",
            id,
            request("bad", version + 2, baseline, bad)
        )
        .is_err()
    );
    assert_eq!(read(&db, "note", id).unwrap(), kept["snapshot"]);
}
#[test]
fn migration_upgrade_and_reopen_preserve_receipts() {
    let path = std::env::temp_dir().join(format!("hbs-upgrade-{}.sqlite3", uuid::Uuid::new_v4()));
    {
        let mut db = Connection::open(&path).unwrap();
        crate::storage_migration_tests::migrate_through(&mut db, 185);
        crate::storage::migrate(&mut db).unwrap();
        db.execute("insert into hbs_text_documents(id,revision,content,baseline) values('t',0,'old','old')",[]).unwrap();
        commit(
            &mut db,
            "text",
            "t",
            request("r", 0, json!("new"), json!("old")),
        )
        .unwrap();
    }
    let mut db = Connection::open(&path).unwrap();
    crate::storage::migrate(&mut db).unwrap();
    let receipt = commit(
        &mut db,
        "text",
        "t",
        request("r", 0, json!("new"), json!("old")),
    )
    .unwrap();
    assert_eq!(receipt["snapshot"]["version"], 1);
    assert_eq!(read(&db, "text", "t").unwrap()["saved"], "new");
    drop(db);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn failure_after_note_save_rolls_back_body_baseline_revision_and_receipt() {
    let mut db = database();
    let id = "00000000-0000-4000-8000-000000000186";
    let before =
        crate::notes::hbs::seed(&mut db, id, serde_json::from_value(note()).unwrap()).unwrap();
    let mut saved = before["saved"].clone();
    saved["blocks"][0]["content"]["items"][0]["text"] = json!("must roll back");
    let mut baseline = before["baseline"].clone();
    baseline["blocks"][0]["type"] = json!("managedImage");
    baseline["blocks"][0]["content"] = json!({"kind":"asset","assetId":"a".repeat(64),"displayName":"missing","mimeType":"image/png","caption":""});
    assert!(
        commit(
            &mut db,
            "note",
            id,
            request(
                "rollback",
                before["version"].as_i64().unwrap(),
                saved,
                baseline
            )
        )
        .is_err()
    );
    assert_eq!(read(&db, "note", id).unwrap(), before);
    assert_eq!(
        db.query_row(
            "select count(*) from hbs_commit_receipts where request_id='rollback'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}
#[test]
fn structured_authority_uses_real_content_baseline_and_validator() {
    let mut db = database();
    let fields: Vec<Value> = [
        "research_question",
        "approach",
        "key_result",
        "conclusion",
        "limitation",
        "open_question",
    ]
    .iter()
    .map(|field| json!({"field":field,"status":"not_found","statements":[]}))
    .collect();
    let value = json!({"kind":"paper_overview_unit_v1","schemaVersion":1,"profileId":"generic_overview_v1","fields":fields});
    let (id, before) = seed_structured(&mut db, "synthetic-paper", value).unwrap();
    let mut saved = before["saved"].clone();
    saved["fields"][0] = json!({"field":"research_question","status":"filled","statements":[{"id":"s1","text":"manual text","epistemicRole":"uncertain","evidenceItemIds":[]}]});
    let result = commit(
        &mut db,
        "structured",
        &id,
        request(
            "s",
            before["version"].as_i64().unwrap(),
            saved.clone(),
            before["baseline"].clone(),
        ),
    )
    .unwrap();
    assert_eq!(result["snapshot"]["saved"], saved);
    let raw: String = db
        .query_row(
            "select content_json from structured_resources where id=?1",
            [&id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(serde_json::from_str::<Value>(&raw).unwrap(), saved);
    let failed = commit(
        &mut db,
        "structured",
        &id,
        request(
            "invalid",
            result["snapshot"]["version"].as_i64().unwrap(),
            json!({"kind":"wrong"}),
            saved,
        ),
    );
    assert!(failed.unwrap_err().contains("MODEL_INVALID_DOCUMENT"));
    assert_eq!(read(&db, "structured", &id).unwrap(), result["snapshot"]);
    let inspected = inspection::read(&mut db, "structured", &id).unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(
            inspected["storage"]["B"][0]["rows"][0]["baseline_json"]
                .as_str()
                .unwrap()
        )
        .unwrap(),
        before["baseline"]
    );
    let checkpoint = json!({"format":1,"kind":"structured","observed":result["snapshot"],"content":result["snapshot"]["saved"],"ancestor":before["saved"],"baselineAncestor":before["baseline"],"ordinary":[]});
    preservation_save(&mut db, "structured", &id, checkpoint.clone()).unwrap();
    assert_eq!(
        preservation_read(&db, "structured", &id).unwrap(),
        checkpoint
    );
}
