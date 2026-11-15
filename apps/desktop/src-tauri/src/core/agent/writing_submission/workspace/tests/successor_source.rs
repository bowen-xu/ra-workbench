use super::*;
use crate::ai::structured_resource_validation::Flow;
use rusqlite::hooks::{AuthAction, AuthContext, Authorization, TransactionOperation};

#[tokio::test]
async fn source_pause_settles_before_copy_or_intent_and_fences_uncertain_rollback() {
    for successor in [false, true] {
        for fail_rollback in [false, true] {
            let f = fixture(true);
            let prepared = WritingSubmissionService::from_app_store(&f.store)
                .prepare(&f.models, request(&f, Uuid::new_v4()))
                .await
                .unwrap();
            let receipt = WritingSubmissionReceipt {
                conversation_id: f.conversation,
                request_id: prepared.record.request_id,
                manuscript_id: f.manuscript,
            };
            if fail_rollback {
                f.store
                    .connection()
                    .unwrap()
                    .authorizer(Some(|context: AuthContext<'_>| match context.action {
                        AuthAction::Transaction {
                            operation: TransactionOperation::Rollback,
                        } => Authorization::Deny,
                        _ => Authorization::Allow,
                    }));
            }
            let mut captured = false;
            let capture = |_: &rusqlite::Connection, _: &std::path::Path, _| {
                captured = true;
                Err(Flow::Need("read-project"))
            };
            let result = if successor {
                successor_preparation::prepare(
                    &f.store,
                    receipt,
                    prepared.record.context_snapshot_id,
                    successor_preparation::Successor {
                        generation: 1,
                        predecessor: Uuid::new_v4(),
                        lease_id: Uuid::new_v4(),
                    },
                    f.manuscript,
                    capture,
                )
            } else {
                preparation::prepare(
                    &f.store,
                    receipt,
                    prepared.record.context_snapshot_id,
                    f.manuscript,
                    capture,
                )
            };
            assert!(captured);
            assert!(!f.store.agent_workspace_directory().exists());
            if fail_rollback {
                assert!(matches!(result, Err(Error::RecoveryRequired)));
                assert!(f.store.connection().is_err());
            } else {
                assert!(matches!(
                    result,
                    Ok(preparation::Attempt::Need("read-project"))
                ));
                let db = f.store.connection().unwrap();
                assert!(db.is_autocommit());
                let count: i64 = db
                .query_row(
                    "select count(*) from writing_agent_workspace_generations where request_id=?1",
                    [receipt.request_id.to_string()],
                    |row| row.get(0),
                )
                .unwrap();
                assert_eq!(count, 0);
            }
        }
    }
}
