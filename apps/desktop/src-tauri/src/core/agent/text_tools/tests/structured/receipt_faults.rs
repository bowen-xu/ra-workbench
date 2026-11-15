//! Actual structured-resource writes and SQL refusal at the outer tool receipt.
use super::*;
use crate::agent::text_tools::structured::decode_targets;

fn target_key(f: &Fixture, call: &str) -> String {
    format!("{}:{call}:file:0", f.tools.record.request_id)
}
fn operation(key: &str) -> String {
    format!("text:{:x}", Sha256::digest(key))
}
fn reject_receipt(f: &Fixture, key: &str) {
    let db = f.tools.store.connection().unwrap();
    db.execute_batch("create temp table structured_receipt_fault(request_id text primary key);
        create temp trigger structured_receipt_refusal before update on main.agent_text_patch_calls
        when new.state='complete' and exists(select 1 from structured_receipt_fault where request_id=new.request_id)
        begin select raise(abort,'private synthetic structured receipt cause'); end;").unwrap();
    db.execute("insert into structured_receipt_fault values(?1)", [key])
        .unwrap();
}
fn clear_receipt_fault(f: &Fixture) {
    f.tools
        .store
        .connection()
        .unwrap()
        .execute_batch(
            "drop trigger structured_receipt_refusal;drop table structured_receipt_fault;",
        )
        .unwrap();
}
fn call(id: &str, body: &str) -> ToolCall {
    ToolCall {
        namespace: None,
        raw_arguments_json: None,
        call_id: id.into(),
        name: PATCH.into(),
        arguments: json!({"patch":add(body)}),
    }
}
async fn unknown(f: &Fixture, call: ToolCall) -> Value {
    let result = f.tools.authorize_and_execute(call).await;
    let Err(RunError::ToolOutcomeUnknownDiagnostic(detail)) = result else {
        panic!("a persisted structured effect cannot become ordinary failed feedback");
    };
    let detail = serde_json::to_value(detail).unwrap();
    assert_eq!(
        detail["conversationId"],
        json!(f.tools.record.conversation_id)
    );
    assert_eq!(detail["requestId"], json!(f.tools.record.request_id));
    assert_eq!(detail["turnId"], json!(f.tools.record.turn_id));
    assert!(!detail.to_string().contains("private synthetic"));
    assert!(!detail.to_string().contains("Synthetic template"));
    detail
}

#[tokio::test]
async fn persisted_template_with_lost_target_receipt_recovers_original_outcome_without_rewriting_later_input()
 {
    let f = template_fixture(true);
    let key = target_key(&f, "structured-target-lost");
    reject_receipt(&f, &key);
    let original_call = call("structured-target-lost", TEMPLATE);
    let detail = unknown(&f, original_call.clone()).await;
    assert_eq!(detail["phase"], "target-terminal");
    assert_eq!(detail["sqliteExtendedCode"], 1811);
    let (resource_id, later_saved) = {
        let mut db = f.tools.store.connection().unwrap();
        let result = crate::structured_resources::query_outcome(&db, &operation(&key))
            .unwrap()
            .unwrap();
        assert_eq!(result.status, "applied");
        assert_eq!(result.resource.revision, 1);
        assert!(!result.resource.pending_field_ids.is_empty());
        let id = result.resource.id.clone();
        crate::hbs_storage::test_sources::write(
            &mut db,
            crate::structured_resources::WriteDraft {
                resource_id: id.clone(),
                expected_revision: 1,
                content: TEMPLATE.replace("Synthetic template", "Later manual input"),
                request_id: "later-manual-input".into(),
                conversation_id: Some(f.tools.record.conversation_id.to_string()),
                expected_draft_revision: None,
                source_snapshot_id: None,
                submission_id: None,
                editor_kind: "human".into(),
            },
        )
        .unwrap();
        let later =
            serde_json::to_value(crate::structured_resources::get(&db, &id).unwrap()).unwrap();
        (id, later)
    };
    clear_receipt_fault(&f);
    let output = f
        .tools
        .authorize_and_execute(original_call.clone())
        .await
        .unwrap();
    assert!(!output.is_error, "{}", output.content);
    let value: Value = serde_json::from_str(&output.content).unwrap();
    assert_eq!(value["targets"][0]["structuredResource"]["revision"], 1);
    let repeated = f.tools.authorize_and_execute(original_call).await.unwrap();
    assert_eq!(repeated.content, output.content);
    let db = f.tools.store.connection().unwrap();
    let latest = crate::structured_resources::get(&db, &resource_id).unwrap();
    assert_eq!(serde_json::to_value(&latest).unwrap(), later_saved);
    assert_eq!(latest.revision, 2);
    assert_eq!(latest.content.unwrap()["name"], "Later manual input");
    let writes: i64 = db
        .query_row(
            "select count(*) from structured_resource_operations where request_id=?1",
            [operation(&key)],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(writes, 1);
}

#[tokio::test]
async fn invalid_json_with_lost_receipt_keeps_the_exact_original_raw_draft() {
    let f = template_fixture(true);
    let key = target_key(&f, "raw-draft-lost");
    reject_receipt(&f, &key);
    let original_call = call("raw-draft-lost", "{");
    let detail = unknown(&f, original_call.clone()).await;
    assert_eq!(detail["phase"], "target-terminal");
    let before = {
        let db = f.tools.store.connection().unwrap();
        let result = crate::structured_resources::query_outcome(&db, &operation(&key))
            .unwrap()
            .unwrap();
        assert_eq!(result.status, "invalid");
        let draft = result.draft.unwrap();
        assert_eq!(draft.content, "{\n");
        assert_eq!(draft.revision, 1);
        serde_json::to_value(draft).unwrap()
    };
    clear_receipt_fault(&f);
    let result = f.tools.authorize_and_execute(original_call).await.unwrap();
    let result: Value = serde_json::from_str(&result.content).unwrap();
    assert_eq!(result["status"], "declined");
    let db = f.tools.store.connection().unwrap();
    let after = crate::structured_resources::query_outcome(&db, &operation(&key))
        .unwrap()
        .unwrap()
        .draft
        .unwrap();
    assert_eq!(serde_json::to_value(after).unwrap(), before);
}

#[tokio::test]
async fn staged_raw_draft_without_final_outcome_stays_unknown_and_does_not_repeat_the_write() {
    let f = template_fixture(true);
    let key = target_key(&f, "staged-only");
    {
        let db = f.tools.store.connection().unwrap();
        db.execute_batch("create temp trigger refuse_structured_outcome before insert on main.structured_resource_operations begin select raise(abort,'private synthetic outcome cause'); end;").unwrap();
    }
    let original_call = call("staged-only", TEMPLATE);
    let detail = unknown(&f, original_call.clone()).await;
    assert_eq!(detail["code"], "TEXT_PATCH_EFFECT_UNCONFIRMED");
    assert_eq!(detail["phase"], "structured-write");
    assert_eq!(detail["sqliteExtendedCode"], 1811);
    let before: (String, i64, String) = {
        let db = f.tools.store.connection().unwrap();
        assert!(
            crate::structured_resources::query_outcome(&db, &operation(&key))
                .unwrap()
                .is_none()
        );
        db.query_row("select draft_id,draft_revision,request_hash from structured_resource_write_intents where request_id=?1", [operation(&key)], |row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).unwrap()
    };
    f.tools.store.connection().unwrap().execute_batch("drop trigger refuse_structured_outcome;
        create temp trigger never_restage before insert on main.structured_resource_write_intents begin select raise(abort,'must not restage'); end;").unwrap();
    let _ = unknown(&f, original_call).await;
    let db = f.tools.store.connection().unwrap();
    assert!(
        crate::structured_resources::query_outcome(&db, &operation(&key))
            .unwrap()
            .is_none()
    );
    let after: (String,i64,String) = db.query_row("select draft_id,draft_revision,request_hash from structured_resource_write_intents where request_id=?1", [operation(&key)], |row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).unwrap();
    assert_eq!(after, before);
    let (raw, revision): (String, i64) = db
        .query_row(
            "select content,revision from structured_resource_drafts where id=?1",
            [&before.0],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(raw, format!("{TEMPLATE}\n"));
    assert_eq!(revision, 1);
}

#[tokio::test]
async fn creation_scope_failure_keeps_the_original_saved_resource_and_repairs_only_its_scope() {
    let f = template_fixture(true);
    let key = target_key(&f, "scope-lost");
    f.tools.store.connection().unwrap().execute_batch("create temp trigger refuse_created_scope before update on main.agent_context_configurations begin select raise(abort,'private synthetic scope cause'); end;").unwrap();
    let original_call = call("scope-lost", TEMPLATE);
    let detail = unknown(&f, original_call.clone()).await;
    assert_eq!(detail["phase"], "structured-scope");
    // The configuration adapter does not expose its driver cause; do not invent one.
    assert!(detail["sqliteExtendedCode"].is_null());
    let original = {
        let db = f.tools.store.connection().unwrap();
        let original = crate::structured_resources::query_outcome(&db, &operation(&key))
            .unwrap()
            .unwrap();
        assert_eq!(original.status, "applied");
        original
    };
    assert_eq!(
        unknown(&f, original_call.clone()).await["phase"],
        "structured-scope"
    );
    f.tools.store.connection().unwrap().execute_batch("drop trigger refuse_created_scope;
        create temp trigger do_not_repeat_structured_write before insert on main.structured_resource_write_intents begin select raise(abort,'must not rewrite'); end;").unwrap();
    let output = f.tools.authorize_and_execute(original_call).await.unwrap();
    assert!(!output.is_error);
    let db = f.tools.store.connection().unwrap();
    let restored = crate::structured_resources::query_outcome(&db, &operation(&key))
        .unwrap()
        .unwrap();
    assert_eq!(
        serde_json::to_value(restored).unwrap(),
        serde_json::to_value(&original).unwrap()
    );
    let saved = configuration::read_in(&db, f.tools.record.conversation_id)
        .unwrap()
        .unwrap();
    assert!(
        saved.state["persistent"]
            .as_array()
            .unwrap()
            .iter()
            .any(|selection| selection["resource"]["resourceId"] == original.resource.id)
    );
}

#[tokio::test]
async fn owned_public_feedback_passes_the_real_executor_and_replay_for_valid_and_invalid_content() {
    for (id, body, status, is_error) in [
        ("valid", TEMPLATE, "complete", false),
        ("invalid", "{", "declined", true),
    ] {
        let f = template_fixture(true);
        let request = call(id, body);
        let output = f
            .tools
            .authorize_and_execute(request.clone())
            .await
            .unwrap();
        assert_eq!(output.is_error, is_error);
        let value: Value = serde_json::from_str(&output.content).unwrap();
        assert_eq!(value["status"], status);
        assert!(value["targets"][0]["structuredResource"]["resourceId"].is_string());
        assert_eq!(
            value["targets"][0]["message"],
            value["targets"][0]["structuredResource"]["guidance"]
        );
        assert!(
            value["targets"][0]["structuredResource"]
                .get("savedDiff")
                .is_none()
        );
        let replayed = f.tools.authorize_and_execute(request).await.unwrap();
        assert_eq!(replayed.content, output.content);
    }
}

#[test]
fn public_target_projection_adapter_retains_strict_other_field_validation() {
    let target = json!({"path":"source://synthetic","status":"applied","message":"saved","structuredResource":{"resourceId":"synthetic","guidance":"saved"}});
    assert_eq!(
        decode_targets(json!([target.clone()])).unwrap()[0].status,
        TargetStatus::Applied
    );
    let mut corrupt = target.clone();
    corrupt["message"] = json!("different");
    assert!(decode_targets(json!([corrupt])).is_err());
    let mut extra = target;
    extra["unownedExtra"] = json!(true);
    assert!(decode_targets(json!([extra])).is_err());
}

#[tokio::test]
async fn confirmed_pre_effect_rejections_remain_feedback_and_create_no_resource_or_draft() {
    for fault in ["target-begin", "invalid-title"] {
        let f = template_fixture(true);
        let key = target_key(&f, fault);
        let before = {
            let db = f.tools.store.connection().unwrap();
            if fault == "target-begin" {
                db.execute_batch("create temp table refused_structured_begin(request_id text primary key);
                    create temp trigger refuse_structured_begin before insert on main.agent_text_patch_calls
                    when exists(select 1 from refused_structured_begin where request_id=new.request_id)
                    begin select raise(abort,'private synthetic begin cause'); end;").unwrap();
                db.execute("insert into refused_structured_begin values(?1)", [&key])
                    .unwrap();
            }
            serde_json::to_value(
                configuration::read_in(&db, f.tools.record.conversation_id).unwrap(),
            )
            .unwrap()
        };
        let body = if fault == "invalid-title" {
            TEMPLATE.replace("Synthetic template", &"x".repeat(121))
        } else {
            TEMPLATE.into()
        };
        let output = f
            .tools
            .authorize_and_execute(call(fault, &body))
            .await
            .unwrap();
        assert!(output.is_error);
        let value: Value = serde_json::from_str(&output.content).unwrap();
        assert_eq!(value["targets"][0]["status"], "failed");
        let diagnostic: Value =
            serde_json::from_str(value["targets"][0]["message"].as_str().unwrap()).unwrap();
        assert!(!diagnostic.to_string().contains("private synthetic"));
        if fault == "target-begin" {
            assert_eq!(diagnostic["sqliteExtendedCode"], 1811);
        }
        let db = f.tools.store.connection().unwrap();
        for table in [
            "structured_resource_operations",
            "structured_resource_write_intents",
            "structured_resource_creations",
        ] {
            let sql = match table {
                "structured_resource_operations" => {
                    "select count(*) from structured_resource_operations where request_id=?1"
                }
                "structured_resource_write_intents" => {
                    "select count(*) from structured_resource_write_intents where request_id=?1"
                }
                _ => "select count(*) from structured_resource_creations where request_id=?1",
            };
            let id = if table == "structured_resource_creations" {
                format!("create:{}", operation(&key))
            } else {
                operation(&key)
            };
            assert_eq!(
                db.query_row(sql, [id], |row| row.get::<_, i64>(0)).unwrap(),
                0,
                "{fault}: {table}"
            );
        }
        assert_eq!(
            serde_json::to_value(
                configuration::read_in(&db, f.tools.record.conversation_id).unwrap()
            )
            .unwrap(),
            before
        );
        assert_eq!(
            db.query_row(
                "select count(*) from structured_resource_drafts where creator_conversation_id=?1",
                [f.tools.record.conversation_id.to_string()],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
    }
}
