use crate::structured_resources as resources;
use rusqlite::{Connection, params};
use serde_json::json;

fn checksums(db: &Connection) -> Vec<(i64, String)> {
    db.prepare("select version,sha256 from migration_checksums where version<=176 order by version")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
}

#[test]
fn resource_upgrade_preserves_manual_content_old_checksums_and_reopens_repair_drafts() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("upgrade.sqlite3");
    let mut db = Connection::open(&path).unwrap();
    crate::storage_migration_tests::migrate_through(&mut db, 176);
    crate::storage_integrity::backfill_migration_checksums(&db).unwrap();
    let before = checksums(&db);
    db.execute_batch("insert into bibliographic_items(id,item_type,title,created_at,updated_at) values('paper','journalArticle','Paper','now','now');
      insert into structured_reading_units(id,paper_id,unit_kind,title,body,epistemic_role,created_at,updated_at) values('unit','paper','research_question','Question','手工正文必须保留。','question','now','now');").unwrap();
    crate::storage::migrate(&mut db).unwrap();
    let identities = resources::ensure_paper_resources(&db, "paper")
        .unwrap()
        .into_iter()
        .map(|row| row.id)
        .collect::<Vec<_>>();
    assert_eq!(identities.len(), 3);
    let catalog = json!({"templates":[
        {"id":"custom-a","label":"同名模板","fields":[{"id":"question","title":"问题","question":"回答哪一个问题？"}]},
        {"id":"custom-b","label":"同名模板","fields":[{"id":"result","title":"结果","question":"取得什么结果？"}]}
    ]});
    let input = resources::LegacyImportDraft {
        request_id: "import".into(),
        catalog: Some(catalog),
        paper_id: Some("paper".into()),
        presentation: None,
    };
    resources::import_legacy_with_evidence(
        &crate::hbs_storage::test_sources::ReadingEvidence,
        &mut db,
        input.clone(),
    )
    .unwrap();
    let reading = resources::find_by_paper(&db, "paper", "structuredReading").unwrap();
    assert!(
        reading
            .content
            .as_ref()
            .unwrap()
            .to_string()
            .contains("手工正文必须保留。")
    );
    resources::import_legacy_with_evidence(
        &crate::hbs_storage::test_sources::ReadingEvidence,
        &mut db,
        input,
    )
    .unwrap();
    assert_eq!(
        resources::get(&db, &reading.id).unwrap().revision,
        reading.revision
    );
    let editable = crate::hbs_storage::test_sources::write(
        &mut db,
        resources::WriteDraft {
            resource_id: reading.id.clone(),
            expected_revision: reading.revision,
            content: reading.content.as_ref().unwrap().to_string(),
            request_id: "manual-after-upgrade".into(),
            conversation_id: None,
            expected_draft_revision: None,
            source_snapshot_id: None,
            submission_id: None,
            editor_kind: "human".into(),
        },
    )
    .unwrap();
    assert_eq!(
        editable.status, "applied",
        "imported manual content must remain editable: {:?}",
        editable.errors
    );
    let global = resources::list_headers(&db, None)
        .unwrap()
        .into_iter()
        .filter(|row| {
            row.kind == "readingTemplate" && row.paper_id.is_none() && row.title == "同名模板"
        })
        .collect::<Vec<_>>();
    assert_eq!(
        global.len(),
        2,
        "equal names must not collapse different template definitions"
    );
    let target = &global[0];
    let invalid = crate::hbs_storage::test_sources::write(
        &mut db,
        resources::WriteDraft {
            resource_id: target.id.clone(),
            expected_revision: target.revision,
            content: "{not JSON".into(),
            request_id: "invalid-draft".into(),
            conversation_id: Some("conversation".into()),
            expected_draft_revision: None,
            source_snapshot_id: None,
            submission_id: Some("first-turn".into()),
            editor_kind: "agent".into(),
        },
    )
    .unwrap();
    assert_eq!(invalid.status, "invalid");
    let pending = invalid.draft.unwrap();
    assert_eq!(checksums(&db), before);
    drop(db);

    let mut db = Connection::open(&path).unwrap();
    crate::storage::migrate(&mut db).unwrap();
    assert_eq!(checksums(&db), before);
    assert_eq!(
        resources::ensure_paper_resources(&db, "paper")
            .unwrap()
            .into_iter()
            .map(|row| row.id)
            .collect::<Vec<_>>(),
        identities
    );
    let reopened = resources::read_text(&db, &target.id, Some("conversation"), false).unwrap();
    assert_eq!(reopened.text, "{not JSON");
    assert_eq!(reopened.draft.unwrap().revision, pending.revision);
    assert!(
        !resources::read_text(&db, &target.id, Some("other-conversation"), false)
            .unwrap()
            .text
            .contains("{not JSON")
    );
    assert_eq!(
        db.query_row(
            "select body from structured_reading_units where id='unit'",
            [],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        "手工正文必须保留。"
    );
    assert_eq!(
        db.prepare("pragma foreign_key_check")
            .unwrap()
            .query_map([], |_| Ok(()))
            .unwrap()
            .count(),
        0
    );
    assert_eq!(
        db.query_row(
            "select count(*) from structured_resource_write_intents where request_id=?1",
            params!["invalid-draft"],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
}
