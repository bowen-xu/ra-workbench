use super::*;
use rusqlite::Connection;
use std::os::unix::fs::symlink;

struct Fixture {
    _directory: tempfile::TempDir,
    store: AppStore,
    service: HookConsentService,
}
impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().canonicalize().unwrap();
        let mut db = Connection::open(root.join("native.sqlite")).unwrap();
        crate::storage::migrate(&mut db).unwrap();
        let store = AppStore::for_test(db, root.join("objects"));
        std::fs::create_dir_all(store.agent_configuration_directory().join("agent")).unwrap();
        let service = HookConsentService::from_app_store(&store);
        let fixture = Self {
            _directory: directory,
            store,
            service,
        };
        fixture.write(true);
        fixture
    }
    fn text(&self, enabled: bool) -> String {
        format!(
            "schema_version=1\n[[handlers]]\nid='00000000-0000-0000-0000-000000000012'\nstage='pre_compact'\nenabled={enabled}\nprogram='/bin/sh'\ndirectory='{}'\narguments=['-c', 'touch must-not-execute']\nenvironment={{FIXTURE_ENV='private-fixture'}}\ntimeout_ms=2000\nmax_output_bytes=1024\n",
            self.service.root.display()
        )
    }
    fn write(&self, enabled: bool) {
        std::fs::write(
            self.service.root.join("agent/hooks.toml"),
            self.text(enabled),
        )
        .unwrap();
    }
    fn approve(&self, review: &HandlerReview, scope: Scope) -> ApproveRequest {
        ApproveRequest {
            scope,
            handler_id: review.handler_id,
            expected_revision: review.revision,
            fingerprint: review.fingerprint.clone(),
        }
    }
}
fn scope() -> Scope {
    Scope {
        host: Host::Notes,
        model: ModelIdentity {
            provider_id: "fixture".into(),
            model_id: "model".into(),
            revision: Some("v1".into()),
        },
    }
}
fn id() -> Uuid {
    Uuid::from_u128(0x12)
}

#[test]
fn stop_stage_review_and_approval_are_bound_to_the_exact_configuration() {
    let fixture = Fixture::new();
    let path = fixture.service.root.join("agent/hooks.toml");
    let stop = fixture
        .text(true)
        .replace("stage='pre_compact'", "stage='stop'");
    std::fs::write(&path, &stop).unwrap();
    let review = fixture.service.review(scope()).unwrap().handlers.remove(0);
    assert_eq!(review.stage, "stop");
    let stale = fixture.approve(&review, scope());
    fixture
        .service
        .approve(fixture.approve(&review, scope()))
        .unwrap();
    let value = serde_json::to_string(&fixture.service.review(scope()).unwrap()).unwrap();
    assert!(!value.contains("must-not-execute"));
    std::fs::write(&path, stop.replace("stage='stop'", "stage='subagent_stop'")).unwrap();
    let next = fixture.service.review(scope()).unwrap().handlers.remove(0);
    assert_eq!(next.stage, "subagentStop");
    assert!(!next.matches_current);
    assert!(matches!(
        fixture.service.approve(stale),
        Err(Error::Conflict)
    ));
    assert!(!fixture.service.root.join("must-not-execute").exists());
}

#[test]
fn settings_approval_updates_the_factory_controller_without_executing_commands() {
    let fixture = Fixture::new();
    let consent = fixture.service.consent();
    let key = fixture.service.key(&scope(), id()).unwrap();
    let held = consent.load(&key).unwrap();
    let review = fixture.service.review(scope()).unwrap().handlers.remove(0);
    assert_eq!(review.revision, 0);
    assert!(!review.granted && !review.matches_current);
    let approved = fixture
        .service
        .approve(fixture.approve(&review, scope()))
        .unwrap();
    assert_eq!((approved.revision, approved.granted), (1, true));
    let review = fixture.service.review(scope()).unwrap().handlers.remove(0);
    assert!(review.granted && review.matches_current);
    let snapshot = fixture.service.snapshot().unwrap().unwrap();
    assert!(held.allows(
        &key,
        &snapshot.identity(),
        snapshot.configuration.revision()
    ));
    fixture
        .service
        .revoke(RevokeRequest {
            scope: scope(),
            handler_id: id(),
            expected_revision: 1,
        })
        .unwrap();
    assert!(!held.allows(
        &key,
        &snapshot.identity(),
        snapshot.configuration.revision()
    ));
    assert!(!fixture.service.root.join("must-not-execute").exists());
}

#[test]
fn stale_file_root_host_model_and_ledger_versions_cannot_approve() {
    let fixture = Fixture::new();
    let review = fixture.service.review(scope()).unwrap().handlers.remove(0);
    let mut foreign = scope();
    foreign.host = Host::Writing;
    assert!(matches!(
        fixture.service.approve(fixture.approve(&review, foreign)),
        Err(Error::Conflict)
    ));
    let mut foreign = scope();
    foreign.model.revision = Some("v2".into());
    assert!(matches!(
        fixture.service.approve(fixture.approve(&review, foreign)),
        Err(Error::Conflict)
    ));
    std::fs::write(
        fixture.service.root.join("agent/hooks.toml"),
        format!("{}# changed\n", fixture.text(true)),
    )
    .unwrap();
    assert!(matches!(
        fixture.service.approve(fixture.approve(&review, scope())),
        Err(Error::Conflict)
    ));
    fixture.write(true);
    let old_root = fixture.service.root.with_file_name("old-config");
    std::fs::rename(&fixture.service.root, &old_root).unwrap();
    std::fs::create_dir_all(fixture.service.root.join("agent")).unwrap();
    fixture.write(true);
    assert!(matches!(
        fixture.service.approve(fixture.approve(&review, scope())),
        Err(Error::Conflict)
    ));
    let review = fixture.service.review(scope()).unwrap().handlers.remove(0);
    fixture
        .service
        .approve(fixture.approve(&review, scope()))
        .unwrap();
    assert!(matches!(
        fixture.service.approve(fixture.approve(&review, scope())),
        Err(Error::Conflict)
    ));
    let current = fixture.service.review(scope()).unwrap().handlers.remove(0);
    assert_eq!(current.revision, 1);
    assert!(current.granted && current.matches_current);
}

