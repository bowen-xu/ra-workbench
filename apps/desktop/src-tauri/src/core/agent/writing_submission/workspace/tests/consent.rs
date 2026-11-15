use super::*;
use crate::agent::{
    repository::SqliteConversationStore,
    service::AgentSessionService,
    shell_tools::{EXEC, ShellTools, workspace_host::WorkspaceShellHost},
};
use research_agent_core::{model::ToolCall, runner::ToolExecutor};
use std::sync::Arc;

#[tokio::test]
async fn dropped_revocation_waiter_still_finishes_owned_process() {
    let f = fixture(true);
    let service = WritingSubmissionService::from_app_store(&f.store);
    let submitted = service
        .prepare(&f.models, request(&f, Uuid::new_v4()))
        .await
        .unwrap();
    let prepared = service
        .prepare_workspace(
            WritingSubmissionReceipt {
                conversation_id: f.conversation,
                request_id: submitted.record.request_id,
                manuscript_id: f.manuscript,
            },
            submitted.record.context_snapshot_id,
            WritingWorkspaceRequest {
                manuscript_id: f.manuscript,
                buffers: vec![],
            },
        )
        .await
        .unwrap();
    let approved = service.approve_workspace(prepared).await.unwrap();
    let binding = approved.binding.clone();
    let scope = approved.lease.manifest().identity.clone();
    let sessions = Arc::new(AgentSessionService::new(Arc::new(
        SqliteConversationStore::new(
            f.store.shared_agent_connection(),
            "consent-dropped-waiter".into(),
        )
        .unwrap(),
    )));
    let registry = sessions.processes().clone();
    registry
        .begin_turn(scope.conversation_id, scope.turn_id)
        .unwrap();
    let db = f.store.shared_agent_connection();
    let protected = f.store.manuscript_directory().canonicalize().unwrap();
    let host_registry = registry.clone();
    let host = Arc::new(
        tokio::task::spawn_blocking(move || {
            WorkspaceShellHost::from_approved_binding(
                approved.lease,
                host_registry,
                vec![protected],
                db,
                approved.binding,
            )
            .unwrap()
        })
        .await
        .unwrap(),
    );
    let tools = ShellTools::new(registry.clone(), scope, host.clone());
    let output = tools
        .authorize_and_execute(ToolCall {
            namespace: None,
            raw_arguments_json: None,
            call_id: "live-dropped".into(),
            name: EXEC.into(),
            arguments: serde_json::json!({"command":"sleep 30","yield_ms":0}),
        })
        .await
        .unwrap();
    assert!(!output.is_error, "{}", output.content);

    // Hold admission until the outer waiter is dropped. Sending the signal and
    // polling revoke occur in one uninterrupted task poll before its first await.
    let guard = f.store.connection().unwrap();
    let (started, ready) = tokio::sync::oneshot::channel();
    let caller = tokio::spawn(async move {
        let _ = started.send(());
        service.revoke_workspace(sessions, binding).await
    });
    ready.await.unwrap();
    caller.abort();
    let _ = caller.await;
    drop(guard);
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            if host.changes().await.is_ok() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the retained owner must finish the process");
    let state: String = f.store.connection().unwrap().query_row(
        "select state from agent_execution_workspaces where conversation_id=?1 and request_id=?2",
        params![f.conversation.to_string(), submitted.record.request_id.to_string()], |row| row.get(0),
    ).unwrap();
    assert_eq!(state, "revoked");
    registry.shutdown().await.unwrap();
}

