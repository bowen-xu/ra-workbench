use super::*;
use crate::data_interface::SqlImmediateReceipt;
use research_data_runtime::ReadFence;

#[test]
fn status_dto_preserves_decimal_fence_and_safe_terminal_flags() {
    let epoch = Uuid::from_u128(9);
    let fence = ReadFence {
        epoch,
        position: 9_007_199_254_740_993,
    };
    let statuses = [
        SqlImmediateStatus::Committed(SqlImmediateReceipt {
            operation_id: Uuid::from_u128(1),
            epoch,
            sql_fence: fence,
            fingerprint: [0; 32],
            result: (),
            replayed: true,
        }),
        SqlImmediateStatus::NoSqlReceipt {
            epoch,
            sql_fence: fence,
            closed: true,
        },
        SqlImmediateStatus::Unconfirmed {
            epoch,
            sql_fence: fence,
            blocked: true,
            closed: false,
            reason: "owner_blocked_receipt_unconfirmed",
        },
    ];
    for (status, kind) in statuses
        .into_iter()
        .zip(["committed", "noSqlReceipt", "unconfirmed"])
    {
        let response = AuthorizationStatusResponse {
            schema_version: 1,
            conversation_id: Uuid::from_u128(2),
            request_id: Uuid::from_u128(1),
            status: status.into(),
        };
        let value = serde_json::to_value(response).unwrap();
        assert_eq!(value["status"]["type"], kind);
        assert_eq!(value["status"]["epoch"], epoch.to_string());
        assert_eq!(value["status"]["sqlFence"], "9007199254740993");
        assert!(value["status"].get("fingerprint").is_none());
        assert!(value["status"].get("replayed").is_none());
        match kind {
            "noSqlReceipt" => assert_eq!(value["status"]["closed"], true),
            "unconfirmed" => {
                assert_eq!(value["status"]["blocked"], true);
                assert_eq!(value["status"]["closed"], false);
                assert_eq!(
                    value["status"]["reason"],
                    "owner_blocked_receipt_unconfirmed"
                );
            }
            _ => {}
        }
    }
}

#[test]
fn status_request_is_closed_and_validates_before_storage_access() {
    let valid = serde_json::json!({"schemaVersion":1,"conversationId":Uuid::from_u128(2),"requestId":Uuid::from_u128(1),"binding":{"kind":"note","resourceId":"note-one"}});
    assert!(
        serde_json::from_value::<AuthorizationStatusRequest>(valid.clone())
            .unwrap()
            .decode()
            .is_ok()
    );
    for (field, value) in [
        ("schemaVersion", serde_json::json!(2)),
        ("conversationId", serde_json::json!(Uuid::nil())),
        ("requestId", serde_json::json!(Uuid::nil())),
    ] {
        let mut wrong = valid.clone();
        wrong[field] = value;
        assert!(matches!(
            serde_json::from_value::<AuthorizationStatusRequest>(wrong)
                .unwrap()
                .decode(),
            Err(Error::InvalidInput)
        ));
    }
    let mut unknown = valid;
    unknown["events"] = serde_json::json!([]);
    assert!(serde_json::from_value::<AuthorizationStatusRequest>(unknown).is_err());
}