#[test]
fn disabled_or_linked_sources_never_approve_and_missing_source_can_be_revoked() {
    let fixture = Fixture::new();
    fixture.write(false);
    let disabled = fixture.service.review(scope()).unwrap().handlers.remove(0);
    assert!(!disabled.enabled);
    assert!(matches!(
        fixture.service.approve(fixture.approve(&disabled, scope())),
        Err(Error::Conflict)
    ));
    fixture.write(true);
    let review = fixture.service.review(scope()).unwrap().handlers.remove(0);
    fixture
        .service
        .approve(fixture.approve(&review, scope()))
        .unwrap();
    std::fs::remove_file(fixture.service.root.join("agent/hooks.toml")).unwrap();
    symlink(
        "/outside/unopened",
        fixture.service.root.join("agent/hooks.toml"),
    )
    .unwrap();
    assert!(matches!(
        fixture.service.review(scope()),
        Err(Error::Unavailable)
    ));
    std::fs::remove_dir_all(&fixture.service.root).unwrap();
    assert!(fixture.service.review(scope()).unwrap().handlers.is_empty());
    let revoked = fixture
        .service
        .revoke(RevokeRequest {
            scope: scope(),
            handler_id: id(),
            expected_revision: 1,
        })
        .unwrap();
    assert_eq!((revoked.revision, revoked.granted), (2, false));
    let restarted = HookConsentService::from_app_store(&fixture.store);
    let state = restarted
        .consent
        .read(&restarted.key(&scope(), id()).unwrap())
        .unwrap()
        .unwrap();
    assert_eq!((state.revision, state.granted), (2, false));
}

#[test]
fn wire_rejects_paths_and_plans_and_review_discloses_metadata_only() {
    let fixture = Fixture::new();
    let review = fixture.service.review(scope()).unwrap();
    let value = serde_json::to_value(review).unwrap();
    assert_eq!(value["schemaVersion"], 1);
    assert_eq!(value["handlers"][0]["stage"], "preCompact");
    assert_eq!(value["handlers"][0].as_object().unwrap().len(), 7);
    let wire = value.to_string();
    for private in [
        "private-fixture",
        "must-not-execute",
        "environment",
        fixture.service.root.to_str().unwrap(),
    ] {
        assert!(!wire.contains(private));
    }
    let scope_wire = serde_json::to_value(scope()).unwrap();
    let valid = serde_json::json!({"scope":scope_wire,"handlerId":id(),"expectedRevision":0,"fingerprint":"ab".repeat(32)});
    assert!(serde_json::from_value::<ApproveRequest>(valid.clone()).is_ok());
    for field in ["path", "command", "program", "environment", "sourceRoot"] {
        let mut altered = valid.clone();
        altered[field] = serde_json::json!("not-accepted");
        assert!(serde_json::from_value::<ApproveRequest>(altered).is_err());
    }
    let mut altered = valid;
    altered["scope"]["model"]["path"] = serde_json::json!("not-accepted");
    assert!(serde_json::from_value::<ApproveRequest>(altered).is_err());
    assert!(matches!(
        fixture.service.revoke(RevokeRequest {
            scope: scope(),
            handler_id: Uuid::nil(),
            expected_revision: 1
        }),
        Err(Error::InvalidInput)
    ));
    let mut invalid = scope();
    invalid.model.model_id.clear();
    assert!(matches!(
        fixture.service.review(invalid),
        Err(Error::InvalidInput)
    ));
}

#[test]
fn failed_settings_revocation_reports_failure_and_keeps_live_admission_closed() {
    let fixture = Fixture::new();
    let review = fixture.service.review(scope()).unwrap().handlers.remove(0);
    fixture
        .service
        .approve(fixture.approve(&review, scope()))
        .unwrap();
    let key = fixture.service.key(&scope(), id()).unwrap();
    let held = fixture.service.consent.load(&key).unwrap();
    let snapshot = fixture.service.snapshot().unwrap().unwrap();
    fixture.store.connection().unwrap().execute_batch("create trigger reject_settings before update on agent_compaction_hook_trust begin select raise(abort,'fixture'); end;").unwrap();
    let error = fixture
        .service
        .revoke(RevokeRequest {
            scope: scope(),
            handler_id: id(),
            expected_revision: 1,
        })
        .err()
        .unwrap();
    assert_eq!(error, Error::Unavailable);
    assert_eq!(serde_json::to_string(&error).unwrap(), "\"unavailable\"");
    assert!(!held.allows(
        &key,
        &snapshot.identity(),
        snapshot.configuration.revision()
    ));
    let recovered = fixture.service.review(scope()).unwrap().handlers.remove(0);
    assert_eq!(recovered.revision, 1);
    assert!(recovered.granted && recovered.matches_current);
}