#[tokio::test]
async fn uncertain_revocation_waits_for_running_process_cleanup() {
    let f = fixture(true);
    let service = WritingSubmissionService::from_app_store(&f.store);
    let submitted = service
        .prepare(&f.models, request(&f, Uuid::new_v4()))
        .await
        .unwrap();
    let prepared = service
        .prepare_workspace(
            WritingSubmissionReceipt {
                conversation_id: f.conversation,
                request_id: submitted.record.request_id,
                manuscript_id: f.manuscript,
            },
            submitted.record.context_snapshot_id,
            WritingWorkspaceRequest {
                manuscript_id: f.manuscript,
                buffers: vec![],
            },
        )
        .await
        .unwrap();
    let approved = service.approve_workspace(prepared).await.unwrap();
    let binding = approved.binding.clone();
    let scope = approved.lease.manifest().identity.clone();
    let sessions = Arc::new(AgentSessionService::new(Arc::new(
        SqliteConversationStore::new(
            f.store.shared_agent_connection(),
            "consent-process-fault".into(),
        )
        .unwrap(),
    )));
    let registry = sessions.processes().clone();
    registry
        .begin_turn(scope.conversation_id, scope.turn_id)
        .unwrap();
    let db = f.store.shared_agent_connection();
    let protected = f.store.manuscript_directory().canonicalize().unwrap();
    let host_registry = registry.clone();
    let host = Arc::new(
        tokio::task::spawn_blocking(move || {
            WorkspaceShellHost::from_approved_binding(
                approved.lease,
                host_registry,
                vec![protected],
                db,
                approved.binding,
            )
            .unwrap()
        })
        .await
        .unwrap(),
    );
    let tools = ShellTools::new(registry.clone(), scope, host.clone());
    let output = tools
        .authorize_and_execute(ToolCall {
            namespace: None,
            raw_arguments_json: None,
            call_id: "live-fault".into(),
            name: EXEC.into(),
            arguments: serde_json::json!({"command":"sleep 30","yield_ms":0}),
        })
        .await
        .unwrap();
    assert!(!output.is_error, "{}", output.content);
    f.store.connection().unwrap().execute_batch("pragma foreign_keys=on;
        create table test_cancel_parent(id integer primary key);
        create table test_cancel_child(parent_id integer references test_cancel_parent(id) deferrable initially deferred);
        create trigger test_cancel_commit after update on agent_execution_workspaces begin insert into test_cancel_child(parent_id) values (1); end;").unwrap();
    assert!(matches!(
        service.revoke_workspace(sessions, binding).await,
        Err(Error::RecoveryRequired)
    ));
    assert!(f.store.connection().is_err());
    // Changes require the execution marker to have been cleared by process finish.
    assert!(host.changes().await.unwrap().is_empty());
    registry.shutdown().await.unwrap();
}

#[tokio::test]
async fn consent_rollback_failure_fences_owner_after_wrong_binding() {
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization, TransactionOperation};
    let f = fixture(true);
    let service = WritingSubmissionService::from_app_store(&f.store);
    let submitted = service
        .prepare(&f.models, request(&f, Uuid::new_v4()))
        .await
        .unwrap();
    let prepared = service
        .prepare_workspace(
            WritingSubmissionReceipt {
                conversation_id: f.conversation,
                request_id: submitted.record.request_id,
                manuscript_id: f.manuscript,
            },
            submitted.record.context_snapshot_id,
            WritingWorkspaceRequest {
                manuscript_id: f.manuscript,
                buffers: vec![],
            },
        )
        .await
        .unwrap();
    let mut wrong = prepared.binding.clone();
    wrong.lease_id = Uuid::new_v4();
    let sessions = Arc::new(AgentSessionService::new(Arc::new(
        SqliteConversationStore::new(f.store.shared_agent_connection(), "consent-rollback".into())
            .unwrap(),
    )));
    assert!(matches!(
        service
            .revoke_workspace(sessions.clone(), wrong.clone())
            .await,
        Err(Error::Binding(_))
    ));
    {
        let db = f.store.connection().unwrap();
        db.authorizer(Some(|context: AuthContext<'_>| match context.action {
            AuthAction::Transaction {
                operation: TransactionOperation::Rollback,
            } => Authorization::Deny,
            _ => Authorization::Allow,
        }));
    }
    assert!(matches!(
        service.revoke_workspace(sessions, wrong).await,
        Err(Error::RecoveryRequired)
    ));
    assert!(f.store.connection().is_err());
}

