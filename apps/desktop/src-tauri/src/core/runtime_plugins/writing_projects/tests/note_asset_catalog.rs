//! Source-scoped assets through the real save and catalog preparation owners.
use super::{
    file_save::{note, request},
    files::{seed, store},
    *,
};
use crate::runtime_plugins::writing_projects::files::{
    FileRequest, FileResult, catalog_preparation::prepare_catalog, note_assets::Call, operate_files,
};
use research_workbench_native_sdk::writing_catalog::PreparationScope;
use std::fs;

#[tokio::test]
#[ignore = "requires the actual independently built writing v2 package"]
async fn actual_package_uuid_assets_survive_catalog_prepare_save_retry_and_reopen() {
    let f = Fixture::new();
    let store = store(&f);
    seed(&store, &f.active, "main.tex", "Original source");
    let FileResult::NoteAsset(asset) = operate_files(
        store.clone(),
        f.service.clone(),
        f.catalog.clone(),
        f.key.clone(),
        f.active.clone(),
        FileRequest::NoteAsset {
            call: Call::Import {
                display_name: "source.txt".into(),
                mime_type: "text/plain".into(),
                bytes: b"Synthetic attachment".to_vec(),
            },
        },
    )
    .await
    .unwrap() else {
        panic!("asset receipt")
    };
    let mut document: serde_json::Value =
        serde_json::from_str(&note("stable-assets", "Asset catalog body")).unwrap();
    document["blocks"].as_array_mut().unwrap().push(serde_json::json!({
        "id": "file", "type": "file", "props": {},
        "content": {"kind": "asset", "assetId": asset.asset_id,
            "displayName": asset.display_name, "mimeType": asset.mime_type, "caption": "Evidence"},
        "children": []
    }));
    let content = document.to_string();
    let path = "notes/assets.ranote";
    seed(&store, &f.active, path, &content);
    for retry in [false, true] {
        if retry {
            store.connection().unwrap().execute(
                "update writing_note_catalog set state='error',error_cause='synthetic prior error'
                 where manuscript_id=?1 and relative_path=?2",
                rusqlite::params![f.active, path],
            ).unwrap();
        }
        let prepared = prepare_catalog(
            store.clone(),
            f.service.clone(),
            f.catalog.clone(),
            f.key.clone(),
            retry,
            PreparationScope::Active,
        )
        .await
        .unwrap();
        assert_eq!((prepared.processed_count, prepared.error_count), (1, 0));
        let (id, state, body): (String, String, String) = store
            .connection()
            .unwrap()
            .query_row(
                "select note_id,state,search_text from writing_note_catalog
             join writing_note_catalog_body using(manuscript_id,relative_path)
             where manuscript_id=?1 and relative_path=?2",
                rusqlite::params![f.active, path],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!((id.as_str(), state.as_str()), ("stable-assets", "ready"));
        assert!(body.contains("Asset catalog body"));
        assert!(body.contains("Evidence"));
    }
    document["blocks"][0]["content"]["items"][0]["text"] = "Updated asset note".into();
    let updated = document.to_string();
    let saved = operate_files(
        store.clone(),
        f.service.clone(),
        f.catalog.clone(),
        f.key.clone(),
        f.active.clone(),
        FileRequest::Save {
            request: request(
                &f.active,
                path,
                &updated,
                Some(&crate::scoped_text_files::legacy::sha256(
                    content.as_bytes(),
                )),
            ),
        },
    )
    .await
    .unwrap();
    assert!(matches!(saved, FileResult::Read(_)));
    let body: String = store.connection().unwrap().query_row(
        "select search_text from writing_note_catalog_body where manuscript_id=?1 and relative_path=?2",
        rusqlite::params![f.active,path], |row|row.get(0),
    ).unwrap();
    assert!(body.contains("Updated asset note"));
    let asset_path = store
        .manuscript_directory()
        .join(&f.active)
        .join("source/.ranote-assets")
        .join(&asset.asset_id);
    assert_eq!(fs::read(&asset_path).unwrap(), b"Synthetic attachment");
    f.service.shutdown().await.unwrap();
    drop(store);
    let Fixture {
        directory,
        database,
        ..
    } = f;
    drop(database);
    let db = rusqlite::Connection::open(directory.path().join("synthetic.sqlite3")).unwrap();
    let reopened_body: String = db
        .query_row(
            "select search_text from writing_note_catalog_body",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(reopened_body, body);
    assert_eq!(fs::read(asset_path).unwrap(), b"Synthetic attachment");
}
