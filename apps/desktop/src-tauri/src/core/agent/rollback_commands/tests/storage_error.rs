use super::*;
use crate::data_interface::SqlImmediateDiagnostic;

#[test]
fn storage_unknown_dto_retains_safe_operation_diagnostics_exactly() {
    let diagnostic = SqlImmediateDiagnostic {
        operation_id: Uuid::from_u128(1),
        epoch: Some(Uuid::from_u128(9)),
        sql_fence: Some("9007199254740993".into()),
        phase: "commit",
        sqlite_code: Some("SystemIoFailure".into()),
        sqlite_extended_code: Some(778),
        safe_cause: "sql_commit_result_unknown",
    };
    let value = serde_json::to_value(Error::Storage(SqlImmediateFailure::Unknown(Box::new(
        diagnostic,
    ))))
    .unwrap();
    let expected: serde_json::Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../tests/fixtures/agent-sql-immediate-unknown-v1.json"
    )))
    .unwrap();
    assert_eq!(value, expected);
}

#[test]
fn domain_authorization_errors_keep_existing_command_meanings() {
    for (error, code) in [
        (CatalogError::InvalidInput, "invalidInput"),
        (CatalogError::NotFound, "hostUnavailable"),
        (CatalogError::Conflict, "conflict"),
    ] {
        let authorization = AuthorizationError::from(SqlImmediateFailure::Domain(error));
        let AuthorizationError::Domain(error) = authorization else {
            panic!("domain error must not become storage")
        };
        assert_eq!(
            serde_json::to_value(catalog_error(error)).unwrap()["code"],
            code
        );
    }
}
