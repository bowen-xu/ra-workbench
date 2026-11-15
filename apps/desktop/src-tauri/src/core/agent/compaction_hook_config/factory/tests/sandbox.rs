use super::*;
use crate::agent::{
    compaction_hook_config::consent::SqliteHookConsent, instructions::inspect_instruction_root,
    process::OfflineSandboxPaths, repository::compaction_hook_trust::GrantKey,
};
use std::{os::unix::fs::symlink, sync::Mutex};

struct Fixture {
    _directory: tempfile::TempDir,
    root: PathBuf,
    source: PathBuf,
    record: SubmissionRecord,
    authority: Arc<Authority>,
    consent: Arc<SqliteHookConsent>,
}
impl Fixture {
    fn new(script: &str) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().canonicalize().unwrap();
        for name in ["source/agent", "work", "read-only", "protected", "outside"] {
            std::fs::create_dir_all(root.join(name)).unwrap();
        }
        std::fs::write(root.join("protected/input"), "private-fixture").unwrap();
        std::fs::write(root.join("read-only/input"), "allowed-fixture").unwrap();
        symlink(root.join("protected/input"), root.join("work/private-link")).unwrap();
        let source = root.join("source");
        let config = format!(
            "schema_version=1\n[[handlers]]\nid='00000000-0000-0000-0000-000000000012'\nstage='session_start_compact'\nenabled=true\nprogram='/bin/sh'\ndirectory='{}'\narguments=['-c', '''{}''']\ntimeout_ms=4000\nmax_output_bytes=4096\n",
            root.join("work").display(),
            script
        );
        std::fs::write(source.join("agent/hooks.toml"), &config).unwrap();
        let record = record();
        let authority = Arc::new(Authority {
            root: source.clone(),
            record: record.clone(),
            revision: parse(&config).unwrap().revision().into(),
            source_allowed: AtomicBool::new(true),
            invocation_allowed: AtomicBool::new(true),
            grants: AtomicUsize::new(0),
            checks: AtomicUsize::new(0),
        });
        let mut db = rusqlite::Connection::open(root.join("protected/consent.sqlite")).unwrap();
        crate::storage::migrate(&mut db).unwrap();
        let consent = Arc::new(SqliteHookConsent::new(Arc::new(Mutex::new(db))));
        let key = GrantKey::new(
            &source,
            Uuid::from_u128(0x12),
            AgentHost::Notes,
            record.input.model.clone(),
        )
        .unwrap();
        inspect_instruction_root(&source, |identity| {
            consent.update(&key, 0, identity, &authority.revision, true)
        })
        .unwrap()
        .unwrap();
        Self {
            _directory: directory,
            root,
            source,
            record,
            authority,
            consent,
        }
    }
    fn factory(&self, work: PathBuf, protected: Vec<PathBuf>) -> consented::ConsentedHookFactory {
        self.factory_with_authority(work, protected, self.authority.clone())
    }
    fn factory_with_authority(
        &self,
        work: PathBuf,
        protected: Vec<PathBuf>,
        authority: Arc<dyn ConfigurationAuthority>,
    ) -> consented::ConsentedHookFactory {
        let root = self.root.clone();
        let record = self.record.clone();
        let source = self.source.clone();
        ConfiguredHookFactory::new(self.source.clone(), authority, Arc::new(Observer))
            .unwrap()
            .with_offline_sandbox(Arc::new(move |identity, handler, host, submission| {
                assert_eq!(identity.canonical_path, source);
                assert_eq!(submission, &record);
                assert_eq!(handler.id, Uuid::from_u128(0x12));
                assert!(matches!(host, AgentHost::Notes));
                Ok(OfflineSandboxPaths {
                    work: work.clone(),
                    read_only: vec![root.join("read-only")],
                    protected: protected.clone(),
                    allow_system_metadata: false,
                })
            }))
            .with_consent(self.consent.clone())
    }
    fn approved_factory(&self) -> consented::ConsentedHookFactory {
        self.factory(
            self.root.join("work"),
            vec![self.root.join("protected"), self.source.clone()],
        )
    }
}

struct TranscriptAuthority {
    inner: Arc<Authority>,
    path: PathBuf,
}
impl ConfigurationAuthority for TranscriptAuthority {
    fn authorize_source(
        &self,
        root: &InstructionRootIdentity<'_>,
        host: AgentHost,
        record: &SubmissionRecord,
    ) -> Result<(), HookConfigError> {
        self.inner.authorize_source(root, host, record)
    }
    fn authorize_command(
        &self,
        root: &InstructionRootIdentity<'_>,
        handler: &ConfiguredHook,
        revision: &str,
        host: AgentHost,
        record: &SubmissionRecord,
    ) -> Result<AuthorizedHookCommand, HookConfigError> {
        let mut plan = self
            .inner
            .authorize_command(root, handler, revision, host, record)?;
        plan.transcript_path = Some(self.path.clone());
        Ok(plan)
    }
}
impl HookInvocationAuthority for TranscriptAuthority {
    fn check(
        &self,
        id: Uuid,
        revision: &str,
        request: &CompactionHookRequest,
    ) -> Result<(), HookConfigError> {
        self.inner.check(id, revision, request)
    }
}

