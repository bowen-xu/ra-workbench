use super::*;
use crate::{
    ai::structured_resource_validation::{DecisionOrigin, Failure, FailureCause},
    hbs_storage::boundary::{BoundaryFailure, HbsOutcome},
    runtime_plugins::native::{NativeHostError, NativeProfile, SafeDatabaseError},
};
use serde_json::json;

fn boundary(cause: FailureCause, outcome: HbsOutcome) -> BoundaryFailure {
    BoundaryFailure {
        failure: Failure {
            error: "MODEL_INVALID_DOCUMENT：invalid_content $.fields original safe cause".into(),
            cause,
        },
        phase: "validation",
        outcome,
        rollback_error: None,
    }
}

#[test]
fn static_worker_failure_keeps_core_identity_without_a_native_plugin() {
    let wire = serde_json::to_value(static_worker_failure(Some("core-only-static:1"))).unwrap();
    assert_eq!(wire["code"], "MODEL_STORAGE_ERROR");
    assert_eq!(wire["requestId"], "core-only-static:1");
    assert!(wire.get("nativeFailure").is_none());
}

#[test]
fn actual_native_rejection_keeps_model_and_independent_request_identities() {
    let error = CommandError::from_operation(
        HbsOperationError::Boundary(boundary(
            FailureCause::NativeRejection(Box::new(DecisionOrigin {
                package_key: "a".repeat(64),
                instance_id: "native-instance".into(),
                native_request_id: "native-request:7".into(),
            })),
            HbsOutcome::RolledBack,
        )),
        Some("core-request:1"),
    );
    let wire = serde_json::to_value(error).unwrap();
    assert_eq!(wire["code"], "MODEL_INVALID_DOCUMENT");
    assert_eq!(
        wire["message"],
        "MODEL_INVALID_DOCUMENT：invalid_content $.fields original safe cause"
    );
    assert_eq!(wire["requestId"], "core-request:1");
    assert_eq!(
        wire["nativeFailure"],
        json!({"kind":"pureRejected","phase":"validation","outcome":"rolledBack","pluginId":"literature_library","code":"invalid_input","packageKey":"a".repeat(64),"instanceId":"native-instance","nativeRequestId":"native-request:7"})
    );
}

#[test]
fn source_sql_keeps_safe_driver_cause_and_model_context_without_a_fake_frame() {
    let db = rusqlite::Connection::open_in_memory().unwrap();
    db.execute_batch("create table diagnostic_fixture(value); create trigger private_driver before insert on diagnostic_fixture begin select raise(abort,'PRIVATE_DRIVER_MESSAGE'); end;").unwrap();
    let sql = db
        .execute("insert into diagnostic_fixture values(1)", [])
        .unwrap_err();
    assert!(sql.to_string().contains("PRIVATE_DRIVER_MESSAGE"));
    let cause = FailureCause::SourceDatabase(Box::new(SafeDatabaseError::sql("source-read", &sql)));
    let mut failure = boundary(cause, HbsOutcome::RolledBack);
    failure.failure.error = format!("MODEL_INVALID_DOCUMENT：{sql}");
    let wire = serde_json::to_value(CommandError::from_operation(
        HbsOperationError::Boundary(failure),
        Some("core-request:2"),
    ))
    .unwrap();
    assert_eq!(wire["code"], "MODEL_STORAGE_ERROR");
    assert!(
        wire["message"]
            .as_str()
            .unwrap()
            .contains("MODEL_INVALID_DOCUMENT")
    );
    assert_eq!(wire["nativeFailure"]["kind"], "sourceData");
    assert_eq!(
        wire["nativeFailure"]["databaseError"]["safeCause"],
        "sqlite_operation_failed"
    );
    assert!(wire["nativeFailure"]["databaseError"]["sqliteCode"].is_string());
    assert!(wire["nativeFailure"].get("nativeRequestId").is_none());
    assert!(wire["nativeFailure"].get("packageKey").is_none());
    assert!(!wire.to_string().contains("PRIVATE_DRIVER_MESSAGE"));
}

