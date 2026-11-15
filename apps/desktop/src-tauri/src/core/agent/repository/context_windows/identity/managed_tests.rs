use super::*;
use rusqlite::hooks::{AuthAction, AuthContext, Authorization, TransactionOperation};

#[test]
fn known_rejection_releases_owner_but_failed_rollback_fences_it() {
    let root = tempfile::tempdir().unwrap();
    let mut sql = Connection::open(root.path().join("identity.sqlite")).unwrap();
    crate::storage::migrate(&mut sql).unwrap();
    let db = crate::data_interface::Database::open(sql, &root.path().join("log")).unwrap();
    {
        let mut guard = db.lock().unwrap();
        assert_eq!(
            read_or_create_managed(&mut guard, Uuid::nil(), Uuid::now_v7()),
            Err(ManagedIdentityError::Domain(Error::Invalid))
        );
    }
    {
        let mut guard = db.lock().unwrap();
        guard.authorizer(Some(|context: AuthContext<'_>| match context.action {
            AuthAction::Transaction {
                operation: TransactionOperation::Rollback,
            } => Authorization::Deny,
            _ => Authorization::Allow,
        }));
        assert!(matches!(
            read_or_create_managed(&mut guard, Uuid::nil(), Uuid::now_v7()),
            Err(ManagedIdentityError::Persistence(
                ContextPersistenceDiagnostic {
                    phase: "rollback",
                    safe_cause: "sql_rollback_failed",
                    ..
                }
            ))
        ));
    }
    assert!(db.lock().is_err());
}
