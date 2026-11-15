use super::*;

#[tokio::test]
async fn attachment_import_saves_relations_keeps_review_pins_and_rechecks_origin_before_keep() {
    let mut f = fixture(true);
    std::fs::create_dir_all(f.tools.store.content_directory()).unwrap();
    let bytes = b"synthetic authorized attachment".to_vec();
    let asset = crate::notes::assets::import_note_asset_from(
        &f.tools.store,
        &crate::notes::NoteAssetManager::default(),
        crate::notes::assets::NoteAssetImportRequest {
            bytes,
            display_name: "source.txt".into(),
            mime_type: "text/plain".into(),
        },
    )
    .unwrap();
    {
        let mut db = f.tools.store.connection().unwrap();
        db.execute("insert into bibliographic_items(id,item_type,title,created_at,updated_at) values('paper','document','Source','now','now')", []).unwrap();
        db.execute("insert into attachments(id,item_id,content_sha256,display_name,created_at) values('attachment','paper',?1,'source.txt','now')", [&asset.id]).unwrap();
        let mut input = f.tools.record.input.clone();
        input.context.extend(
            resource_grants::grants(
                &db,
                &[Resource::Paper {
                    resource_id: "paper".into(),
                    attachment_id: Some("attachment".into()),
                    page_index: None,
                }],
                Some(crate::agent::writing_resources::test_sources::source()),
            )
            .unwrap(),
        );
        let tx = db.transaction().unwrap();
        f.tools.record =
            submissions::insert_in(&tx, f.tools.record.conversation_id, Uuid::new_v4(), input)
                .unwrap();
        tx.commit().unwrap();
    }
    let target = path(f.page);
    read(&f, &target).await;
    let plain = f
        .tools
        .apply(
            "plain-link",
            &patch(&target, "old", "[Source](https://example.com/source.txt)"),
        )
        .await
        .unwrap();
    assert_eq!(plain["targets"][0]["status"], "applied");
    assert_eq!(
        f.tools
            .store
            .connection()
            .unwrap()
            .query_row("select count(*) from agent_note_patch_assets", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
        0
    );
    let shown = review(&f);
    decisions::decide(&f.tools.store, decision(&f, &shown, "reject")).unwrap();
    read(&f, &target).await;
    let patch = patch(
        &target,
        "old",
        "<file src=\"import://attachment\" name=\"source.txt\" mime=\"text/plain\" caption=\"Source\"/>",
    );
    let imported = f.tools.apply("import", &patch).await.unwrap();
    assert_eq!(imported["targets"][0]["status"], "applied", "{imported}");
    let shown = review(&f);
    {
        let db = f.tools.store.connection().unwrap();
        assert_eq!(
            db.query_row("select count(*) from note_block_assets", [], |row| row
                .get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            db.query_row("select count(*) from agent_note_patch_assets", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
            1
        );
        db.execute(
            "update bibliographic_items set trashed_at='now' where id='paper'",
            [],
        )
        .unwrap();
    }
    assert!(decisions::decide(&f.tools.store, decision(&f, &shown, "accept")).is_err());
    f.tools
        .store
        .connection()
        .unwrap()
        .execute(
            "update bibliographic_items set trashed_at=null where id='paper'",
            [],
        )
        .unwrap();
    let accepted = decisions::decide(&f.tools.store, decision(&f, &shown, "accept")).unwrap();
    assert_eq!(
        accepted.pages[0].document.blocks[0].content["assetId"],
        asset.id
    );
    assert_eq!(
        accepted.pages[0].document.blocks[0].content["displayName"],
        "source.txt"
    );
    assert_eq!(
        accepted.pages[0].document.blocks[0].content["mimeType"],
        "text/plain"
    );
    assert_eq!(
        f.tools
            .store
            .connection()
            .unwrap()
            .query_row("select count(*) from note_block_assets", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    decisions::decide(&f.tools.store, decision(&f, &shown, "undo")).unwrap();
    assert_eq!(
        f.tools
            .store
            .connection()
            .unwrap()
            .query_row("select count(*) from note_block_assets", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn imported_image_uses_importer_identity_name_and_mime_in_the_accepted_document() {
    let mut f = fixture(true);
    std::fs::create_dir_all(f.tools.store.content_directory()).unwrap();
    let asset = crate::notes::assets::import_note_asset_from(
        &f.tools.store,
        &crate::notes::NoteAssetManager::default(),
        crate::notes::assets::NoteAssetImportRequest {
            bytes: b"synthetic authorized image".to_vec(),
            display_name: "original-screenshot.jpeg".into(),
            mime_type: "image/jpeg".into(),
        },
    )
    .unwrap();
    {
        let mut db = f.tools.store.connection().unwrap();
        db.execute("insert into bibliographic_items(id,item_type,title,created_at,updated_at) values('paper','document','Image source','now','now')", []).unwrap();
        db.execute("insert into attachments(id,item_id,content_sha256,display_name,created_at) values('image-attachment','paper',?1,'original-screenshot.jpeg','now')", [&asset.id]).unwrap();
        let mut input = f.tools.record.input.clone();
        input.context.extend(
            resource_grants::grants(
                &db,
                &[Resource::Paper {
                    resource_id: "paper".into(),
                    attachment_id: Some("image-attachment".into()),
                    page_index: None,
                }],
                Some(crate::agent::writing_resources::test_sources::source()),
            )
            .unwrap(),
        );
        let tx = db.transaction().unwrap();
        f.tools.record =
            submissions::insert_in(&tx, f.tools.record.conversation_id, Uuid::new_v4(), input)
                .unwrap();
        tx.commit().unwrap();
    }
    let target = path(f.page);
    read(&f, &target).await;
    let proposed = patch(
        &target,
        "old",
        "<image src=\"import://image-attachment\" name=\"original-screenshot.jpeg\" mime=\"image/jpeg\" caption=\"Evidence\"/>",
    );
    let result = f.tools.apply("import-image", &proposed).await.unwrap();
    assert_eq!(result["targets"][0]["status"], "applied", "{result}");
    let shown = review(&f);
    let accepted = decisions::decide(&f.tools.store, decision(&f, &shown, "accept")).unwrap();
    let block = &accepted.pages[0].document.blocks[0];
    assert_eq!(block.block_type, "image");
    assert_eq!(block.content["assetId"], asset.id);
    assert_eq!(block.content["displayName"], format!("{}.jpg", asset.id));
    assert_eq!(block.content["mimeType"], "image/jpeg");
    assert_eq!(block.content["caption"], "Evidence");
}
