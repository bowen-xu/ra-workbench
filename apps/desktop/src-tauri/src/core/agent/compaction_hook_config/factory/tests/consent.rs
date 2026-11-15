use super::*;
use crate::agent::{
    compaction_hook_config::consent::SqliteHookConsent, instructions::inspect_instruction_root,
    repository::compaction_hook_trust::GrantKey,
};
use rusqlite::Connection;
use std::sync::Mutex;

struct Fixture {
    directory: tempfile::TempDir,
    root: PathBuf,
    record: SubmissionRecord,
    authority: Arc<Authority>,
    consent: Arc<SqliteHookConsent>,
    factory: consented::ConsentedHookFactory,
}
impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().canonicalize().unwrap();
        std::fs::create_dir(root.join("agent")).unwrap();
        let text = text(&root);
        std::fs::write(root.join("agent/hooks.toml"), &text).unwrap();
        let record = record();
        let authority = Arc::new(Authority {
            root: root.clone(),
            record: record.clone(),
            revision: parse(&text).unwrap().revision().into(),
            source_allowed: AtomicBool::new(true),
            invocation_allowed: AtomicBool::new(true),
            grants: AtomicUsize::new(0),
            checks: AtomicUsize::new(0),
        });
        let mut db = Connection::open(directory.path().join("native.sqlite")).unwrap();
        crate::storage::migrate(&mut db).unwrap();
        let consent = Arc::new(SqliteHookConsent::new(Arc::new(Mutex::new(db))));
        let factory =
            ConfiguredHookFactory::new(root.clone(), authority.clone(), Arc::new(Observer))
                .unwrap()
                .with_consent(consent.clone());
        Self {
            directory,
            root,
            record,
            authority,
            consent,
            factory,
        }
    }
    fn key(&self, host: AgentHost, model: ModelIdentity) -> GrantKey {
        GrantKey::new(&self.root, Uuid::from_u128(0x12), host, model).unwrap()
    }
    fn approve(&self, host: AgentHost, model: ModelIdentity, revision: &str, expected: u64) {
        let key = self.key(host, model);
        inspect_instruction_root(&self.root, |root| {
            self.consent.update(&key, expected, root, revision, true)
        })
        .unwrap()
        .unwrap();
    }
    async fn prepare(&self) -> Result<PreparedCompactionHooks, ModelError> {
        self.factory.prepare(AgentHost::Notes, &self.record).await
    }
}

#[tokio::test]
async fn durable_consent_gates_real_command_and_revokes_already_prepared_owner() {
    let fixture = Fixture::new();
    assert!(fixture.prepare().await.is_err());
    assert_eq!(fixture.authority.grants.load(Ordering::SeqCst), 0);
    fixture.approve(
        AgentHost::Notes,
        fixture.record.input.model.clone(),
        &fixture.authority.revision,
        0,
    );
    let hooks = fixture.prepare().await.unwrap().unwrap();
    assert!(!fixture.root.join("executed").exists());
    assert_eq!(
        hooks
            .run_with_context(request(&fixture.record))
            .await
            .additional_contexts,
        ["frozen-context"]
    );
    std::fs::remove_file(fixture.root.join("executed")).unwrap();
    let key = fixture.key(AgentHost::Notes, fixture.record.input.model.clone());
    fixture.consent.revoke(&key, 1).unwrap();
    let revoked = hooks.run_with_context(request(&fixture.record)).await;
    assert_eq!(revoked.outcome, CompactionHookOutcome::Continue);
    assert!(revoked.additional_contexts.is_empty());
    assert!(!fixture.root.join("executed").exists());
    assert!(fixture.prepare().await.is_err());
    // The underlying native policy only sees the successfully admitted command.
    assert_eq!(fixture.authority.checks.load(Ordering::SeqCst), 1);
    assert!(fixture.directory.path().join("native.sqlite").exists());
}

#[tokio::test]
async fn approvals_are_bound_to_host_exact_model_and_opened_root_identity() {
    let fixture = Fixture::new();
    fixture.approve(
        AgentHost::Writing,
        fixture.record.input.model.clone(),
        &fixture.authority.revision,
        0,
    );
    let mut foreign = fixture.record.input.model.clone();
    foreign.revision = Some("other".into());
    fixture.approve(AgentHost::Notes, foreign, &fixture.authority.revision, 0);
    assert!(fixture.prepare().await.is_err());
    let key = fixture.key(AgentHost::Notes, fixture.record.input.model.clone());
    inspect_instruction_root(&fixture.root, |root| {
        let replaced = InstructionRootIdentity {
            canonical_path: root.canonical_path,
            device: root.device,
            inode: root.inode + 1,
        };
        fixture
            .consent
            .update(&key, 0, &replaced, &fixture.authority.revision, true)
    })
    .unwrap()
    .unwrap();
    assert!(fixture.prepare().await.is_err());
    assert_eq!(fixture.authority.grants.load(Ordering::SeqCst), 0);
    assert!(!fixture.root.join("executed").exists());
}

#[tokio::test]
async fn changed_file_cannot_inherit_consent_and_host_can_still_refuse() {
    let fixture = Fixture::new();
    fixture.approve(
        AgentHost::Notes,
        fixture.record.input.model.clone(),
        &fixture.authority.revision,
        0,
    );
    fixture
        .authority
        .source_allowed
        .store(false, Ordering::SeqCst);
    assert!(fixture.prepare().await.is_err());
    fixture
        .authority
        .source_allowed
        .store(true, Ordering::SeqCst);
    let hooks = fixture.prepare().await.unwrap().unwrap();
    fixture
        .authority
        .invocation_allowed
        .store(false, Ordering::SeqCst);
    assert!(
        hooks
            .run_with_context(request(&fixture.record))
            .await
            .additional_contexts
            .is_empty()
    );
    assert!(!fixture.root.join("executed").exists());
    fixture
        .authority
        .invocation_allowed
        .store(true, Ordering::SeqCst);
    std::fs::write(
        fixture.root.join("agent/hooks.toml"),
        format!("{}# changed\n", text(&fixture.root)),
    )
    .unwrap();
    assert!(fixture.prepare().await.is_err());
    assert_eq!(fixture.authority.grants.load(Ordering::SeqCst), 1);
}
