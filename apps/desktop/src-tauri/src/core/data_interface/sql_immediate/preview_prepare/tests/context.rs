//! A Native pause must not claim a queue item or acknowledge a preparation.
use super::*;
use crate::{
    ai::structured_resource_validation::{Failure, Flow},
    data_interface::database::sql_immediate::writing_source_tests::Waiting,
};

#[test]
fn native_pause_releases_sql_without_claim_and_rechecks_changed_queue() {
    let f = Fixture::new();
    let original = f.queue();
    let result =
        f.db.prepare_queue_preview_context(&f.intent, f.source(), &mut Waiting);
    assert!(matches!(result, Err(Flow::Need(_))));
    assert!(f.db.lock().unwrap().is_autocommit());
    assert_eq!(f.counts(&f.observer()), (0, 0, 0));
    assert_eq!(f.queue(), original);
    f.edit(QueueEdit::Add(QueuedInput {
        id: Uuid::new_v4(),
        text: "Later".into(),
        user_input: vec![],
        origin: None,
    }));
    let result =
        f.db.prepare_queue_preview_context(&f.intent, f.source(), &mut Waiting);
    assert!(matches!(
        result,
        Err(Flow::Fail(Failure {
            error: SqlImmediateFailure::Domain(CatalogError::Conflict),
            ..
        }))
    ));
    assert_eq!(f.counts(&f.observer()), (0, 0, 0));
    assert!(f.queue().preparing().is_none());
}

#[test]
fn native_pause_rollback_failure_fences_queue_and_never_returns_input() {
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization, TransactionOperation};
    let f = Fixture::new();
    f.db.lock()
        .unwrap()
        .authorizer(Some(|context: AuthContext<'_>| match context.action {
            AuthAction::Transaction {
                operation: TransactionOperation::Rollback,
            } => Authorization::Deny,
            _ => Authorization::Allow,
        }));
    let result =
        f.db.prepare_queue_preview_context(&f.intent, f.source(), &mut Waiting);
    assert!(
        matches!(result, Err(Flow::Fail(Failure {error: SqlImmediateFailure::Unknown(detail), ..})) if detail.phase == "rollback")
    );
    assert!(f.db.lock().is_err());
    assert_eq!(f.counts(&f.observer()), (0, 0, 0));
}
