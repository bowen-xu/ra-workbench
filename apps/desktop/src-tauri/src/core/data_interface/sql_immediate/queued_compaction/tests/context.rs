//! Source pauses preserve the original compact queue claim and history CAS.
use super::*;
use crate::{
    ai::structured_resource_validation::{Failure, Flow},
    data_interface::database::sql_immediate::writing_source_tests::Waiting,
};

fn assert_no_preparation(f: &Fixture, request: Uuid) {
    let db = Connection::open(f.root.path().join("queue.sqlite3")).unwrap();
    for (table, field) in [
        ("agent_submissions", "request_id"),
        ("agent_input_queue_receipts", "request_id"),
        ("agent_compaction_preparations", "request_id"),
        ("data_sql_operation_receipts", "operation_id"),
    ] {
        let count: u64 = db
            .query_row(
                &format!("select count(*) from {table} where {field}=?1"),
                [request.to_string()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 0, "{table}");
    }
}

#[test]
fn native_pause_releases_sql_without_claim_and_rechecks_history_revision() {
    let f = Fixture::new();
    let command = f.intent(Uuid::new_v4(), None);
    let original = f.q();
    let result =
        f.db.prepare_queued_compaction_context(&command, f.source(&command), &mut Waiting);
    assert!(matches!(result, Err(Flow::Need(_))));
    assert!(f.db.lock().unwrap().is_autocommit());
    assert_no_preparation(&f, command.request_id);
    assert_eq!(f.q(), original);
    f.db.lock()
        .unwrap()
        .execute(
            "update agent_conversations set durable_count=durable_count+1 where id=?1",
            [f.conversation.to_string()],
        )
        .unwrap();
    let result =
        f.db.prepare_queued_compaction_context(&command, f.source(&command), &mut Waiting);
    assert!(matches!(
        result,
        Err(Flow::Fail(Failure {
            error: SqlImmediateFailure::Domain(CatalogError::Conflict),
            ..
        }))
    ));
    assert_eq!(f.q(), original);
    assert_no_preparation(&f, command.request_id);
}

#[test]
fn native_pause_rollback_failure_fences_compaction_and_never_returns_input() {
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization, TransactionOperation};
    let f = Fixture::new();
    let command = f.intent(Uuid::new_v4(), None);
    f.db.lock()
        .unwrap()
        .authorizer(Some(|context: AuthContext<'_>| match context.action {
            AuthAction::Transaction {
                operation: TransactionOperation::Rollback,
            } => Authorization::Deny,
            _ => Authorization::Allow,
        }));
    let result =
        f.db.prepare_queued_compaction_context(&command, f.source(&command), &mut Waiting);
    assert!(
        matches!(result, Err(Flow::Fail(Failure { error: SqlImmediateFailure::Unknown(detail), .. })) if detail.phase == "rollback")
    );
    assert!(f.db.lock().is_err());
    assert_no_preparation(&f, command.request_id);
}
