use super::*;
use research_agent_core::instructions::InstructionDisposition;
use std::fs;

fn config(f: &Fixture, body: &str) {
    let root = f.store.agent_configuration_directory();
    fs::create_dir_all(root.join("agent")).unwrap();
    fs::write(root.join("agent/config.toml"), body).unwrap();
}
const PROFILES: &str = "schema_version = 1\n[profiles.note]\ntext = 'Page instructions'\n[profiles.reading_note]\ntext = 'Reading instructions'\n";

#[tokio::test]
async fn profiles_are_scoped_frozen_and_restored_without_current_configuration() {
    let f = reading_fixture();
    config(&f, PROFILES);
    let page_request = Uuid::new_v4();
    let read_request = Uuid::new_v4();
    let page = prepare(&f.store, &f.models, request(&f, page_request))
        .await
        .unwrap();
    let read = reading::prepare(&f.store, &f.models, reading_request(&f, read_request))
        .await
        .unwrap();
    assert_eq!(page.input.instructions[0].text, "Page instructions");
    assert_eq!(read.input.instructions[0].text, "Reading instructions");
    assert_ne!(
        page.input.instructions[0].scope_id,
        read.input.instructions[0].scope_id
    );
    assert_eq!(
        page.input.instructions[0].revision,
        read.input.instructions[0].revision
    );
    assert_eq!(
        read.input.instruction_configs[0].layers[0]
            .instructions
            .max_bytes,
        Some(32768)
    );
    assert_eq!(
        reading::prepare(&f.store, &f.models, reading_request(&f, read_request))
            .await
            .unwrap(),
        read
    );
    config(&f, &format!("{PROFILES}# configuration changed\n"));
    assert_eq!(
        reading::prepare(&f.store, &f.models, reading_request(&f, read_request)).await,
        Err(Error::Conflict)
    );
    let newer = reading::prepare(&f.store, &f.models, reading_request(&f, Uuid::new_v4()))
        .await
        .unwrap();
    assert_eq!(
        newer.input.instructions[0].text,
        read.input.instructions[0].text
    );
    assert_ne!(
        newer.input.instructions[0].revision,
        read.input.instructions[0].revision
    );
    config(&f, "invalid configuration");
    let reopened = AppStore::for_test(
        Connection::open(f.root.path().join("fixture.sqlite")).unwrap(),
        f.root.path().join("data/objects"),
    );
    assert_eq!(
        restore(&reopened, receipt(&f, page_request)).await.unwrap(),
        page
    );
    assert_eq!(
        reading::restore(&reopened, reading_receipt(&f, read_request))
            .await
            .unwrap(),
        read
    );
    // Scope validation cannot relabel another host's instructions even when IDs match.
    let mut mixed = read.input.clone();
    mixed.instructions = page.input.instructions.clone();
    mixed.instruction_configs = page.input.instruction_configs.clone();
    assert_eq!(
        super::super::super::profile::validate(&mixed, Resource::Reading(f.page)),
        Err(Error::InvalidInput)
    );
}

#[tokio::test]
async fn profile_budget_is_utf8_safe_and_omissions_keep_provenance() {
    let f = reading_fixture();
    for (text, budget, expected, disposition) in [
        ("界A界", 4, "界A", InstructionDisposition::Truncated),
        ("界", 1, "", InstructionDisposition::Truncated),
        (
            "Meaningful instruction",
            0,
            "",
            InstructionDisposition::BudgetExcluded,
        ),
        ("  ", 5, "", InstructionDisposition::Empty),
    ] {
        config(
            &f,
            &format!(
                "schema_version=1\n[profiles.reading_note]\ntext='{text}'\nmax_bytes={budget}"
            ),
        );
        let id = Uuid::new_v4();
        let record = reading::prepare(&f.store, &f.models, reading_request(&f, id))
            .await
            .unwrap();
        let fragment = &record.input.instructions[0];
        assert_eq!(fragment.text, expected);
        assert_eq!(fragment.original_bytes, text.len());
        assert_eq!(fragment.disposition, disposition);
        assert_eq!(
            reading::restore(&f.store, reading_receipt(&f, id))
                .await
                .unwrap(),
            record
        );
    }
}

#[tokio::test]
async fn invalid_profiles_fail_before_submission_and_never_echo_configuration() {
    let f = reading_fixture();
    for body in [
        "schema_version=1\n[profiles.note]\ntext='PRIVATE-SYNTHETIC'\nshell=true",
        "schema_version=1\n[profiles.note]\ntext='ok'\nmax_bytes=1048577",
        "schema_version=1\n[profiles.reading_note]\ntext='ok'\nmax_bytes=-1",
        "schema_version=1\n[profiles.unknown]\ntext='PRIVATE-SYNTHETIC'",
    ] {
        config(&f, body);
        let error = reading::prepare(&f.store, &f.models, reading_request(&f, Uuid::new_v4()))
            .await
            .unwrap_err();
        assert_eq!(error, Error::ProfileUnavailable);
        assert!(!error.to_string().contains("PRIVATE-SYNTHETIC"));
        assert_eq!(count(&f), 0);
    }
    config(&f, &"x".repeat(65537));
    assert_eq!(
        prepare(&f.store, &f.models, request(&f, Uuid::new_v4())).await,
        Err(Error::ProfileUnavailable)
    );
    assert_eq!(count(&f), 0);
}

