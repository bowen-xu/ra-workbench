use super::*;
use crate::agent::writing_application_commands as boundary;

fn request(scope: ApplicationScope, path: &str) -> boundary::Request {
    action(scope, path, "applyDisk")
}
fn action(scope: ApplicationScope, path: &str, action: &str) -> boundary::Request {
    serde_json::from_value(serde_json::json!({
        "action":action,
        "scope":{"conversationId":scope.conversation,"manuscriptId":scope.manuscript,
            "proposalId":scope.proposal,"operationId":scope.operation},
        "path":path
    }))
    .unwrap()
}

#[tokio::test]
async fn disk_reconciliation_command_confirms_without_replaying_or_overwriting_edits() {
    let f = fixture_with_objects();
    let scope = existing(&f, false).await;
    f.store.connection().unwrap().execute_batch(
        "create trigger fail_agent_ack before update on manuscript_files begin select raise(abort, 'fixture'); end;"
    ).unwrap();
    assert!(
        boundary::execute(&f.store, request(scope, "main.tex"))
            .await
            .is_err()
    );
    f.store
        .connection()
        .unwrap()
        .execute_batch("drop trigger fail_agent_ack;")
        .unwrap();
    for path in ["../main.tex", "unknown.tex"] {
        assert!(
            boundary::execute(&f.store, action(scope, path, "reconcileDisk"))
                .await
                .is_err()
        );
    }
    let response = serde_json::to_value(
        boundary::execute(&f.store, action(scope, "main.tex", "reconcileDisk"))
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(response["status"], "applied");
    assert_eq!(response["files"][0]["phase"], "acknowledged");
    std::fs::write(source(&f).join("main.tex"), b"later user edit").unwrap();
    let repeated = serde_json::to_value(
        boundary::execute(&f.store, action(scope, "main.tex", "reconcileDisk"))
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(repeated, response);
    assert_eq!(
        std::fs::read(source(&f).join("main.tex")).unwrap(),
        b"later user edit"
    );
}

#[tokio::test]
async fn disk_command_applies_frozen_changes_and_preserves_later_edits_on_retry() {
    for remove in [false, true] {
        let f = fixture_with_objects();
        let scope = existing(&f, remove).await;
        for path in ["../main.tex", ".agent-before-private", "unknown.tex"] {
            assert!(
                boundary::execute(&f.store, request(scope, path))
                    .await
                    .is_err()
            );
        }
        let response = serde_json::to_value(
            boundary::execute(&f.store, request(scope, "main.tex"))
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(response["status"], "applied");
        let rollback = serde_json::from_value(serde_json::json!({"action":"rollbackDisk","scope":{
            "conversationId":scope.conversation,"manuscriptId":scope.manuscript,
            "proposalId":scope.proposal,"operationId":scope.operation
        }}))
        .unwrap();
        assert!(boundary::execute(&f.store, rollback).await.is_err());
        assert_eq!(response["files"][0]["phase"], "acknowledged");
        assert_eq!(response["files"][0]["target"]["kind"], "disk");
        if remove {
            assert!(!source(&f).join("main.tex").exists());
        } else {
            assert_eq!(
                std::fs::read(source(&f).join("main.tex")).unwrap(),
                b"replacement"
            );
        }
        std::fs::write(source(&f).join("main.tex"), b"later user edit").unwrap();
        assert_eq!(
            serde_json::to_value(
                boundary::execute(&f.store, request(scope, "main.tex"))
                    .await
                    .unwrap()
            )
            .unwrap(),
            response
        );
        assert_eq!(
            std::fs::read(source(&f).join("main.tex")).unwrap(),
            b"later user edit"
        );
    }
}

#[tokio::test]
async fn disk_command_keeps_uncertain_write_recoverable_without_redispatch() {
    let f = fixture_with_objects();
    let scope = existing(&f, false).await;
    f.store.connection().unwrap().execute_batch(
        "create trigger fail_agent_ack before update on manuscript_files begin select raise(abort, 'fixture'); end;"
    ).unwrap();
    assert!(
        boundary::execute(&f.store, request(scope, "main.tex"))
            .await
            .is_err()
    );
    assert_eq!(
        std::fs::read(source(&f).join("main.tex")).unwrap(),
        b"replacement"
    );
    std::fs::write(source(&f).join("main.tex"), b"manual recovery edit").unwrap();
    let response = serde_json::to_value(
        boundary::execute(&f.store, request(scope, "main.tex"))
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(response["status"], "recoveryRequired");
    assert_eq!(response["files"][0]["phase"], "dispatched");
    assert_eq!(
        std::fs::read(source(&f).join("main.tex")).unwrap(),
        b"manual recovery edit"
    );
}