#[test]
fn technical_failure_keeps_fixed_process_metadata_and_omits_arbitrary_child_text() {
    let mut error = NativeHostError::for_profile(
        NativeProfile::NoteTraceDecision,
        "plugin_host_spawn_failed",
        "process I/O failed (operation=child_start_kill, kind=PermissionDenied, raw_os_error=Some(13))",
    );
    error.package_key = Some("b".repeat(64));
    let wire = serde_json::to_value(CommandError::from_operation(
        HbsOperationError::native(error, "validation", HbsOutcome::RolledBack),
        Some("core-request:3"),
    ))
    .unwrap();
    assert_eq!(wire["code"], "MODEL_STORAGE_ERROR");
    assert!(
        wire["message"]
            .as_str()
            .unwrap()
            .contains("operation=child_start_kill, kind=PermissionDenied, raw_os_error=Some(13)")
    );
    let mut private = NativeHostError::for_profile(
        NativeProfile::NoteTraceDecision,
        "plugin_host_output_failed",
        "PRIVATE_BODY /private/path Authorization: secret",
    );
    private.cleanup_error = Some("PRIVATE_CLEANUP".into());
    let wire = serde_json::to_value(CommandError::from_operation(
        HbsOperationError::native(private, "validation", HbsOutcome::Unknown),
        Some("core-request:4"),
    ))
    .unwrap();
    assert!(!wire.to_string().contains("PRIVATE_"));
    assert_eq!(wire["nativeFailure"]["code"], "plugin_host_output_failed");
    assert_eq!(wire["nativeFailure"]["outcome"], "unknown");
}

#[test]
fn failed_rejection_rollback_and_committed_errors_cannot_look_definite() {
    let mut failure = boundary(
        FailureCause::NativeRejection(Box::new(DecisionOrigin {
            package_key: "c".repeat(64),
            instance_id: "instance".into(),
            native_request_id: "native:8".into(),
        })),
        HbsOutcome::Unknown,
    );
    failure.rollback_error = Some(Box::new(SafeDatabaseError::invariant(
        "rollback_failed",
        "rollback",
    )));
    let wire = serde_json::to_value(CommandError::from_operation(
        HbsOperationError::Boundary(failure),
        Some("core:8"),
    ))
    .unwrap();
    assert_eq!(wire["nativeFailure"]["outcome"], "unknown");
    assert_eq!(
        wire["nativeFailure"]["rollbackError"]["code"],
        "rollback_failed"
    );
    let wire = serde_json::to_value(CommandError::from_operation(
        HbsOperationError::Boundary(boundary(FailureCause::Original, HbsOutcome::Committed)),
        Some("core:9"),
    ))
    .unwrap();
    assert_eq!(wire["nativeFailure"]["kind"], "technical");
    assert_eq!(wire["nativeFailure"]["outcome"], "committed");
    let mut failure = boundary(FailureCause::Original, HbsOutcome::Unknown);
    failure.failure.error = "MODEL_RECOVERY_INVALID：PRIVATE_DATABASE_CAUSE".into();
    let wire = serde_json::to_value(CommandError::from_operation(
        HbsOperationError::Boundary(failure),
        Some("core:10"),
    ))
    .unwrap();
    assert_eq!(wire["nativeFailure"]["kind"], "technical");
    assert!(!wire.to_string().contains("PRIVATE_DATABASE_CAUSE"));
    let mut failure = boundary(FailureCause::Original, HbsOutcome::Committed);
    failure.phase = "postcommit-read";
    failure.failure.error = "MODEL_STORAGE_ERROR：not_found：找不到结构化资源".into();
    let wire = serde_json::to_value(CommandError::from_operation(
        HbsOperationError::Boundary(failure),
        Some("core:11"),
    ))
    .unwrap();
    assert_eq!(wire["nativeFailure"]["code"], "not_found");
    assert_eq!(
        wire["message"],
        "MODEL_STORAGE_ERROR：not_found：找不到结构化资源"
    );
    assert!(wire["nativeFailure"].get("nativeRequestId").is_none());
}
