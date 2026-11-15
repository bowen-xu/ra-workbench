use super::*;
use rusqlite::Connection;
use serde_json::json;
#[path = "tests/history.rs"]
mod history;
#[path = "tests/persistence.rs"]
mod persistence;
#[path = "tests/profiles.rs"]
mod profiles;
#[path = "tests/provenance.rs"]
mod provenance;
#[path = "tests/review.rs"]
mod review;
#[path = "tests/startup.rs"]
mod startup;
#[path = "tests/templates.rs"]
mod templates;
#[path = "tests/write_runtime.rs"]
mod write_runtime;

fn database() -> Connection {
    let mut db = Connection::open_in_memory().unwrap();
    crate::storage::migrate(&mut db).unwrap();
    db
}
fn template() -> String {
    json!({"kind":"reading_template_v1","schemaVersion":1,"name":"测试模板","fields":[{"id":"question","title":"问题","question":"研究回应的问题。"}]}).to_string()
}
fn paper(db: &Connection) -> Resource {
    db.execute("insert into bibliographic_items(id,item_type,title,created_at,updated_at) values('paper','journalArticle','Test','now','now')",[]).unwrap();
    find_by_paper(db, "paper", "structuredReading").unwrap()
}
fn reading(text: &str) -> String {
    json!({"kind":"paper_overview_unit_v1","schemaVersion":1,"profileId":"generic_overview_v1","fields":[
        {"field":"research_question","status":"filled","statements":[{"id":"question-1","text":text,"epistemicRole":"uncertain","evidenceItemIds":[]}]},
        {"field":"approach","status":"not_found","statements":[]},{"field":"key_result","status":"not_found","statements":[]},{"field":"conclusion","status":"not_found","statements":[]},{"field":"limitation","status":"not_found","statements":[]},{"field":"open_question","status":"not_found","statements":[]}
    ]}).to_string()
}
fn decision(
    db: &mut Connection,
    resource: &Resource,
    action: &str,
    scope: Option<&str>,
    ids: Option<Vec<&str>>,
) -> Resource {
    crate::hbs_storage::test_sources::review(
        db,
        ReviewDraft {
            resource_id: resource.id.clone(),
            expected_revision: resource.revision,
            request_id: uuid::Uuid::new_v4().to_string(),
            action: action.into(),
            scope: scope.map(str::to_owned),
            field_ids: ids.map(|ids| ids.into_iter().map(str::to_owned).collect()),
        },
    )
    .unwrap()
}
fn draft(id: &str, content: &str, request: &str) -> WriteDraft {
    WriteDraft {
        resource_id: id.into(),
        expected_revision: 0,
        content: content.into(),
        request_id: request.into(),
        conversation_id: Some("test-conversation".into()),
        expected_draft_revision: None,
        source_snapshot_id: None,
        submission_id: Some("submission".into()),
        editor_kind: "agent".into(),
    }
}

#[test]
fn invalid_raw_content_is_durable_idempotent_and_patchable_before_template_approval() {
    let mut db = database();
    let target = create_template(&db, "测试模板", "create", Some("test-conversation")).unwrap();
    let input = draft(&target.id, "{invalid", "write-invalid");
    let result = crate::hbs_storage::test_sources::write(&mut db, input.clone()).unwrap();
    assert_eq!(result.status, "invalid");
    assert_eq!(get(&db, &target.id).unwrap().revision, 0);
    assert_eq!(
        crate::hbs_storage::test_sources::write(&mut db, input)
            .unwrap()
            .draft
            .unwrap()
            .revision,
        1
    );
    let saved = read_text(&db, &target.id, Some("test-conversation"), false).unwrap();
    assert_eq!(saved.text, "{invalid");
    let pending = crate::hbs_storage::test_sources::patch(
        &mut db,
        PatchDraft {
            resource_id: target.id.clone(),
            draft_id: result.draft.unwrap().id,
            expected_revision: 0,
            expected_draft_revision: 1,
            request_id: "patch".into(),
            edits: vec![TextEdit {
                old_text: "{invalid".into(),
                new_text: template(),
            }],
            conversation_id: Some("test-conversation".into()),
            submission_id: Some("submission".into()),
        },
    )
    .unwrap();
    assert_eq!(pending.status, "applied");
    assert!(pending.draft.is_none());
    assert!(!pending.resource.pending_field_ids.is_empty());
    assert_eq!(get(&db, &target.id).unwrap().revision, 1);
    assert!(
        read_text(&db, &target.id, Some("test-conversation"), false)
            .unwrap()
            .draft
            .is_none()
    );
}

