use super::*;
use serde_json::json;

fn request() -> QueuePreviewRequest {
    serde_json::from_value(json!({"inputId":Uuid::new_v4(),"expectedRevision":1,
        "submission":{"conversationId":Uuid::new_v4(),"requestId":Uuid::new_v4(),
            "text":"Explain", "context":{"manuscriptId":Uuid::new_v4(),"files":[]}, "model":null}}))
    .unwrap()
}

#[test]
fn prepare_fingerprint_uses_complete_decoded_defaults_and_preserves_all_options() {
    let request = request();
    let canonical = serde_json::to_value(&request).unwrap();
    assert_eq!(request.intent().unwrap().request_json, canonical);
    assert_eq!(canonical["submission"]["userInput"], json!([]));
    assert_eq!(canonical["submission"]["resources"], json!([]));
    assert!(canonical["submission"]["retrieval"].is_null());
    assert!(canonical["submission"]["sessionConfig"].is_null());
    assert!(canonical["submission"]["turnConfig"].is_null());
    let mut full = canonical.clone();
    full["submission"]["userInput"] = json!([{"type":"text","text":"literal"}]);
    full["submission"]["resources"] = json!([{"kind":"manuscriptFile","resourceId":request.submission.context.manuscript_id,"path":"main.tex"}]);
    full["submission"]["sessionConfig"] = json!({"maxBytes":2048,"fallbackFilenames":[]});
    full["submission"]["turnConfig"] = json!({"maxBytes":0,"fallbackFilenames":["AGENTS.md"]});
    full["submission"]["model"] = json!({"providerId":"fixture","modelId":"frozen","reasoningEffort":"high","serviceTier":"priority"});
    let full: QueuePreviewRequest = serde_json::from_value(full).unwrap();
    let fingerprint = full.intent().unwrap().request_json;
    assert_eq!(fingerprint, serde_json::to_value(&full).unwrap());
    assert_ne!(fingerprint, canonical);
    assert_eq!(
        fingerprint["submission"]["resources"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        fingerprint["submission"]["model"]["serviceTier"],
        "priority"
    );
    let mut unknown = fingerprint;
    unknown["submission"]["execute"] = json!(true);
    assert!(serde_json::from_value::<QueuePreviewRequest>(unknown).is_err());
}

#[test]
fn status_echoes_canonical_intent_without_fabricating_proof_for_absence_or_unknown() {
    let request = request();
    let intent = serde_json::to_value(&request).unwrap();
    let missing = serde_json::to_value(PrepareStatusResponse {
        schema_version: 1,
        intent: request.clone(),
        status: PrepareStatus::NotFound,
    })
    .unwrap();
    assert_eq!(
        missing,
        json!({"schemaVersion":1,"intent":intent,"status":{"type":"notFound"}})
    );
    let unknown = serde_json::to_value(PrepareStatusResponse {
        schema_version: 1,
        intent: request.clone(),
        status: PrepareStatus::Unconfirmed {
            detail: Box::new(SqlImmediateDiagnostic {
                operation_id: request.submission.request_id,
                epoch: None,
                sql_fence: None,
                phase: "status",
                sqlite_code: None,
                sqlite_extended_code: None,
                safe_cause: "owner_blocked_receipt_unconfirmed",
            }),
        },
    })
    .unwrap();
    assert_eq!(unknown["intent"], intent);
    assert_eq!(unknown["status"]["type"], "unconfirmed");
    assert_eq!(unknown["status"]["detail"].as_object().unwrap().len(), 7);
    assert!(unknown["status"]["detail"]["epoch"].is_null());
    assert!(unknown["status"].get("proof").is_none());
}

#[test]
fn prepare_storage_unknown_keeps_scope_and_original_operation() {
    let request = request();
    let value = serde_json::to_value(request.error(SqlImmediateFailure::Unknown(Box::new(
        SqlImmediateDiagnostic {
            operation_id: request.submission.request_id,
            epoch: Some(Uuid::new_v4()),
            sql_fence: Some("9".into()),
            phase: "commit",
            sqlite_code: Some("ConstraintViolation".into()),
            sqlite_extended_code: Some(787),
            safe_cause: "sql_commit_failed",
        },
    ))))
    .unwrap();
    assert_eq!(value["code"], "storage");
    assert_eq!(value["reason"]["code"], "DATA_SQL_COMMIT_UNKNOWN");
    assert_eq!(
        value["reason"]["conversationId"],
        request.submission.conversation_id.to_string()
    );
    assert_eq!(
        value["reason"]["manuscriptId"],
        request.submission.context.manuscript_id.to_string()
    );
    assert_eq!(value["reason"]["inputId"], request.input_id.to_string());
    assert_eq!(
        value["reason"]["previewRequestId"],
        request.submission.request_id.to_string()
    );
    assert_eq!(
        value["reason"]["operationId"],
        request.submission.request_id.to_string()
    );
    assert_eq!(value["reason"]["reason"]["sqliteExtendedCode"], 787);
}

#[test]
fn queued_preview_native_transport_failure_keeps_old_class_and_safe_source_identity() {
    use crate::runtime_plugins::native::{NativeHostError, NativeProfile};
    let request = request();
    let native_id = Uuid::new_v4();
    let mut error = NativeHostError::for_profile(
        NativeProfile::WritingProjects,
        "plugin_host_exited",
        "写作宿主已退出。",
    );
    error.request_id = Some(native_id.to_string());
    error.exit_code = Some(43);
    let value = serde_json::to_value(request.source_error(error.into())).unwrap();
    assert_eq!(value["code"], "preparation");
    assert_eq!(value["reason"], "unavailable");
    let detail = &value["writingSource"];
    assert_eq!(
        detail["conversationId"],
        request.submission.conversation_id.to_string()
    );
    assert_eq!(
        detail["manuscriptId"],
        request.submission.context.manuscript_id.to_string()
    );
    assert_eq!(detail["inputId"], request.input_id.to_string());
    assert_eq!(
        detail["operationId"],
        request.submission.request_id.to_string()
    );
    assert_eq!(detail["source"]["error"]["code"], "plugin_host_exited");
    assert_eq!(
        detail["source"]["error"]["requestId"],
        native_id.to_string()
    );
    assert_eq!(detail["source"]["error"]["exitCode"], 43);
}