#[tokio::test]
async fn consent_commit_failure_fences_approval_and_revocation() {
    for revoke in [false, true] {
        let f = fixture(true);
        let service = WritingSubmissionService::from_app_store(&f.store);
        let submitted = service
            .prepare(&f.models, request(&f, Uuid::new_v4()))
            .await
            .unwrap();
        let prepared = service
            .prepare_workspace(
                WritingSubmissionReceipt {
                    conversation_id: f.conversation,
                    request_id: submitted.record.request_id,
                    manuscript_id: f.manuscript,
                },
                submitted.record.context_snapshot_id,
                WritingWorkspaceRequest {
                    manuscript_id: f.manuscript,
                    buffers: vec![],
                },
            )
            .await
            .unwrap();
        let binding = prepared.binding.clone();
        let sessions = Arc::new(AgentSessionService::new(Arc::new(
            SqliteConversationStore::new(f.store.shared_agent_connection(), "consent-fault".into())
                .unwrap(),
        )));
        if revoke {
            drop(service.approve_workspace(prepared).await.unwrap());
        } else {
            drop(prepared);
        }
        let prepared = if revoke {
            None
        } else {
            Some(
                service
                    .restore_workspace(
                        WritingSubmissionReceipt {
                            conversation_id: f.conversation,
                            request_id: binding.request_id,
                            manuscript_id: f.manuscript,
                        },
                        binding.context_snapshot_id,
                    )
                    .await
                    .unwrap(),
            )
        };
        f.store.connection().unwrap().execute_batch("pragma foreign_keys=on;
            create table test_consent_parent(id integer primary key);
            create table test_consent_child(parent_id integer references test_consent_parent(id) deferrable initially deferred);
            create trigger test_consent_commit after update on agent_execution_workspaces begin insert into test_consent_child(parent_id) values (1); end;").unwrap();
        let result = if let Some(prepared) = prepared {
            service.approve_workspace(prepared).await.map(|_| ())
        } else {
            service.revoke_workspace(sessions, binding).await
        };
        assert!(matches!(result, Err(Error::RecoveryRequired)));
        assert!(
            f.store.connection().is_err(),
            "unconfirmed consent must fence the managed owner"
        );
    }
}

#[tokio::test]
async fn native_consent_and_revocation_stop_real_owned_execution_and_allow_retry() {
    let f = fixture(true);
    let service = WritingSubmissionService::from_app_store(&f.store);
    let submitted = service
        .prepare(&f.models, request(&f, Uuid::new_v4()))
        .await
        .unwrap();
    let receipt = WritingSubmissionReceipt {
        conversation_id: f.conversation,
        request_id: submitted.record.request_id,
        manuscript_id: f.manuscript,
    };
    let prepared = service
        .prepare_workspace(
            receipt,
            submitted.record.context_snapshot_id,
            WritingWorkspaceRequest {
                manuscript_id: f.manuscript,
                buffers: vec![],
            },
        )
        .await
        .unwrap();
    let approved = service.approve_workspace(prepared).await.unwrap();
    let binding = approved.binding.clone();
    let scope = approved.lease.manifest().identity.clone();
    let sessions = Arc::new(AgentSessionService::new(Arc::new(
        SqliteConversationStore::new(f.store.shared_agent_connection(), "consent-test".into())
            .unwrap(),
    )));
    let registry = sessions.processes().clone();
    registry
        .begin_turn(scope.conversation_id, scope.turn_id)
        .unwrap();
    let db = f.store.shared_agent_connection();
    let protected = f.store.manuscript_directory().canonicalize().unwrap();
    let host_registry = registry.clone();
    let host = Arc::new(
        tokio::task::spawn_blocking(move || {
            WorkspaceShellHost::from_approved_binding(
                approved.lease,
                host_registry,
                vec![protected],
                db,
                approved.binding,
            )
            .unwrap()
        })
        .await
        .unwrap(),
    );
    let tools = ShellTools::new(registry.clone(), scope.clone(), host.clone());
    let output = tools
        .authorize_and_execute(ToolCall {
            namespace: None,
            raw_arguments_json: None,
            call_id: "live".into(),
            name: EXEC.into(),
            arguments: serde_json::json!({"command":"sleep 30","yield_ms":0}),
        })
        .await
        .unwrap();
    assert!(!output.is_error, "{}", output.content);
    // A different binding cannot cancel the legitimate operation.
    let mut wrong = binding.clone();
    wrong.lease_id = Uuid::new_v4();
    assert!(
        service
            .revoke_workspace(sessions.clone(), wrong)
            .await
            .is_err()
    );
    service
        .revoke_workspace(sessions.clone(), binding.clone())
        .await
        .unwrap();
    // Completion means the execution marker/lock has been released, not merely
    // that a cancellation request was queued.
    assert!(host.changes().await.unwrap().is_empty());
    assert!(
        tools
            .authorize_and_execute(ToolCall {
                namespace: None,
                raw_arguments_json: None,
                call_id: "after".into(),
                name: EXEC.into(),
                arguments: serde_json::json!({"command":"touch forbidden"})
            })
            .await
            .is_err()
    );
    service.revoke_workspace(sessions, binding).await.unwrap();
    registry.shutdown().await.unwrap();
}