#[test]
fn paper_creation_preallocates_stable_uuid_resources_without_empty_formal_payloads() {
    let db = database();
    db.execute("insert into bibliographic_items(id,item_type,title,created_at,updated_at) values('paper','journalArticle','Test','now','now')",[]).unwrap();
    let before = ensure_paper_resources(&db, "paper").unwrap();
    assert_eq!(before.len(), 3);
    for resource in &before {
        uuid::Uuid::parse_str(&resource.id).unwrap();
        assert_eq!(resource.revision, 0);
        assert!(resource.content.is_none());
        assert_eq!(read_text(&db, &resource.id, None, false).unwrap().text, "");
        assert_eq!(
            resource.source_uri,
            format!("source://library/paper/{}", resource.id)
        );
    }
    assert_eq!(
        before.iter().map(|r| &r.id).collect::<Vec<_>>(),
        ensure_paper_resources(&db, "paper")
            .unwrap()
            .iter()
            .map(|r| &r.id)
            .collect::<Vec<_>>()
    );
}

#[test]
fn portrait_coverage_type_error_preserves_the_draft_with_an_exact_path() {
    let mut db = database();
    paper(&db);
    let target = find_by_paper(&db, "paper", "portrait").unwrap();
    let raw = json!({"kind":"paper_portrait_v1","schemaVersion":1,
        "coverage":{"pages":[1]},"claims":[],"template":[]})
    .to_string();
    let result =
        crate::hbs_storage::test_sources::write(&mut db, draft(&target.id, &raw, "bad-coverage"))
            .unwrap();
    assert_eq!(result.status, "invalid");
    assert_eq!(result.errors[0].path, "$.coverage");
    assert_eq!(result.errors[0].code, "invalid_type");
    assert_eq!(
        read_text(&db, &target.id, Some("test-conversation"), false)
            .unwrap()
            .text,
        raw
    );
    assert!(get(&db, &target.id).unwrap().content.is_none());
}

#[test]
fn stale_template_write_retains_repair_without_replacing_current_content() {
    let mut db = database();
    let target = create_template(&db, "测试", "create", None).unwrap();
    let input = draft(&target.id, &template(), "first");
    let first = crate::hbs_storage::test_sources::write(&mut db, input.clone()).unwrap();
    assert_eq!(first.status, "applied");
    let mut stale = input;
    stale.content = template().replace("研究回应的问题。", "新的说明。");
    stale.request_id = "stale".into();
    let result = crate::hbs_storage::test_sources::write(&mut db, stale).unwrap();
    assert_eq!(result.status, "conflict");
    assert!(result.draft.is_some());
    assert_eq!(
        get(&db, &target.id).unwrap().content,
        first.resource.content
    );
}

#[test]
fn unbound_reading_cannot_publish_an_unapproved_embedded_template_suggestion() {
    let mut db = database();
    let target = paper(&db);
    let value = json!({"kind":"paper_overview_unit_v1","schemaVersion":1,"profileId":"paper_template_v1","paperTemplate":{"portraitCandidateId":"old-portrait","portraitRevision":1,"fields":[{"id":"question","title":"问题","question":"本文问题","claimIds":[]}]},"fields":[{"field":"question","status":"not_found","statements":[]}]});
    let invalid = crate::hbs_storage::test_sources::write(
        &mut db,
        draft(&target.id, &value.to_string(), "unapproved-suggestion"),
    )
    .unwrap();
    assert_eq!(invalid.status, "invalid");
    assert!(invalid.errors[0].message.contains("尚未确认并采用"));
    assert!(get(&db, &target.id).unwrap().content.is_none());
    let repaired = crate::hbs_storage::test_sources::write(
        &mut db,
        draft(
            &target.id,
            &reading("固定旧配置仍可编辑。"),
            "generic-transition",
        ),
    )
    .unwrap();
    assert_eq!(repaired.status, "applied", "{:?}", repaired.errors);
}
