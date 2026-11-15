use super::*;

fn omitted() -> String {
    text("/unused").replace("directory='/unused'\n", "")
}

#[test]
fn omitted_directory_requires_explicit_native_binding_and_changes_consent_version() {
    let config = parse(&omitted()).unwrap();
    let handler = &config.handlers()[0];
    assert!(handler.candidate().is_err());
    let candidate = handler.candidate_in(Path::new("/native/work")).unwrap();
    assert_eq!(candidate.directory, Path::new("/native/work"));
    assert_eq!(candidate.program, Path::new("/bin/sh"));
    assert_ne!(
        config.revision(),
        parse(&text("/native/work")).unwrap().revision()
    );
    for invalid in ["relative", "", "/native/../outside", "/native/./work"] {
        assert!(
            handler.candidate_in(Path::new(invalid)).is_err(),
            "{invalid}"
        );
    }
}

#[test]
fn explicit_directory_must_match_native_scope_and_invalid_values_stay_invalid() {
    let config = parse(&text("/native/work")).unwrap();
    let handler = &config.handlers()[0];
    assert!(handler.candidate_in(Path::new("/native/other")).is_err());
    assert_eq!(
        handler
            .candidate_in(Path::new("/native/work"))
            .unwrap()
            .directory,
        handler.candidate().unwrap().directory
    );
    for invalid in ["", "relative", "../outside"] {
        assert!(parse(&text(invalid)).is_err());
    }
}

#[tokio::test]
async fn unbound_config_cannot_execute_by_using_process_or_system_cwd() {
    let config = parse(&omitted()).unwrap();
    let result = config.authorize(
        model(),
        |hook, _| {
            Ok(AuthorizedHookCommand {
                process: hook.candidate()?,
                transcript_path: None,
                launcher: None,
            })
        },
        Arc::new(Observer),
        Arc::new(Allow),
    );
    assert!(result.is_err());
}

#[cfg(target_os = "macos")]
#[tokio::test]
async fn native_bound_omitted_cwd_reaches_real_sandbox_and_hook_stdin() {
    use crate::agent::process::{OfflineSandboxPaths, prepare_offline_sandbox};
    let temporary = tempfile::tempdir().unwrap();
    let parent = temporary.path().canonicalize().unwrap();
    let work = parent.join("work");
    let protected = parent.join("originals");
    std::fs::create_dir(&work).unwrap();
    std::fs::create_dir(&protected).unwrap();
    let source = omitted().replace(
        "cat >/dev/null; touch executed; printf approved-context",
        "cat > input.json; touch executed; printf approved-context",
    );
    let hooks = parse(&source)
        .unwrap()
        .authorize(
            model(),
            |hook, _| {
                Ok(AuthorizedHookCommand {
                    process: prepare_offline_sandbox(
                        hook.candidate_in(&work)?,
                        OfflineSandboxPaths {
                            work: work.clone(),
                            read_only: vec![],
                            protected: vec![protected.clone()],
                            allow_system_metadata: false,
                        },
                    )
                    .map_err(|_| HookConfigError)?,
                    transcript_path: None,
                    launcher: None,
                })
            },
            Arc::new(Observer),
            Arc::new(Allow),
        )
        .unwrap();
    let request = CompactionHookRequest {
        operation_id: Uuid::new_v4(),
        conversation_id: Uuid::new_v4(),
        turn_id: Uuid::new_v4(),
        stage: CompactionHookStage::SessionStartCompact,
        phase: research_agent_core::context::compaction_strategy::CompactionPhase::Manual,
        model: model(),
    };
    let result = hooks.run_with_context(request).await;
    assert_eq!(result.additional_contexts, ["approved-context"]);
    assert!(work.join("executed").exists());
    assert!(!parent.join("executed").exists());
    assert!(!protected.join("executed").exists());
    let input: serde_json::Value =
        serde_json::from_slice(&std::fs::read(work.join("input.json")).unwrap()).unwrap();
    assert_eq!(input["cwd"], work.to_str().unwrap());
}
