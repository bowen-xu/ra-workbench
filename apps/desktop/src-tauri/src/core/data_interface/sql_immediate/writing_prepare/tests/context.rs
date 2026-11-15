//! Native decisions leave the same transaction shell through an explicit pause.
use super::*;
use crate::ai::structured_resource_validation::Failure;
use research_workbench_native_sdk::{writing_context::ContextInput, writing_project::Input};

use super::super::super::writing_source_tests::Waiting;

#[test]
fn native_selected_pause_settles_original_transaction_before_returning_input() {
    for compact in [false, true] {
        let f = Fixture::new(compact);
        let need =
            f.db.prepare_writing_context(&f.intent, f.source(), &mut Waiting)
                .unwrap_err();
        assert!(
            matches!(need, Flow::Need(input) if matches!(input.as_ref(), Input::Context { input } if matches!(input.as_ref(), ContextInput::Validate { request } if request.manuscript_id == f.intent.manuscript())))
        );
        assert!(f.db.lock().unwrap().is_autocommit());
        assert_eq!(f.count("agent_submissions"), 0);
        assert_eq!(f.count("data_sql_operation_receipts"), 0);
        assert_eq!(f.count("agent_compaction_preparations"), 0);
        assert_eq!(f.trace(), "previewed");
        // The exact original operation can now be resumed or retried normally.
        f.prepare().unwrap();
        assert_eq!(f.count("agent_submissions"), 1);
        assert_eq!(f.count("data_sql_operation_receipts"), 1);
    }
}

#[test]
fn supplemental_current_file_pause_is_propagated_instead_of_skipped_as_unavailable() {
    let mut f = Fixture::new(false);
    f.intent.request.context.files.clear();
    f.intent.request.resources.push(
        crate::agent::context_session::sources::Resource::ManuscriptFile {
            resource_id: f.intent.manuscript(),
            path: "main.tex".into(),
        },
    );
    let result =
        f.db.prepare_writing_context(&f.intent, f.source(), &mut Waiting);
    assert!(
        matches!(result, Err(Flow::Need(input)) if matches!(input.as_ref(), Input::Context { input } if matches!(input.as_ref(), ContextInput::CurrentFile { manuscript_id, .. } if *manuscript_id == f.intent.manuscript())))
    );
    assert_eq!(f.count("agent_submissions"), 0);
    assert_eq!(f.count("data_sql_operation_receipts"), 0);
    assert_eq!(f.trace(), "previewed");
}

#[test]
fn failed_pause_rollback_fences_database_and_never_returns_a_native_input() {
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization, TransactionOperation};
    let f = Fixture::new(false);
    f.db.lock()
        .unwrap()
        .authorizer(Some(|context: AuthContext<'_>| match context.action {
            AuthAction::Transaction {
                operation: TransactionOperation::Rollback,
            } => Authorization::Deny,
            _ => Authorization::Allow,
        }));
    let result =
        f.db.prepare_writing_context(&f.intent, f.source(), &mut Waiting);
    assert!(
        matches!(result, Err(Flow::Fail(Failure { error: SqlImmediateFailure::Unknown(detail), .. })) if detail.phase == "rollback" && detail.safe_cause == "sql_rollback_failed")
    );
    assert!(f.db.lock().is_err());
    assert_eq!(f.count("agent_submissions"), 0);
    assert_eq!(f.count("data_sql_operation_receipts"), 0);
}
