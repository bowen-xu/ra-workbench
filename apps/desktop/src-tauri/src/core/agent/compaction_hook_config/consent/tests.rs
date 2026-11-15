use super::*;
use crate::agent::model::AgentHost;
use research_agent_core::protocol::ModelIdentity;
use std::path::Path;
use uuid::Uuid;

fn root() -> InstructionRootIdentity<'static> {
    InstructionRootIdentity {
        canonical_path: Path::new("/native/hooks"),
        device: 1,
        inode: 2,
    }
}
fn key(id: u128) -> GrantKey {
    GrantKey::new(
        root().canonical_path,
        Uuid::from_u128(id),
        AgentHost::Notes,
        ModelIdentity {
            provider_id: "fixture".into(),
            model_id: "model".into(),
            revision: Some("v1".into()),
        },
    )
    .unwrap()
}
fn hash() -> String {
    "ab".repeat(32)
}
fn consent() -> SqliteHookConsent {
    let mut db = Connection::open_in_memory().unwrap();
    crate::storage::migrate(&mut db).unwrap();
    SqliteHookConsent::new(Arc::new(Mutex::new(db)))
}

#[test]
fn retained_handle_observes_approval_revocation_and_version_changes() {
    let consent = consent();
    let key = key(1);
    let handle = consent.load(&key).unwrap();
    assert!(!handle.allows(&key, &root(), &hash()));
    consent.update(&key, 0, &root(), &hash(), true).unwrap();
    assert!(handle.allows(&key, &root(), &hash()));
    assert!(Arc::ptr_eq(&handle, &consent.load(&key).unwrap()));
    consent.revoke(&key, 1).unwrap();
    assert!(!handle.allows(&key, &root(), &hash()));
    assert_eq!(
        consent.update(&key, 1, &root(), &hash(), true),
        Err(TrustError::Conflict)
    );
    consent.load(&key).unwrap();
    assert!(!handle.allows(&key, &root(), &hash()));
    let changed = "cd".repeat(32);
    consent.update(&key, 2, &root(), &changed, true).unwrap();
    assert!(!handle.allows(&key, &root(), &hash()));
    assert!(handle.allows(&key, &root(), &changed));
    assert!(!handle.allows(&self::key(2), &root(), &changed));
}

#[test]
fn failed_revocation_keeps_database_but_closes_live_admission_until_reload() {
    let consent = consent();
    let key = key(1);
    consent.update(&key, 0, &root(), &hash(), true).unwrap();
    let handle = consent.load(&key).unwrap();
    consent.db.lock().unwrap().execute_batch("create trigger reject_consent before update on agent_compaction_hook_trust begin select raise(abort,'fixture'); end;").unwrap();
    assert_eq!(consent.revoke(&key, 1), Err(TrustError::Storage));
    assert!(!handle.allows(&key, &root(), &hash()));
    let db = consent.db.lock().unwrap();
    assert!(repository::read(&db, &key).unwrap().unwrap().granted);
    drop(db);
    consent.load(&key).unwrap();
    assert!(handle.allows(&key, &root(), &hash()));
}

#[test]
fn pending_storage_operation_denies_without_waiting_and_preserves_other_scopes() {
    let consent = Arc::new(consent());
    let key = key(1);
    let other = self::key(2);
    consent.update(&key, 0, &root(), &hash(), true).unwrap();
    consent.update(&other, 0, &root(), &hash(), true).unwrap();
    let handle = consent.load(&key).unwrap();
    let other_handle = consent.load(&other).unwrap();
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let worker_consent = Arc::clone(&consent);
    let worker_key = key.clone();
    let worker = std::thread::spawn(move || {
        worker_consent
            .apply(&worker_key, |db| {
                started_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                repository::revoke(db, &worker_key, 1).map(Some)
            })
            .unwrap();
    });
    started_rx.recv().unwrap();
    // Worker holds SQLite until this thread releases it: a blocking read would
    // deadlock here. No timeout or scheduler-dependent sleep is needed.
    assert!(!handle.allows(&key, &root(), &hash()));
    assert!(other_handle.allows(&other, &root(), &hash()));
    release_tx.send(()).unwrap();
    worker.join().unwrap();
    assert!(!handle.allows(&key, &root(), &hash()));
}

#[test]
fn capacity_never_evicts_an_owner_handle_or_writes_an_untracked_grant() {
    let mut consent = consent();
    consent.capacity = 2;
    let first = consent.load(&key(1)).unwrap();
    let second = consent.load(&key(2)).unwrap();
    assert_eq!(
        consent.update(&key(3), 0, &root(), &hash(), true),
        Err(TrustError::Storage)
    );
    assert!(
        repository::read(&consent.db.lock().unwrap(), &key(3))
            .unwrap()
            .is_none()
    );
    drop(first);
    consent.update(&key(3), 0, &root(), &hash(), true).unwrap();
    assert!(Arc::ptr_eq(&second, &consent.load(&key(2)).unwrap()));
    assert!(
        consent
            .load(&key(3))
            .unwrap()
            .allows(&key(3), &root(), &hash())
    );
}

#[test]
fn restart_loads_durable_decision_and_reader_contention_fails_closed() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("consent.sqlite");
    let open = || {
        let mut db = Connection::open(&path).unwrap();
        crate::storage::migrate(&mut db).unwrap();
        SqliteHookConsent::new(Arc::new(Mutex::new(db)))
    };
    let consent = open();
    consent.update(&key(1), 0, &root(), &hash(), true).unwrap();
    drop(consent);
    let consent = open();
    let handle = consent.load(&key(1)).unwrap();
    assert!(handle.allows(&key(1), &root(), &hash()));
    let guard = handle.state.lock().unwrap();
    assert!(!handle.allows(&key(1), &root(), &hash()));
    drop(guard);
    consent.revoke(&key(1), 1).unwrap();
    drop(handle);
    drop(consent);
    assert!(
        !open()
            .load(&key(1))
            .unwrap()
            .allows(&key(1), &root(), &hash())
    );
}