#[tokio::test]
async fn transcript_path_is_disclosed_only_with_canonical_read_grant() {
    let fixture = Fixture::new("cat > request.json; printf ready");
    let factory = |path| {
        fixture.factory_with_authority(
            fixture.root.join("work"),
            vec![fixture.root.join("protected"), fixture.source.clone()],
            Arc::new(TranscriptAuthority {
                inner: fixture.authority.clone(),
                path,
            }),
        )
    };
    for path in [
        fixture.root.join("protected/input"),
        fixture.root.join("work/private-link"),
        fixture.root.join("outside/missing"),
    ] {
        assert!(
            factory(path)
                .prepare(AgentHost::Notes, &fixture.record)
                .await
                .is_err()
        );
    }
    assert!(!fixture.root.join("work/request.json").exists());
    let transcript = fixture.root.join("read-only/input");
    let factory = factory(transcript.clone());
    let hooks = factory
        .prepare(AgentHost::Notes, &fixture.record)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        hooks
            .run_with_context(request(&fixture.record))
            .await
            .additional_contexts,
        ["ready"]
    );
    let input: serde_json::Value =
        serde_json::from_slice(&std::fs::read(fixture.root.join("work/request.json")).unwrap())
            .unwrap();
    assert_eq!(input["transcript_path"], transcript.to_str().unwrap());
}

#[tokio::test]
async fn consented_hook_uses_seatbelt_for_reads_writes_and_symlink_escape() {
    let fixture = Fixture::new(
        r#"cat >/dev/null
cat ../read-only/input > allowed
if cat ../protected/input >/dev/null 2>/dev/null; then printf unsafe; exit 0; fi
if cat private-link >/dev/null 2>/dev/null; then printf unsafe; exit 0; fi
if /bin/sh -c 'printf changed > ../protected/input' 2>/dev/null; then printf unsafe; exit 0; fi
if /bin/sh -c 'printf escaped > ../outside/marker' 2>/dev/null; then printf unsafe; exit 0; fi
if cat ../source/agent/hooks.toml >/dev/null 2>/dev/null; then printf unsafe; exit 0; fi
printf sandboxed"#,
    );
    let factory = fixture.approved_factory();
    let hooks = factory
        .prepare(AgentHost::Notes, &fixture.record)
        .await
        .unwrap()
        .unwrap();
    assert!(!fixture.root.join("work/allowed").exists());
    let result = hooks.run_with_context(request(&fixture.record)).await;
    assert_eq!(result.additional_contexts, ["sandboxed"]);
    assert_eq!(
        std::fs::read_to_string(fixture.root.join("work/allowed")).unwrap(),
        "allowed-fixture"
    );
    assert_eq!(
        std::fs::read_to_string(fixture.root.join("protected/input")).unwrap(),
        "private-fixture"
    );
    assert!(!fixture.root.join("outside/marker").exists());
}

#[tokio::test]
async fn sandboxed_hook_cannot_connect_to_loopback() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let fixture = Fixture::new(&format!(
        "cat >/dev/null; if /usr/bin/curl --noproxy '*' --connect-timeout 1 --max-time 2 http://{}/ >/dev/null 2>/dev/null; then printf unsafe; else printf blocked; fi",
        listener.local_addr().unwrap()
    ));
    let factory = fixture.approved_factory();
    let hooks = factory
        .prepare(AgentHost::Notes, &fixture.record)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        hooks
            .run_with_context(request(&fixture.record))
            .await
            .additional_contexts,
        ["blocked"]
    );
    assert!(
        matches!(listener.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock)
    );
}

#[tokio::test]
async fn invalid_grants_never_publish_an_unsandboxed_fallback() {
    let fixture = Fixture::new("cat >/dev/null; printf ran > marker; printf unsafe");
    // Configured cwd cannot select the host's writable grant; protected data
    // cannot be included in any work/read scope even when consent is valid.
    for factory in [
        fixture.factory(
            fixture.root.join("outside"),
            vec![fixture.root.join("protected")],
        ),
        fixture.factory(fixture.root.join("work"), vec![fixture.root.join("work")]),
    ] {
        assert!(
            factory
                .prepare(AgentHost::Notes, &fixture.record)
                .await
                .is_err()
        );
    }
    assert!(!fixture.root.join("work/marker").exists());
}
