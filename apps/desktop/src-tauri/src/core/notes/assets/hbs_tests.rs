use rusqlite::Connection;
use serde_json::{Value, json};
use std::path::Path;

const NOTE: &str = "00000000-0000-4000-8000-000000000186";

fn fixture_assets(db: &mut Connection, objects: &Path) -> Result<Value, String> {
    use crate::notes::assets::{NoteAssetImportRequest, NoteAssetManager, import_note_asset_at};
    use base64::Engine;
    // Fixed synthetic bytes exercise the original Core asset importer.
    let png = base64::engine::general_purpose::STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aGN8AAAAASUVORK5CYII=")
        .map_err(|error| error.to_string())?;
    let manager = NoteAssetManager::default();
    let image = import_note_asset_at(
        db,
        objects,
        &manager,
        NoteAssetImportRequest {
            display_name: "HBS 合成图片.png".into(),
            mime_type: "image/png".into(),
            bytes: png,
        },
    )
    .map_err(|error| error.to_string())?;
    let file = import_note_asset_at(
        db,
        objects,
        &manager,
        NoteAssetImportRequest {
            display_name: "HBS 合成附件.txt".into(),
            mime_type: "text/plain".into(),
            bytes: b"Synthetic HBS attachment\n".to_vec(),
        },
    )
    .map_err(|error| error.to_string())?;
    Ok(json!({"image":image,"file":file}))
}

#[test]
fn example_assets_use_original_importer_and_note_repository() {
    let mut db = Connection::open_in_memory().expect("database");
    crate::storage::migrate(&mut db).expect("migrations");
    let objects = std::env::temp_dir().join(format!("hbs-assets-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&objects).expect("objects");
    let assets = fixture_assets(&mut db, &objects).expect("import");
    assert_eq!(
        assets,
        fixture_assets(&mut db, &objects).expect("deduplicate")
    );
    let blocks: Vec<Value> = ["image", "file"].into_iter().map(|kind| {
        let asset = &assets[kind];
        let bytes = crate::notes::assets::read_note_asset_at(&db, &objects, asset["id"].as_str().expect("id")).expect("read original asset");
        assert!(!bytes.is_empty());
        json!({"id":format!("sample-{kind}"),"type":kind,"props":{},"children":[],"content":{
            "kind":"asset","assetId":asset["id"],"displayName":asset["displayName"],"mimeType":asset["mimeType"],"caption":"synthetic"}})
    }).collect();
    let document = serde_json::from_value(json!({"schemaVersion":1,"pageId":NOTE,"blocks":blocks}))
        .expect("note document");
    crate::notes::hbs::seed(&mut db, NOTE, document).expect("original note repository");
    let count: i64 = db
        .query_row(
            "select count(*) from note_blocks where page_id=?1 and block_type in ('image','file')",
            [NOTE],
            |row| row.get(0),
        )
        .expect("stored blocks");
    assert_eq!(count, 2);
    let count: i64 = db
        .query_row(
            "select count(*) from hbs_note_baseline_assets where page_id=?1",
            [NOTE],
            |row| row.get(0),
        )
        .expect("baseline assets");
    assert_eq!(count, 2);
    std::fs::remove_dir_all(objects).expect("cleanup");
}
