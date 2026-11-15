use super::*;
fn checkpoint(kind: &str, snapshot: &Value, content: Value) -> Value {
    json!({"format":1,"kind":kind,"observed":snapshot,"content":content,"ancestor":snapshot["saved"],"baselineAncestor":snapshot["baseline"],"ordinary":[]})
}
#[test]
fn upgrade_189_and_reopen_keep_preservation_even_when_s_rejects_save() {
    let path =
        std::env::temp_dir().join(format!("hbs-preservation-{}.sqlite3", uuid::Uuid::new_v4()));
    let candidate;
    {
        let mut db = Connection::open(&path).unwrap();
        crate::storage_migration_tests::migrate_through(&mut db, 189);
        db.execute(
            "insert into hbs_text_documents(id,content,baseline) values('t','saved','base')",
            [],
        )
        .unwrap();
        crate::storage::migrate(&mut db).unwrap();
        let before = read(&db, "text", "t").unwrap();
        candidate = checkpoint("text", &before, json!("edited new survives"));
        assert_eq!(
            preservation_save(&mut db, "text", "t", candidate.clone()).unwrap()["revision"],
            1
        );
        let result = commit(
            &mut db,
            "text",
            "t",
            request("conflict", 99, json!("candidate"), json!("base")),
        )
        .unwrap();
        assert_eq!(result["type"], "conflict");
        assert_eq!(read(&db, "text", "t").unwrap(), before);
    }
    let mut db = Connection::open(&path).unwrap();
    crate::storage::migrate(&mut db).unwrap();
    assert_eq!(preservation_read(&db, "text", "t").unwrap(), candidate);
    assert_eq!(
        preservation_save(&mut db, "text", "t", candidate.clone()).unwrap()["revision"],
        2
    );
    let mut bad = candidate.clone();
    bad["kind"] = json!("note");
    assert!(preservation_save(&mut db, "text", "t", bad).is_err());
    assert_eq!(preservation_read(&db, "text", "t").unwrap(), candidate);
    let inspected = inspection::read(&mut db, "text", "t").unwrap();
    assert_eq!(inspected["storage"]["S"][0]["rows"][0]["content"], "saved");
    assert!(inspected["storage"]["S′"][0]["rows"][0]["checkpoint_json"].is_string());
    drop(db);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn saved_candidate_review_origin_survives_reopen_and_rejects_invalid_update() {
    let path = std::env::temp_dir().join(format!(
        "hbs-review-origin-{}.sqlite3",
        uuid::Uuid::new_v4()
    ));
    let expected;
    {
        let mut db = Connection::open(&path).unwrap();
        crate::storage::migrate(&mut db).unwrap();
        db.execute(
            "insert into hbs_text_documents(id,content,baseline) values('t','91%','80%')",
            [],
        )
        .unwrap();
        let snapshot = read(&db, "text", "t").unwrap();
        let mut value = checkpoint("text", &snapshot, json!("91%"));
        value["reviewAncestor"] = json!("85%");
        preservation_save(&mut db, "text", "t", value.clone()).unwrap();
        let mut bad = value.clone();
        bad["reviewAncestor"] = json!({"invalid":"text document"});
        assert!(preservation_save(&mut db, "text", "t", bad).is_err());
        assert_eq!(preservation_read(&db, "text", "t").unwrap(), value);
        expected = value;
    }
    let mut db = Connection::open(&path).unwrap();
    crate::storage::migrate(&mut db).unwrap();
    assert_eq!(preservation_read(&db, "text", "t").unwrap(), expected);
    drop(db);
    std::fs::remove_file(path).unwrap();
}
#[test]
fn note_preservation_pins_real_assets_and_failed_update_rolls_back() {
    use crate::notes::assets::{NoteAssetImportRequest, NoteAssetManager, import_note_asset_at};
    let mut db = database();
    let id = "00000000-0000-4000-8000-000000000186";
    let before =
        crate::notes::hbs::seed(&mut db, id, serde_json::from_value(note()).unwrap()).unwrap();
    let directory =
        std::env::temp_dir().join(format!("hbs-preserved-assets-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&directory).unwrap();
    let asset = import_note_asset_at(
        &mut db,
        &directory,
        &NoteAssetManager::default(),
        NoteAssetImportRequest {
            display_name: "candidate.txt".into(),
            mime_type: "text/plain".into(),
            bytes: b"candidate".to_vec(),
        },
    )
    .unwrap();
    let mut content = note();
    content["blocks"][0]["type"] = json!("file");
    content["blocks"][0]["content"] = json!({"kind":"asset","assetId":asset.id,"displayName":"candidate.txt","mimeType":"text/plain","caption":"only in S-prime"});
    let candidate = checkpoint("note", &before, content);
    preservation_save(&mut db, "note", id, candidate.clone()).unwrap();
    let pins:i64=db.query_row("select count(*) from content_object_references where owner_kind='hbs_preservation' and content_sha256=?1",[&asset.id],|row|row.get(0)).unwrap();
    assert_eq!(pins, 1);
    let mut bad = candidate.clone();
    bad["content"]["blocks"][0]["content"]["assetId"] = json!("c".repeat(64));
    assert!(preservation_save(&mut db, "note", id, bad).is_err());
    assert_eq!(preservation_read(&db, "note", id).unwrap(), candidate);
    assert_eq!(read(&db, "note", id).unwrap(), before);
    let inspected = inspection::read(&mut db, "note", id).unwrap();
    assert_eq!(inspected["storage"]["S"][1]["table"], "note_blocks");
    let raw = inspected["storage"]["S"][1]["rows"][0]["content_json"]
        .as_str()
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(raw).unwrap(),
        note()["blocks"][0]["content"]
    );
    preservation::clear(&mut db, "note", id).unwrap();
    assert!(preservation_read(&db, "note", id).unwrap().is_null());
    let pins: i64 = db
        .query_row("select count(*) from hbs_preservation_assets", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(pins, 0);
    std::fs::remove_dir_all(directory).unwrap();
}
