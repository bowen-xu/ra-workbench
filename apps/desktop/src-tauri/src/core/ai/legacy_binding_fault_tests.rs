use rusqlite::{
    Connection,
    hooks::{AuthAction, AuthContext, Authorization},
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

/// Deny schema reads only after SQLite prepares the target write. The flag
/// proves that a failure came from this injection rather than an earlier step.
pub(super) fn deny_schema_after_write(db: &Connection, target: &'static str) -> Arc<AtomicBool> {
    let armed = Arc::new(AtomicBool::new(false));
    let denied = Arc::new(AtomicBool::new(false));
    let hit = denied.clone();
    db.authorizer(Some(move |context: AuthContext<'_>| match context.action {
        AuthAction::Insert { table_name } | AuthAction::Update { table_name, .. }
            if table_name == target =>
        {
            armed.store(true, Ordering::SeqCst);
            Authorization::Allow
        }
        AuthAction::Read {
            table_name: "sqlite_master",
            ..
        } if armed.load(Ordering::SeqCst) => {
            hit.store(true, Ordering::SeqCst);
            Authorization::Deny
        }
        _ => Authorization::Allow,
    }));
    denied
}

pub(super) fn clear_authorizer(db: &Connection) {
    db.authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
}
