use super::*;
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

struct MultiAuthority {
    roots: HashSet<PathBuf>,
    revisions: HashMap<Uuid, String>,
    denied_root: Mutex<Option<PathBuf>>,
    revoked: Mutex<HashSet<Uuid>>,
    grants: AtomicUsize,
}
impl ConfigurationAuthority for MultiAuthority {
    fn authorize_source(
        &self,
        root: &InstructionRootIdentity<'_>,
        _: AgentHost,
        _: &SubmissionRecord,
    ) -> Result<(), HookConfigError> {
        (self.roots.contains(root.canonical_path)
            && self.denied_root.lock().unwrap().as_deref() != Some(root.canonical_path))
        .then_some(())
        .ok_or(HookConfigError)
    }
    fn authorize_command(
        &self,
        root: &InstructionRootIdentity<'_>,
        handler: &ConfiguredHook,
        revision: &str,
        _: AgentHost,
        _: &SubmissionRecord,
    ) -> Result<AuthorizedHookCommand, HookConfigError> {
        assert!(self.roots.contains(root.canonical_path));
        self.grants.fetch_add(1, Ordering::SeqCst);
        if self.revisions.get(&handler.id).map(String::as_str) != Some(revision) {
            return Err(HookConfigError);
        }
        Ok(AuthorizedHookCommand {
            process: handler.candidate()?,
            transcript_path: None,
            launcher: None,
        })
    }
}
impl HookInvocationAuthority for MultiAuthority {
    fn check(
        &self,
        id: Uuid,
        revision: &str,
        _: &CompactionHookRequest,
    ) -> Result<(), HookConfigError> {
        (self.revisions.get(&id).map(String::as_str) == Some(revision)
            && !self.revoked.lock().unwrap().contains(&id))
        .then_some(())
        .ok_or(HookConfigError)
    }
}
fn configuration(root: &std::path::Path, id: Uuid, label: &str) -> String {
    text(root)
        .replace("00000000-0000-0000-0000-000000000012", &id.to_string())
        .replace("frozen-context", label)
}
fn write(root: &std::path::Path, text: &str) {
    std::fs::create_dir_all(root.join("agent")).unwrap();
    std::fs::write(root.join("agent/hooks.toml"), text).unwrap();
}
fn policy(roots: &[PathBuf], texts: &[String]) -> Arc<MultiAuthority> {
    Arc::new(MultiAuthority {
        roots: roots.iter().cloned().collect(),
        revisions: texts
            .iter()
            .map(|text| {
                let parsed = parse(text).unwrap();
                (parsed.handlers()[0].id, parsed.revision().to_owned())
            })
            .collect(),
        denied_root: Mutex::new(None),
        revoked: Mutex::new(HashSet::new()),
        grants: AtomicUsize::new(0),
    })
}

#[tokio::test]
async fn multi_source_factory_preserves_each_revision_order_and_live_revocation() {
    multi_source(false).await;
}

#[tokio::test]
async fn consented_multi_source_preserves_each_grant_order_and_selective_revocation() {
    multi_source(true).await;
}

async fn multi_source(durable: bool) {
    let directories = [tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap()];
    let roots: Vec<_> = directories
        .iter()
        .map(|root| root.path().canonicalize().unwrap())
        .collect();
    let ids = [Uuid::new_v4(), Uuid::new_v4()];
    let texts = [
        configuration(&roots[0], ids[0], "context-one"),
        configuration(&roots[1], ids[1], "context-two"),
    ];
    for (root, text) in roots.iter().zip(&texts) {
        write(root, text);
    }
    let authority = policy(&roots, &texts);
    let configured =
        ConfiguredHookFactory::from_roots(roots.clone(), authority.clone(), Arc::new(Observer))
            .unwrap();
    let record = record();
    let consent = durable.then(|| {
        use crate::agent::{
            compaction_hook_config::consent::SqliteHookConsent,
            instructions::inspect_instruction_root, repository::compaction_hook_trust::GrantKey,
        };
        let mut db = rusqlite::Connection::open_in_memory().unwrap();
        crate::storage::migrate(&mut db).unwrap();
        let consent = Arc::new(SqliteHookConsent::new(Arc::new(Mutex::new(db))));
        for ((root, id), text) in roots.iter().zip(ids).zip(&texts) {
            let key =
                GrantKey::new(root, id, AgentHost::Notes, record.input.model.clone()).unwrap();
            inspect_instruction_root(root, |identity| {
                consent.update(&key, 0, identity, parse(text).unwrap().revision(), true)
            })
            .unwrap()
            .unwrap();
        }
        consent
    });
    let factory: Arc<dyn CompactionHookFactory> = match &consent {
        Some(consent) => Arc::new(configured.with_consent(consent.clone())),
        None => Arc::new(configured),
    };
    let hooks = factory
        .prepare(AgentHost::Notes, &record)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        hooks.handler_ids(CompactionHookStage::SessionStartCompact),
        ids
    );
    assert!(roots.iter().all(|root| !root.join("executed").exists()));
    write(&roots[1], &format!("{}# changed source\n", texts[1]));
    let result = hooks.run_with_context(request(&record)).await;
    assert_eq!(result.outcome, CompactionHookOutcome::Continue);
    assert_eq!(result.additional_contexts, ["context-one", "context-two"]);
    assert_eq!(authority.grants.load(Ordering::SeqCst), 2);
    for root in &roots {
        std::fs::remove_file(root.join("executed")).unwrap();
    }
    if let Some(consent) = consent {
        let key = crate::agent::repository::compaction_hook_trust::GrantKey::new(
            &roots[0],
            ids[0],
            AgentHost::Notes,
            record.input.model.clone(),
        )
        .unwrap();
        consent.revoke(&key, 1).unwrap();
    } else {
        authority.revoked.lock().unwrap().insert(ids[0]);
    }
    assert_eq!(
        hooks
            .run_with_context(request(&record))
            .await
            .additional_contexts,
        ["context-two"]
    );
    assert!(!roots[0].join("executed").exists());
    assert!(roots[1].join("executed").exists());
    assert!(factory.prepare(AgentHost::Notes, &record).await.is_err());
}

#[tokio::test]
async fn duplicate_handlers_or_denied_sources_never_publish_partial_hooks_or_grant_commands() {
    let directories = [tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap()];
    let roots: Vec<_> = directories
        .iter()
        .map(|root| root.path().canonicalize().unwrap())
        .collect();
    let id = Uuid::new_v4();
    let texts = [
        configuration(&roots[0], id, "one"),
        configuration(&roots[1], id, "two"),
    ];
    for (root, text) in roots.iter().zip(&texts) {
        write(root, text);
    }
    let authority = policy(&roots, &texts);
    let factory =
        ConfiguredHookFactory::from_roots(roots.clone(), authority.clone(), Arc::new(Observer))
            .unwrap();
    assert!(factory.prepare(AgentHost::Notes, &record()).await.is_err());
    assert_eq!(authority.grants.load(Ordering::SeqCst), 0);
    *authority.denied_root.lock().unwrap() = Some(roots[1].clone());
    assert!(factory.prepare(AgentHost::Notes, &record()).await.is_err());
    assert_eq!(authority.grants.load(Ordering::SeqCst), 0);
    assert!(roots.iter().all(|root| !root.join("executed").exists()));
    assert!(
        ConfiguredHookFactory::from_roots(
            vec![roots[0].clone(), roots[0].clone()],
            authority.clone(),
            Arc::new(Observer)
        )
        .is_err()
    );
    assert!(ConfiguredHookFactory::from_roots(vec![], authority, Arc::new(Observer)).is_err());
}
