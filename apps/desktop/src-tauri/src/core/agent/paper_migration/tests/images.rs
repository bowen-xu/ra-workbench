use super::*;
use crate::ai::paper_context::agent_preview;
use sha2::{Digest, Sha256};
use std::collections::HashSet;

#[tokio::test]
async fn migrates_legacy_picture_context_into_the_submission_image_manifest() {
    let mut connection = rusqlite::Connection::open_in_memory().unwrap();
    crate::storage_migration_tests::migrate_through(&mut connection, 217);
    let pdf_hash = "a".repeat(64);
    let image_bytes = b"\x89PNG\r\n\x1a\nlegacy-picture";
    let image_hash = format!("{:x}", Sha256::digest(image_bytes));
    connection
        .execute(
            "insert into bibliographic_items(id,item_type,created_at,updated_at)
                 values('paper','document','now','now')",
            [],
        )
        .unwrap();
    connection
        .execute(
            "insert into llm_provider_configs(provider_id,enabled,base_url,timeout_ms,updated_at)
                 values('legacy-provider',1,'',60000,'now')",
            [],
        )
        .unwrap();
    connection
        .execute(
            "insert into content_objects(sha256,relative_path,mime_type,byte_size,created_at)
                 values(?1,'paper.pdf','application/pdf',1,'now'),
                       (?2,?3,'image/png',?4,'now')",
            params![
                pdf_hash,
                image_hash,
                format!(
                    "ai-reading-images/{}/{}/{}.png",
                    &image_hash[..2],
                    &image_hash[2..4],
                    image_hash
                ),
                image_bytes.len() as i64,
            ],
        )
        .unwrap();
    connection
        .execute(
            "insert into attachments(id,item_id,content_sha256,display_name,created_at)
                 values('attachment','paper',?1,'fixture.pdf','now')",
            [&pdf_hash],
        )
        .unwrap();
    connection
        .execute(
            "insert into docling_parse_runs(
                    id,item_id,attachment_id,content_sha256,service_base_url,docling_status,
                    parse_status,json_content,raw_response,created_at,finished_at)
                 values('parse','paper','attachment',?1,'http://127.0.0.1:1','success',
                    'success','{}','{}','now','now')",
            [&pdf_hash],
        )
        .unwrap();
    #[derive(serde::Serialize)]
    #[serde(rename_all = "camelCase")]
    struct SnapshotRect {
        x: f64,
        y: f64,
        width: f64,
        height: f64,
    }
    #[derive(serde::Serialize)]
    #[serde(rename_all = "camelCase")]
    struct SnapshotRegion {
        page_index: u32,
        rect: SnapshotRect,
    }
    let regions = vec![SnapshotRegion {
        page_index: 0,
        rect: SnapshotRect {
            x: 0.1,
            y: 0.2,
            width: 0.3,
            height: 0.4,
        },
    }];
    #[derive(serde::Serialize)]
    #[serde(rename_all = "camelCase")]
    struct SnapshotAttachment<'a> {
        attachment_id: &'a str,
        content_sha256: &'a str,
        parse_run_id: &'a str,
        ordinal: i64,
    }
    #[derive(serde::Serialize)]
    #[serde(rename_all = "camelCase")]
    struct SnapshotItem<'a> {
        id: &'a str,
        attachment_id: &'a str,
        content_sha256: &'a str,
        parse_run_id: &'a str,
        raw_ref: &'a str,
        kind: &'a str,
        label: Option<&'a str>,
        page_index: Option<u32>,
        regions: &'a [SnapshotRegion],
        charspan: Option<[u32; 2]>,
        source_text_hash: Option<&'a str>,
        text: Option<&'a str>,
        table_json: Option<serde_json::Value>,
        image_content_sha256: Option<&'a str>,
    }
    #[derive(serde::Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Canonical<'a> {
        context_schema_version: i64,
        paper_id: &'a str,
        attachments: Vec<SnapshotAttachment<'a>>,
        items: Vec<SnapshotItem<'a>>,
    }
    let canonical = Canonical {
        context_schema_version: 1,
        paper_id: "paper",
        attachments: vec![SnapshotAttachment {
            attachment_id: "attachment",
            content_sha256: &pdf_hash,
            parse_run_id: "parse",
            ordinal: 0,
        }],
        items: vec![SnapshotItem {
            id: "P1-F1",
            attachment_id: "attachment",
            content_sha256: &pdf_hash,
            parse_run_id: "parse",
            raw_ref: "#/pictures/0",
            kind: "picture",
            label: Some("figure"),
            page_index: Some(0),
            regions: &regions,
            charspan: None,
            source_text_hash: None,
            text: None,
            table_json: None,
            image_content_sha256: Some(&image_hash),
        }],
    };
    let context_hash = format!(
        "{:x}",
        Sha256::digest(serde_json::to_string(&canonical).unwrap().as_bytes())
    );
    let snapshot_id = format!("paper-context-{context_hash}");
    connection
        .execute(
            "insert into paper_context_snapshots(
                    id,paper_id,context_hash,context_schema_version,created_at)
                 values(?1,'paper',?2,1,'now')",
            params![&snapshot_id, &context_hash],
        )
        .unwrap();
    connection
        .execute(
            "insert into paper_context_attachments(
                    snapshot_id,ordinal,attachment_id,content_sha256,parse_run_id)
                 values(?1,0,'attachment',?2,'parse')",
            params![&snapshot_id, &pdf_hash],
        )
        .unwrap();
    connection
        .execute(
            "insert into paper_context_items(
                    snapshot_id,context_item_id,attachment_id,content_sha256,parse_run_id,
                    raw_ref,item_kind,label,page_index,regions_json,charspan_json,
                    source_text_hash,text_content,table_json,image_content_sha256,created_at)
                 values(?1,'P1-F1','attachment',?2,'parse','#/pictures/0','picture',
                    'figure',0,?3,null,null,null,null,?4,'now')",
            params![
                &snapshot_id,
                &pdf_hash,
                serde_json::to_string(&regions).unwrap(),
                image_hash
            ],
        )
        .unwrap();
    connection
        .execute(
            "insert into paper_ai_reading_sessions(
                    id,paper_id,context_snapshot_id,attachment_ids_json,parse_run_ids_json,
                    provider_id,model_id,title,status,created_at,updated_at)
                 values('legacy-picture','paper',?1,'[\"attachment\"]','[\"parse\"]',
                    'legacy-provider','legacy-model','Picture reading','archived',
                    '2026-01-01','2026-01-02')",
            [snapshot_id.as_str()],
        )
        .unwrap();
    connection
            .execute(
                "insert into paper_ai_reading_turns(
                    id,session_id,ordinal,user_message,assistant_message,messages_json,
                    provider_id,model_id,status,created_at,finished_at)
                 values('legacy-picture-turn','legacy-picture',0,'Explain this figure','It is a figure.',
                    '[{\"role\":\"user\",\"content\":\"Explain this figure\"},
                      {\"role\":\"assistant\",\"content\":\"It is a figure.\"}]',
                    'legacy-provider','legacy-model','succeeded','2026-01-01','2026-01-01T01:00:00Z')",
                [],
            )
            .unwrap();
    connection
        .execute(
            "insert into paper_ai_reading_turn_context_items(
                    id,turn_id,context_item_id,paper_id,attachment_id,content_sha256,
                    parse_run_id,raw_ref,item_kind,page_index,regions_json,
                    charspan_json,source_text_hash,image_content_sha256,created_at)
                 values('context-row','legacy-picture-turn','P1-F1','paper','attachment',?1,
                    'parse','#/pictures/0','picture',0,?2,null,null,?3,'now')",
            params![
                pdf_hash,
                serde_json::to_string(&regions).unwrap(),
                image_hash
            ],
        )
        .unwrap();

    let tempdir = tempfile::tempdir().unwrap();
    let store = crate::storage::AppStore::for_test_database(
        crate::data_interface::Database::open(connection, &tempdir.path().join("data-log"))
            .unwrap(),
        tempdir.path().join("objects"),
    );
    let image_path = store.content_directory().join(format!(
        "ai-reading-images/{}/{}/{}.png",
        &image_hash[..2],
        &image_hash[2..4],
        image_hash
    ));
    std::fs::create_dir_all(image_path.parent().unwrap()).unwrap();
    std::fs::write(image_path, image_bytes).unwrap();
    {
        let preview_db = store.connection().unwrap();
        let (_package_directory, catalog, package) =
            crate::runtime_plugins::literature_context::test_fixture::controlled_package();
        let preview = crate::runtime_plugins::literature_context::test_fixture::expected_preview(
            &catalog,
            &package,
            &preview_db,
            &agent_preview::Request {
                paper_id: "paper".into(),
                snapshot_id: snapshot_id.clone(),
                expected_context_hash: context_hash.clone(),
            },
        );
        agent_preview::submission_context_from_preview(preview.clone())
            .expect("picture snapshot should validate");
        agent_preview::submission_images_from_preview(
            preview,
            &preview_db,
            &store,
            Some(&HashSet::from(["P1-F1".to_owned()])),
        )
        .expect("picture bytes should validate");
    }
    {
        let stale_db = store.connection().unwrap();
        stale_db
            .execute(
                "update paper_context_snapshots set context_hash=?1 where id=?2",
                params!["b".repeat(64), &snapshot_id],
            )
            .unwrap();
    }
    let catalog = SqliteConversationCatalog::new(store.shared_agent_connection());
    let sessions = AgentSessionService::new(Arc::new(
        SqliteConversationStore::new(store.shared_agent_connection(), "新对话".into()).unwrap(),
    ));
    let legacy = load_legacy(&store, "paper").unwrap();

    assert!(
        migrate_session(&store, &catalog, &sessions, "paper", &legacy[0])
            .await
            .unwrap()
    );
    let db = store.connection().unwrap();
    let image_row: (String, String, i64) = db
        .query_row(
            "select source_id,content_sha256,byte_size from agent_submission_images",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert!(image_row.0.starts_with("paper-source:"));
    assert_eq!(image_row.1, image_hash);
    assert_eq!(image_row.2, image_bytes.len() as i64);
    let scope: (String, String) = db
        .query_row(
            "select paper_context_snapshot_id,context_hash
                 from agent_paper_submission_scopes",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_ne!(scope.0, snapshot_id);
    assert_eq!(scope.1, context_hash);
    drop(db);
    assert!(
        !migrate_session(&store, &catalog, &sessions, "paper", &legacy[0])
            .await
            .unwrap()
    );
    // Missing rows are indistinguishable from an originally empty manifest.
    // Either way a retry must not add newly materialized image references.
    let original_image: (String,String,String,i64,i64,String,String) = store.connection().unwrap().query_row(
        "select source_id,content_sha256,media_type,byte_size,ordinal,conversation_id,request_id from agent_submission_images", [],
        |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?,row.get(6)?)),
    ).unwrap();
    store
        .connection()
        .unwrap()
        .execute("delete from agent_submission_images", [])
        .unwrap();
    assert!(matches!(
        migrate_session(&store, &catalog, &sessions, "paper", &legacy[0]).await,
        Err(Error::Conflict)
    ));
    assert_eq!(
        store
            .connection()
            .unwrap()
            .query_row("select count(*) from agent_submission_images", [], |row| {
                row.get::<_, u64>(0)
            })
            .unwrap(),
        0
    );
    store.connection().unwrap().execute(
        "insert into agent_submission_images(source_id,content_sha256,media_type,byte_size,ordinal,conversation_id,request_id) values(?1,?2,?3,?4,?5,?6,?7)",
        params![original_image.0,original_image.1,original_image.2,original_image.3,original_image.4,original_image.5,original_image.6],
    ).unwrap();
    let wrong_hash = "c".repeat(64);
    store
        .connection()
        .unwrap()
        .execute(
            "update agent_submission_images set content_sha256=?1",
            [&wrong_hash],
        )
        .unwrap();
    assert!(matches!(
        migrate_session(&store, &catalog, &sessions, "paper", &legacy[0]).await,
        Err(Error::Conflict)
    ));
    let db = store.connection().unwrap();
    assert_eq!(
        db.query_row(
            "select content_sha256 from agent_submission_images",
            [],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        wrong_hash
    );
    db.execute(
        "update agent_submission_images set content_sha256=?1",
        [&image_hash],
    )
    .unwrap();
    db.execute(
        "update agent_paper_submission_scopes set context_hash=?1",
        [&wrong_hash],
    )
    .unwrap();
    drop(db);
    assert!(matches!(
        migrate_session(&store, &catalog, &sessions, "paper", &legacy[0]).await,
        Err(Error::Conflict)
    ));
    assert_eq!(
        store
            .connection()
            .unwrap()
            .query_row(
                "select context_hash from agent_paper_submission_scopes",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        wrong_hash
    );
    let wrong_snapshot = Uuid::new_v4().to_string();
    {
        let db = store.connection().unwrap();
        db.execute(
            "update agent_paper_submission_scopes set context_hash=?1",
            [&context_hash],
        )
        .unwrap();
        db.execute(
            "update agent_submissions set context_snapshot_id=?1",
            [&wrong_snapshot],
        )
        .unwrap();
    }
    assert!(matches!(
        migrate_session(&store, &catalog, &sessions, "paper", &legacy[0]).await,
        Err(Error::Conflict)
    ));
    assert_eq!(
        store
            .connection()
            .unwrap()
            .query_row(
                "select context_snapshot_id from agent_submissions",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        wrong_snapshot
    );
    store.shared_agent_connection().shutdown().unwrap();
}
