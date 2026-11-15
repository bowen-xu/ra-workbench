use super::*;
use crate::structured_resources::{self as resources, WriteDraft};
use research_workbench_native_sdk::editable_template::{
    EditableTemplateDecisionV1, EditableTemplateInputV1, EditableTemplateOutcomeV1,
};
use serde_json::json;

fn template(name: &str) -> Value {
    json!({"kind":"reading_template_v1","schemaVersion":1,"name":name,
        "fields":[{"id":"question","title":"Question","question":"What is the question?"}]})
}
fn write(fixture: &Fixture, content: String) -> WriteDraft {
    let resource = resources::get(&fixture.store.connection().unwrap(), &fixture.id).unwrap();
    WriteDraft {
        resource_id: resource.id,
        expected_revision: resource.revision,
        content,
        request_id: "native-template-write:1".into(),
        conversation_id: None,
        expected_draft_revision: None,
        source_snapshot_id: None,
        submission_id: None,
        editor_kind: "human".into(),
    }
}
fn response(value: &Value, rejected: bool) -> CatalogOutput {
    CatalogOutput::EditableTemplate {
        outcome: Box::new(EditableTemplateOutcomeV1 {
            input: Box::new(EditableTemplateInputV1 {
                candidate_json: value.to_string(),
            }),
            decision: if rejected {
                EditableTemplateDecisionV1::Rejected {
                    message: "模板类型、名称或字段数量无效".into(),
                }
            } else {
                EditableTemplateDecisionV1::Validated {
                    canonical_json: value.to_string(),
                }
            },
        }),
    }
}
fn prepared(outputs: &[CatalogOutput]) -> Fixture {
    let mut fixture = Fixture::new(&[], false, RuntimeEvents::new(|_, _| Ok(())));
    fixture.id = resources::create_template(
        &fixture.store.connection().unwrap(),
        "Original",
        "new-template:1",
        None,
    )
    .unwrap()
    .id;
    fixture.responses(outputs);
    // Setup reused the original fixture's startup. The actual template owner
    // must not consume these three unrelated reading resources.
    for name in ["snapshot-hash", "snapshot-rows", "validation-latest"] {
        std::fs::remove_file(
            fixture
                ._package
                .path()
                .join("writing/data")
                .join(format!("reading-evidence-{name}.v1.sql")),
        )
        .unwrap();
    }
    fixture.key = fixture.service.catalog.discover().unwrap().packages[0]
        .package_key
        .clone();
    fixture
}
fn calls(fixture: &Fixture) -> usize {
    fixture
        .lifecycle()
        .lines()
        .filter(|line| line.starts_with("call "))
        .count()
}

#[tokio::test]
async fn template_write_publishes_once_and_replays_without_reading_sql_or_another_frame() {
    let value = template("Saved through Native");
    let fixture = prepared(&[response(&value, false)]);
    let input = write(&fixture, value.to_string());
    let result = fixture
        .service
        .write_resource(fixture.store.clone(), input.clone())
        .await
        .unwrap();
    assert_eq!(result.status, "applied");
    assert_eq!(result.resource.content, Some(value));
    assert!(result.draft.is_none());
    assert!(result.validation_origin.is_none());
    let replay = fixture
        .service
        .write_resource(fixture.store.clone(), input)
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(replay).unwrap(),
        serde_json::to_value(result).unwrap()
    );
    assert_eq!(calls(&fixture), 1);
    assert!(fixture.store.connection().unwrap().is_autocommit());
    fixture.service.shutdown().await.unwrap();
}

#[tokio::test]
async fn normal_template_rejection_keeps_raw_draft_and_actual_origin_in_operation_replay() {
    let value = template("");
    let fixture = prepared(&[response(&value, true)]);
    let input = write(&fixture, value.to_string());
    let result = fixture
        .service
        .write_resource(fixture.store.clone(), input.clone())
        .await
        .unwrap();
    assert_eq!(result.status, "invalid");
    assert!(result.receipt.is_none());
    assert_eq!(result.draft.as_ref().unwrap().content, input.content);
    assert_eq!(result.errors[0].code, "invalid_content");
    assert_eq!(result.errors[0].path, "$");
    assert_eq!(result.errors[0].message, "模板类型、名称或字段数量无效");
    let origin = result.validation_origin.as_ref().unwrap();
    assert_eq!(origin.request_id, input.request_id);
    assert_eq!(origin.package_key, fixture.key);
    assert!(!origin.instance_id.is_empty());
    assert!(!origin.native_request_id.is_empty());
    let replay = fixture
        .service
        .write_resource(fixture.store.clone(), input)
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(replay).unwrap(),
        serde_json::to_value(result).unwrap()
    );
    assert_eq!(calls(&fixture), 1);
    fixture.service.shutdown().await.unwrap();
}

#[tokio::test]
async fn malformed_json_and_stale_resource_settle_before_an_unused_plugin_decision() {
    let fixture = prepared(&[]);
    let mut initial = write(&fixture, template("Initial formal content").to_string());
    initial.request_id = "seed-template:1".into();
    let seeded =
        crate::hbs_storage::test_sources::write(&mut fixture.store.connection().unwrap(), initial)
            .unwrap();
    assert_eq!(seeded.status, "applied");
    let invalid = fixture
        .service
        .write_resource(
            fixture.store.clone(),
            write(&fixture, "{ raw invalid".into()),
        )
        .await
        .unwrap();
    assert_eq!(invalid.status, "invalid");
    assert_eq!(invalid.errors[0].code, "invalid_json");
    assert!(invalid.validation_origin.is_none());
    assert!(fixture.service.host.lock().await.is_none());
    let mut stale = write(&fixture, template("Stale").to_string());
    stale.request_id = "native-template-write:stale".into();
    stale.expected_revision -= 1;
    stale.expected_draft_revision = Some(invalid.draft.unwrap().revision);
    let conflict = fixture
        .service
        .write_resource(fixture.store.clone(), stale)
        .await
        .unwrap();
    assert_eq!(conflict.status, "conflict");
    assert!(conflict.validation_origin.is_none());
    assert!(fixture.service.host.lock().await.is_none());
    assert_eq!(calls(&fixture), 0);
    fixture.service.shutdown().await.unwrap();
}

#[tokio::test]
async fn mismatched_template_input_is_technical_and_preserves_durable_stage_without_normal_result()
{
    let value = template("Original dispatch");
    let fixture = prepared(&[response(&template("Wrong input"), false)]);
    let input = write(&fixture, value.to_string());
    let error = fixture
        .service
        .write_resource(fixture.store.clone(), input.clone())
        .await
        .unwrap_err();
    assert_eq!(error.code, "storage_error");
    assert_eq!(error.request_id.as_deref(), Some(input.request_id.as_str()));
    assert!(error.message.contains("重试原保存请求"));
    let native = error.native_failure.unwrap();
    assert_eq!(native.code, "plugin_host_response_mismatch");
    assert_eq!(native.kind, "technical");
    let db = fixture.store.connection().unwrap();
    assert!(db.is_autocommit());
    let mut resource = resources::get(&db, &fixture.id).unwrap();
    resources::attach_latest_draft(&db, &mut resource).unwrap();
    let draft = resource.draft.unwrap();
    assert_eq!(draft.content, input.content);
    assert_eq!(
        db.query_row(
            "select count(*) from structured_resource_operations where request_id=?1",
            [&input.request_id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        db.query_row(
            "select count(*) from structured_resource_receipts where request_id=?1",
            [&input.request_id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    drop(db);
    fixture.service.shutdown().await.unwrap();
}
