use super::*;

fn model() -> ModelIdentity {
    ModelIdentity {
        provider_id: "fixture".into(),
        model_id: "model".into(),
        revision: Some("v1".into()),
    }
}
fn root() -> InstructionRootIdentity<'static> {
    InstructionRootIdentity {
        canonical_path: Path::new("/synthetic/app-config"),
        device: u64::MAX,
        inode: 42,
    }
}
fn key() -> GrantKey {
    GrantKey::new(
        root().canonical_path,
        Uuid::from_u128(1),
        AgentHost::Writing,
        model(),
    )
    .unwrap()
}
fn hash() -> String {
    "ab".repeat(32)
}
fn db() -> Connection {
    let mut db = Connection::open_in_memory().unwrap();
    crate::storage::migrate(&mut db).unwrap();
    db
}

#[test]
fn migration_bytes_are_pinned_and_upgrade_never_invents_a_grant() {
    use sha2::{Digest, Sha256};
    let source = include_str!("../../../../../migrations/0146_agent_compaction_hook_trust.sql");
    assert_eq!(
        format!("{:x}", Sha256::digest(source.as_bytes())),
        "032c21bc3c5dc7b48e307f39922b518a6bc7cdbcd12d2d9501a6cf2920898fbc"
    );
    let mut db = Connection::open_in_memory().unwrap();
    crate::storage_migration_tests::migrate_through(&mut db, 145);
    let old_hashes: Vec<(i64, String)> = db
        .prepare("select version,sha256 from migration_checksums order by version")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    crate::storage::migrate(&mut db).unwrap();
    let count: i64 = db
        .query_row(
            "select count(*) from agent_compaction_hook_trust",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
    for (version, hash) in old_hashes {
        let current: String = db
            .query_row(
                "select sha256 from migration_checksums where version=?1",
                [version],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(current, hash);
    }
}

#[test]
fn upgrade_and_reopen_preserve_consent_and_revocation_tombstones() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("trust.sqlite");
    let mut db = Connection::open(&path).unwrap();
    crate::storage_migration_tests::migrate_through(&mut db, 145);
    crate::storage::migrate(&mut db).unwrap();
    assert_eq!(read(&db, &key()).unwrap(), None);
    let saved = update(&mut db, &key(), 0, &root(), &hash(), true).unwrap();
    assert!(saved.allows(&key(), &root(), &hash()));
    drop(db);
    let mut db = Connection::open(&path).unwrap();
    crate::storage::migrate(&mut db).unwrap();
    assert_eq!(read(&db, &key()).unwrap(), Some(saved));
    let revoked = revoke(&mut db, &key(), 1).unwrap();
    assert_eq!(revoked.revision, 2);
    assert!(!revoked.allows(&key(), &root(), &hash()));
    assert_eq!(
        update(&mut db, &key(), 1, &root(), &hash(), true),
        Err(TrustError::Conflict)
    );
    drop(db);
    let mut db = Connection::open(path).unwrap();
    crate::storage::migrate(&mut db).unwrap();
    assert_eq!(read(&db, &key()).unwrap(), Some(revoked));
    let renewed = update(&mut db, &key(), 2, &root(), &hash(), true).unwrap();
    assert_eq!(renewed.revision, 3);
    assert!(renewed.allows(&key(), &root(), &hash()));
}

#[test]
fn approval_does_not_cross_source_handler_host_model_or_configuration() {
    let mut db = db();
    let saved = update(&mut db, &key(), 0, &root(), &hash(), true).unwrap();
    let mut revised = model();
    revised.revision = Some("v2".into());
    let mut renamed = model();
    renamed.model_id = "other".into();
    let mut supplier = model();
    supplier.provider_id = "other".into();
    let mut undated = model();
    undated.revision = None;
    let others = [
        GrantKey::new(
            root().canonical_path,
            key().handler,
            AgentHost::Writing,
            undated,
        )
        .unwrap(),
        GrantKey::new(
            Path::new("/other"),
            key().handler,
            AgentHost::Writing,
            model(),
        )
        .unwrap(),
        GrantKey::new(
            root().canonical_path,
            Uuid::from_u128(2),
            AgentHost::Writing,
            model(),
        )
        .unwrap(),
        GrantKey::new(
            root().canonical_path,
            key().handler,
            AgentHost::Notes,
            model(),
        )
        .unwrap(),
        GrantKey::new(
            root().canonical_path,
            key().handler,
            AgentHost::Writing,
            revised,
        )
        .unwrap(),
        GrantKey::new(
            root().canonical_path,
            key().handler,
            AgentHost::Writing,
            renamed,
        )
        .unwrap(),
        GrantKey::new(
            root().canonical_path,
            key().handler,
            AgentHost::Writing,
            supplier,
        )
        .unwrap(),
    ];
    for other in others {
        assert_eq!(read(&db, &other).unwrap(), None);
        assert!(!saved.allows(&other, &root(), &hash()));
    }
    assert!(!saved.allows(
        &key(),
        &InstructionRootIdentity {
            inode: 43,
            ..root()
        },
        &hash()
    ));
    assert!(!saved.allows(
        &key(),
        &InstructionRootIdentity {
            device: 1,
            ..root()
        },
        &hash()
    ));
    assert!(!saved.allows(&key(), &root(), &"cd".repeat(32)));
    let changed = update(&mut db, &key(), 1, &root(), &"cd".repeat(32), true).unwrap();
    assert!(!changed.allows(&key(), &root(), &hash()));
    assert!(changed.allows(&key(), &root(), &"cd".repeat(32)));
}

#[test]
fn failed_grant_or_revocation_transaction_keeps_the_previous_decision() {
    let mut db = db();
    let original = update(&mut db, &key(), 0, &root(), &hash(), true).unwrap();
    db.execute_batch("create trigger reject_hook_trust before update on agent_compaction_hook_trust begin select raise(abort,'fixture failure'); end;").unwrap();
    assert_eq!(revoke(&mut db, &key(), 1), Err(TrustError::Storage));
    assert_eq!(
        update(&mut db, &key(), 1, &root(), &"cd".repeat(32), false),
        Err(TrustError::Storage)
    );
    assert_eq!(read(&db, &key()).unwrap(), Some(original));
}

#[test]
fn revoke_needs_no_existing_configuration_and_invalid_inputs_cannot_create_grants() {
    let mut db = db();
    for invalid in ["", "A".repeat(64).as_str(), "a".repeat(63).as_str()] {
        assert_eq!(
            update(&mut db, &key(), 0, &root(), invalid, true),
            Err(TrustError::Invalid)
        );
    }
    assert_eq!(
        update(&mut db, &key(), MAX_REVISION, &root(), &hash(), true),
        Err(TrustError::Invalid)
    );
    assert_eq!(read(&db, &key()).unwrap(), None);
    assert!(
        GrantKey::new(
            Path::new("relative"),
            key().handler,
            AgentHost::Writing,
            model()
        )
        .is_err()
    );
    assert!(
        GrantKey::new(
            root().canonical_path,
            Uuid::nil(),
            AgentHost::Writing,
            model()
        )
        .is_err()
    );
    let mut invalid = model();
    invalid.revision = Some("".into());
    assert!(
        GrantKey::new(
            root().canonical_path,
            key().handler,
            AgentHost::Writing,
            invalid
        )
        .is_err()
    );
    update(&mut db, &key(), 0, &root(), &hash(), true).unwrap();
    // Synthetic root has never existed; revoke does not inspect it or read config.
    assert!(!revoke(&mut db, &key(), 1).unwrap().granted);
    assert_eq!(revoke(&mut db, &key(), 1), Err(TrustError::Conflict));
    assert_eq!(read(&db, &key()).unwrap().unwrap().revision, 2);
}
