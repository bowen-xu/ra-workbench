use super::*;
#[path = "tests/native_directory.rs"]
mod native_directory;
#[path = "tests/revocation.rs"]
mod revocation;

#[test]
fn subagent_matcher_uses_codex_exact_and_regex_rules() {
    for matcher in [None, Some(""), Some("*")] {
        assert!(matches_hook_matcher(matcher, "default"));
    }
    assert!(matches_hook_matcher(Some("default|worker"), "default"));
    assert!(!matches_hook_matcher(
        Some("default|worker"),
        "default_extra"
    ));
    assert!(matches_hook_matcher(Some("^def.*$"), "default"));
    assert!(!matches_hook_matcher(Some("^def.*$"), "worker"));
    assert!(!matches_hook_matcher(Some("["), "default"));
}

#[test]
fn legacy_compaction_factory_does_not_silently_ignore_matcher() {
    let configuration = format!("{}matcher='auto'\n", text("/tmp"));
    let config = parse(&configuration).unwrap();
    assert!(
        config
            .authorize(
                model(),
                |_, _| panic!("matcher must not be silently authorized"),
                Arc::new(Observer),
                Arc::new(Allow)
            )
            .is_err()
    );
}

struct RefuseLaunch;
impl HookProcessLauncher for RefuseLaunch {
    fn launch<'a>(
        &'a self,
        _: &'a CompactionHookRequest,
        _: ProcessSpec,
        _: crate::agent::process::ProcessLimits,
    ) -> HookLaunchFuture<'a> {
        Box::pin(async { Err(HookCommandError) })
    }
}
#[tokio::test]
async fn frozen_config_preserves_native_launcher_and_never_falls_back_after_denial() {
    let root = tempfile::tempdir().unwrap();
    let hooks = parse(&text(root.path().to_str().unwrap()))
        .unwrap()
        .authorize(
            model(),
            |hook, _| {
                Ok(AuthorizedHookCommand {
                    process: hook.candidate()?,
                    transcript_path: None,
                    launcher: Some(Arc::new(RefuseLaunch)),
                })
            },
            Arc::new(Observer),
            Arc::new(Allow),
        )
        .unwrap();
    let result = hooks
        .run_with_context(CompactionHookRequest {
            operation_id: Uuid::new_v4(),
            conversation_id: Uuid::new_v4(),
            turn_id: Uuid::new_v4(),
            stage: CompactionHookStage::SessionStartCompact,
            phase: research_agent_core::context::compaction_strategy::CompactionPhase::Manual,
            model: model(),
        })
        .await;
    assert!(result.additional_contexts.is_empty());
    assert!(!root.path().join("executed").exists());
}

struct Observer;
struct Allow;
impl HookInvocationAuthority for Allow {
    fn check(&self, _: Uuid, _: &str, _: &CompactionHookRequest) -> Result<(), HookConfigError> {
        Ok(())
    }
}
impl HookHandlerObserver for Observer {
    fn notify(&self, _: Uuid, _: &CompactionHookRequest, _: CompactionHookNotification) {}
}
fn text(directory: &str) -> String {
    format!(
        "schema_version=1\n[[handlers]]\nid='00000000-0000-0000-0000-000000000010'\nstage='session_start_compact'\nenabled=true\nprogram='/bin/sh'\ndirectory='{directory}'\narguments=['-c', 'cat >/dev/null; touch executed; printf approved-context']\ntimeout_ms=2000\nmax_output_bytes=1024\n"
    )
}
fn model() -> ModelIdentity {
    ModelIdentity {
        provider_id: "test".into(),
        model_id: "model".into(),
        revision: Some("1".into()),
    }
}

#[tokio::test]
async fn frozen_approved_config_runs_only_on_matching_model_without_rereading_source() {
    let root = tempfile::tempdir().unwrap();
    let text = text(root.path().to_str().unwrap());
    let config = parse(&text).unwrap();
    let approved = config.revision().to_owned();
    let hooks = config
        .authorize(
            model(),
            |hook, revision| {
                assert_eq!(revision, approved);
                Ok(AuthorizedHookCommand {
                    process: hook.candidate()?,
                    transcript_path: None,
                    launcher: None,
                })
            },
            Arc::new(Observer),
            Arc::new(Allow),
        )
        .unwrap();
    assert!(!root.path().join("executed").exists());
    let mut request = CompactionHookRequest {
        operation_id: Uuid::new_v4(),
        conversation_id: Uuid::new_v4(),
        turn_id: Uuid::new_v4(),
        stage: CompactionHookStage::SessionStartCompact,
        phase: research_agent_core::context::compaction_strategy::CompactionPhase::Manual,
        model: model(),
    };
    request.model.revision = None;
    let failed = hooks.run_with_context(request.clone()).await;
    assert!(failed.additional_contexts.is_empty());
    assert!(!root.path().join("executed").exists());
    // Source changes do not replace this already-authorized snapshot.
    let changed = parse(&(text + "# changed source\n")).unwrap();
    assert_ne!(changed.revision(), approved);
    request.model = model();
    assert_eq!(
        hooks.run_with_context(request).await.additional_contexts,
        ["approved-context"]
    );
    assert!(root.path().join("executed").exists());
    assert!(
        changed
            .authorize(
                model(),
                |_, revision| {
                    assert_ne!(revision, approved);
                    Err(HookConfigError)
                },
                Arc::new(Observer),
                Arc::new(Allow)
            )
            .is_err()
    );
}