#[cfg(unix)]
#[tokio::test]
async fn profiles_reject_symlinks_and_do_not_read_ambient_agents() {
    use std::os::unix::fs::symlink;
    let f = reading_fixture();
    let root = f.store.agent_configuration_directory();
    fs::create_dir_all(root).unwrap();
    fs::write(
        root.join("AGENTS.md"),
        "Ambient instructions are not a note profile",
    )
    .unwrap();
    let old = reading::prepare(&f.store, &f.models, reading_request(&f, Uuid::new_v4()))
        .await
        .unwrap();
    assert!(old.input.instructions.is_empty());
    config(&f, PROFILES);
    let path = root.join("agent/config.toml");
    fs::rename(&path, root.join("profile-source")).unwrap();
    symlink(root.join("profile-source"), &path).unwrap();
    assert_eq!(
        reading::prepare(&f.store, &f.models, reading_request(&f, Uuid::new_v4())).await,
        Err(Error::ProfileUnavailable)
    );
    assert_eq!(
        reading::restore(&f.store, reading_receipt(&f, old.request_id))
            .await
            .unwrap(),
        old
    );
    assert_eq!(count(&f), 1);
}

#[tokio::test]
async fn runner_uses_original_profile_as_user_content_without_expanding_tools() {
    use crate::agent::{
        execution_profile::AgentExecutionMode,
        note_start,
        repository::SqliteConversationStore,
        service::{AgentSessionService, SubmissionAdmission},
        tasks::TaskCompletion,
    };
    use research_agent_core::model::*;
    use std::{
        collections::VecDeque,
        sync::{Arc, Mutex},
    };
    let f = reading_fixture();
    config(&f, PROFILES);
    let mut selected = request(&f, Uuid::new_v4());
    selected.model = Some(ModelSelection {
        provider_id: "deepseek".into(),
        model_id: "deepseek-v4-pro".into(),
        reasoning_effort: None,
        service_tier: None,
    });
    let record = prepare(&f.store, &f.models, selected).await.unwrap();
    config(
        &f,
        &PROFILES.replace("Page instructions", "New configuration must not leak"),
    );
    let transport = Arc::new(super::super::runner::Transport {
        requests: Mutex::new(vec![]),
        responses: Mutex::new(VecDeque::from([vec![
            ModelStreamEvent::Started {
                response_id: "profile-check".into(),
                actual_model: record.input.model.clone(),
            },
            ModelStreamEvent::TextDelta("Synthetic response".into()),
            ModelStreamEvent::Completed {
                reason: FinishReason::Stop,
                end_turn: None,
            },
        ]])),
    });
    let sessions = Arc::new(AgentSessionService::new(Arc::new(
        SqliteConversationStore::new(f.store.shared_agent_connection(), "profile-fixture".into())
            .unwrap(),
    )));
    let start = note_start::StartRequest {
        conversation_id: f.conversation,
        request_id: record.request_id,
        page_id: f.page,
        context_snapshot_id: record.context_snapshot_id,
        model: record.input.model.clone(),
        mode: AgentExecutionMode::DeepSeekNonThinking,
    };
    let admitted = note_start::start_with_factory(&f.store, sessions, start, |_, _| async {
        Ok(transport.clone() as Arc<dyn ModelTransport>)
    })
    .await
    .unwrap();
    let SubmissionAdmission::Started(owned) = admitted else {
        panic!("expected new turn");
    };
    assert_eq!(owned.task.wait().await, TaskCompletion::Completed);
    let requests = transport.requests.lock().unwrap();
    let request = &requests[0];
    assert!(request.messages.iter().any(|message| matches!(message, ModelMessage::User { content: parts, .. } if parts.iter().any(|part| matches!(part, ContentPart::Text(text) if text.contains("Page instructions"))))));
    assert!(!request.messages.iter().any(|message| matches!(message, ModelMessage::User { content: parts, .. } if parts.iter().any(|part| matches!(part, ContentPart::Text(text) if text.contains("New configuration must not leak"))))));
    assert!(!request.messages.iter().any(|message| matches!(message, ModelMessage::System(text) if text.contains("Page instructions"))));
    assert!(
        !request
            .tools
            .iter()
            .any(|tool| tool.name == "shell" || tool.name == "exec_command")
    );
}