#[test]
fn disabled_defaults_never_request_command_authorization_and_malformed_fields_reject() {
    let text = text("/tmp");
    let disabled = parse(&text.replace("enabled=true\n", "")).unwrap();
    let hooks = disabled
        .authorize(
            model(),
            |_, _| panic!("disabled command must not be granted"),
            Arc::new(Observer),
            Arc::new(Allow),
        )
        .unwrap();
    assert!(
        hooks
            .handler_ids(CompactionHookStage::SessionStartCompact)
            .is_empty()
    );
    for changed in [
        text.replace("schema_version=1", "schema_version=2"),
        text.replace("program='/bin/sh'", "program='sh'"),
        text.replace("timeout_ms=2000", "timeout_ms=0"),
        text.replace("max_output_bytes=1024", "max_output_bytes=1048577"),
        text.replace("stage='session_start_compact'", "stage='unknown'"),
        text.replace("enabled=true", "enabled=true\ntrusted=true"),
        format!(
            "{text}{}",
            text.split_once("[[handlers]]")
                .unwrap()
                .1
                .replace("\nid=", "\n[[handlers]]\nid=")
        ),
    ] {
        assert!(parse(&changed).is_err());
    }
    assert!(parse(&"x".repeat(256 * 1024 + 1)).is_err());
}

#[test]
fn authorized_source_is_bounded_optional_and_rejects_links_before_parsing() {
    let root = tempfile::tempdir().unwrap();
    let root = root.path().canonicalize().unwrap();
    assert!(source::load(&root, |_| Ok(())).unwrap().is_none());
    std::fs::create_dir(root.join("agent")).unwrap();
    std::fs::write(root.join("agent/hooks.toml"), text(root.to_str().unwrap())).unwrap();
    assert!(source::load(&root, |_| Err(HookConfigError)).is_err());
    let loaded = source::load(&root, |identity| {
        assert_eq!(identity.canonical_path, root);
        Ok(())
    })
    .unwrap()
    .unwrap();
    assert_eq!(loaded.handlers().len(), 1);
    assert!(!root.join("executed").exists());
    std::fs::remove_file(root.join("agent/hooks.toml")).unwrap();
    std::fs::write(root.join("outside.toml"), "schema_version=1").unwrap();
    std::os::unix::fs::symlink(root.join("outside.toml"), root.join("agent/hooks.toml")).unwrap();
    assert!(source::load(&root, |_| Ok(())).is_err());
}

#[tokio::test]
async fn host_rewritten_plan_is_frozen_and_partial_authorization_never_executes() {
    let root = tempfile::tempdir().unwrap();
    let text = text(root.path().to_str().unwrap());
    let first = parse(&text).unwrap();
    let second = text.split_once("[[handlers]]").unwrap().1.replace(
        "00000000-0000-0000-0000-000000000010",
        "00000000-0000-0000-0000-000000000011",
    );
    let both = parse(&format!("{text}\n[[handlers]]{second}")).unwrap();
    let mut approvals = 0;
    assert!(
        both.authorize(
            model(),
            |hook, _| {
                approvals += 1;
                if approvals == 2 {
                    return Err(HookConfigError);
                }
                Ok(AuthorizedHookCommand {
                    process: hook.candidate()?,
                    transcript_path: None,
                    launcher: None,
                })
            },
            Arc::new(Observer),
            Arc::new(Allow)
        )
        .is_err()
    );
    assert_eq!(approvals, 2);
    assert!(!root.path().join("executed").exists());

    let hooks = first
        .authorize(
            model(),
            |hook, _| {
                let mut process = hook.candidate()?;
                process.arguments =
                    vec!["-c".into(), "cat >/dev/null; printf rewritten-plan".into()];
                Ok(AuthorizedHookCommand {
                    process,
                    transcript_path: None,
                    launcher: None,
                })
            },
            Arc::new(Observer),
            Arc::new(Allow),
        )
        .unwrap();
    let mut request = CompactionHookRequest {
        operation_id: Uuid::new_v4(),
        conversation_id: Uuid::new_v4(),
        turn_id: Uuid::new_v4(),
        stage: CompactionHookStage::PreCompact,
        phase: research_agent_core::context::compaction_strategy::CompactionPhase::Manual,
        model: model(),
    };
    assert!(
        hooks
            .run_with_context(request.clone())
            .await
            .additional_contexts
            .is_empty()
    );
    request.stage = CompactionHookStage::SessionStartCompact;
    assert_eq!(
        hooks.run_with_context(request).await.additional_contexts,
        ["rewritten-plan"]
    );
    assert!(!root.path().join("executed").exists());
}
