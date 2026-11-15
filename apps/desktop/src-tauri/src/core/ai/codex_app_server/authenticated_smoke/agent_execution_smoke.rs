//! Real shared-runner acceptance over an immutable submission and its built-in
//! frozen-source tool. This deliberately exercises the production writing
//! submission entry rather than reproducing its model/tool loop in the smoke test.
use super::*;
use crate::{
    agent::writing_workspace::WritingWorkspaceRequest,
    agent::{
        WritingContextBuffer, WritingContextFile, WritingContextRequest, WritingContextSelection,
        note_application, note_child_commands, note_review, note_start, note_submission,
        reading_note_start, reading_note_submission,
        repository::{SqliteConversationCatalog, create_conversation},
        service::{AgentSessionService, SubmissionAdmission},
        tasks::TaskCompletion,
        writing_submission::{
            WritingExecutionConfirmation, WritingSubmissionReceipt, WritingSubmissionRequest,
            WritingSubmissionService,
        },
    },
    notes::{agent_context::NoteContextRequest, reading_agent_context},
    storage::AppStore,
};
use research_agent_core::{
    conversation::ConversationId,
    protocol::{ItemKind, TurnOutcome},
};
use rusqlite::Connection;
use sha2::{Digest, Sha256};

#[derive(Debug)]
enum AcceptanceError {
    Setup,
    Admission,
    Task,
    Projection,
    MissingToolCall,
    MissingToolResult,
    MissingAnswer,
    ChildToolResultMissing,
    ChildTurnMissing,
    ChildTurnTimeout,
    ChildOutcome,
    ChildAnswerMissing,
    PaperOverview,
    PaperCandidate,
    MissingShellCall,
    MissingCompileCall,
    CompileResult,
    MissingWorkspaceReview,
    WorkspaceReview,
    Proposal,
    Application,
    Undo,
    NoteReview,
    ScopeIsolation,
}

#[tokio::test]
#[ignore = "real model/tool calls; requires explicit opt-in, app-owned login, and closed development application"]
async fn authenticated_chatgpt_agent_execution_reads_frozen_source() {
    let _owner = PROFILE_OWNER.lock().await;
    let (bridge, work) = development_bridge();
    let outcome = tokio::time::timeout(
        Duration::from_secs(180),
        run_shared_agent(bridge.clone(), work.path(), false, false, false, false),
    )
    .await;
    bridge.stop().await;
    outcome
        .expect("shared Agent acceptance timed out")
        .expect("shared Agent acceptance failed");
}

#[tokio::test]
#[ignore = "real model/tool calls; requires explicit opt-in, app-owned login, and closed development application"]
async fn authenticated_chatgpt_agent_execution_spawns_writing_child() {
    let _owner = PROFILE_OWNER.lock().await;
    let (bridge, work) = development_bridge();
    let outcome = tokio::time::timeout(
        Duration::from_secs(150),
        run_shared_agent(bridge.clone(), work.path(), true, false, false, false),
    )
    .await;
    bridge.stop().await;
    outcome
        .expect("shared Agent child acceptance timed out")
        .expect("shared Agent child acceptance failed");
}

#[tokio::test]
#[ignore = "real model/tool calls; requires explicit opt-in, app-owned login, and closed development application"]
async fn authenticated_chatgpt_agent_execution_reads_dirty_editor_buffer() {
    let _owner = PROFILE_OWNER.lock().await;
    let (bridge, work) = development_bridge();
    let outcome = tokio::time::timeout(
        Duration::from_secs(180),
        run_shared_agent(bridge.clone(), work.path(), false, true, false, false),
    )
    .await;
    bridge.stop().await;
    outcome
        .expect("dirty editor buffer acceptance timed out")
        .expect("dirty editor buffer acceptance failed");
}

#[tokio::test]
#[ignore = "real model/tool calls; requires explicit opt-in, app-owned login, and closed development application"]
async fn authenticated_chatgpt_agent_execution_reads_selected_editor_range() {
    let _owner = PROFILE_OWNER.lock().await;
    let (bridge, work) = development_bridge();
    let outcome = tokio::time::timeout(
        Duration::from_secs(180),
        run_shared_agent(bridge.clone(), work.path(), false, false, true, false),
    )
    .await;
    bridge.stop().await;
    outcome
        .expect("selected editor range acceptance timed out")
        .expect("selected editor range acceptance failed");
}

#[tokio::test]
#[ignore = "real model/tool calls; requires explicit opt-in, app-owned login, and closed development application"]
async fn authenticated_chatgpt_agent_execution_reopens_writing_conversation() {
    let _owner = PROFILE_OWNER.lock().await;
    let (bridge, work) = development_bridge();
    let outcome = tokio::time::timeout(
        Duration::from_secs(240),
        run_shared_agent(bridge.clone(), work.path(), false, false, false, true),
    )
    .await;
    bridge.stop().await;
    outcome
        .expect("writing conversation reopen acceptance timed out")
        .expect("writing conversation reopen acceptance failed");
}

#[tokio::test]
#[ignore = "real model/tool calls; requires explicit opt-in, app-owned login, and closed development application"]
async fn authenticated_chatgpt_agent_execution_creates_and_undoes_writing_changeset() {
    let _owner = PROFILE_OWNER.lock().await;
    let (bridge, work) = development_bridge();
    let outcome = tokio::time::timeout(
        Duration::from_secs(180),
        run_real_changeset_agent(bridge.clone(), work.path(), false, None),
    )
    .await;
    bridge.stop().await;
    outcome
        .expect("real writing changeset acceptance timed out")
        .expect("real writing changeset acceptance failed");
}

#[tokio::test]
#[ignore = "real model/tool calls; requires explicit opt-in, app-owned login, and closed development application"]
async fn authenticated_chatgpt_agent_execution_rejects_stale_editor_changeset() {
    let _owner = PROFILE_OWNER.lock().await;
    let (bridge, work) = development_bridge();
    let outcome = tokio::time::timeout(
        Duration::from_secs(180),
        run_real_changeset_agent(bridge.clone(), work.path(), true, None),
    )
    .await;
    bridge.stop().await;
    outcome
        .expect("stale editor changeset acceptance timed out")
        .expect("stale editor changeset acceptance failed");
}

#[tokio::test]
#[ignore = "real model/tool calls; requires explicit opt-in, app-owned login, a fixture directory, and closed development application"]
async fn authenticated_chatgpt_agent_execution_persists_pending_writing_review() {
    let _owner = PROFILE_OWNER.lock().await;
    let fixture = std::env::var_os("RESEARCH_AGENT_NATIVE_WRITING_REVIEW_FIXTURE_DIRECTORY")
        .map(std::path::PathBuf::from)
        .ok_or(AcceptanceError::Setup)
        .expect("native writing review fixture directory is required");
    let (bridge, work) = development_bridge();
    let outcome = tokio::time::timeout(
        Duration::from_secs(180),
        run_real_changeset_agent(bridge.clone(), work.path(), false, Some(&fixture)),
    )
    .await;
    bridge.stop().await;
    outcome
        .expect("native writing review fixture timed out")
        .expect("native writing review fixture failed");
}

#[tokio::test]
#[ignore = "real model/tool calls; requires explicit opt-in, app-owned login, and closed development application"]
async fn authenticated_chatgpt_agent_execution_applies_two_writing_changesets() {
    let _owner = PROFILE_OWNER.lock().await;
    let (bridge, work) = development_bridge();
    let outcome = tokio::time::timeout(
        Duration::from_secs(300),
        run_real_multi_step_changesets(bridge.clone(), work.path()),
    )
    .await;
    bridge.stop().await;
    outcome
        .expect("multi-step writing changeset acceptance timed out")
        .expect("multi-step writing changeset acceptance failed");
}

async fn run_real_changeset_agent(
    bridge: CodexAppServerManager,
    work: &std::path::Path,
    reject_stale_editor: bool,
    persisted_fixture: Option<&std::path::Path>,
) -> Result<(), AcceptanceError> {
    let mut db = if let Some(directory) = persisted_fixture {
        if !directory.is_absolute() || directory.join("research-assistant.sqlite3").exists() {
            return Err(AcceptanceError::Setup);
        }
        std::fs::create_dir_all(directory).map_err(|_| AcceptanceError::Setup)?;
        Connection::open(directory.join("research-assistant.sqlite3"))
            .map_err(|_| AcceptanceError::Setup)?
    } else {
        Connection::open_in_memory().map_err(|_| AcceptanceError::Setup)?
    };
    crate::storage::migrate(&mut db).map_err(|_| AcceptanceError::Setup)?;
    let content_directory = persisted_fixture.map_or_else(
        || work.join("objects"),
        |directory| directory.join("objects"),
    );
    let app_store = AppStore::for_test(db, content_directory);
    let service = Arc::new(
        AgentSessionService::from_app_store(&app_store).map_err(|_| AcceptanceError::Setup)?,
    );
    let (conversation_id, manuscript_id, source_id, source_hash, _) =
        seed_writing_manuscript(&app_store);
    let credentials =
        crate::ai::credential::JsonCredentialStore::from_path(work.join("credentials.json"));
    let models = crate::agent::model::AgentModelResolver::from_app_store(&app_store, credentials)
        .with_chatgpt(Arc::new(
            crate::agent::authentication::AgentChatGptAuthentication::new(bridge),
        ));
    let writing =
        WritingSubmissionService::from_app_store(&app_store).with_model_resolver(models.clone());
    let original = std::fs::read(
        app_store
            .manuscript_directory()
            .join(manuscript_id.to_string())
            .join("source/main.tex"),
    )
    .map_err(|_| AcceptanceError::Setup)?;
    let editor = reject_stale_editor.then(|| research_agent_core::workspace::EditorVersion {
        session_id: Uuid::new_v4(),
        revision: Uuid::new_v4(),
    });
    let editor_text = "Unsaved editor baseline before the Agent turn.";
    let record = writing
        .prepare(
            &models,
            WritingSubmissionRequest {
                resources: vec![],
            user_input: vec![],
                conversation_id,
                request_id: Uuid::new_v4(),
                text: format!(
                    "Use the approved isolated writing workspace. First call research_exec_command exactly once with a shell command that replaces main.tex with a minimal valid LaTeX article containing the sentence `Agent changeset revision.`. Then call research_review_workspace_changes exactly once. Do not claim that the live manuscript was changed or that the proposal was published. Finally reply with only CHANGESET_READY. The manuscript source is {source_id}."
                ),
                context: WritingContextRequest {
                    manuscript_id,
                    files: vec![WritingContextFile {
                        relative_path: "main.tex".into(),
                        expected_disk_sha256: source_hash.clone(),
                        buffer: editor.as_ref().map(|editor| WritingContextBuffer {
                            revision: editor.revision,
                            text: editor_text.into(),
                        }),
                        selection: None,
                    }],
                },
                retrieval: None,
                session_config: None,
                turn_config: None,
                model: Some(crate::agent::model::AgentModelSelection {
                    provider_id: "openai".into(),
                    model_id: "gpt-5.6-terra".into(),
                    reasoning_effort: None,
                    service_tier: None,
                }),
            },
        )
        .await
        .map_err(|_| AcceptanceError::Setup)?;
    let receipt = WritingSubmissionReceipt {
        conversation_id,
        request_id: record.record.request_id,
        manuscript_id,
    };
    let prepared = writing
        .prepare_workspace(
            receipt,
            record.record.context_snapshot_id,
            WritingWorkspaceRequest {
                manuscript_id,
                buffers: editor
                    .as_ref()
                    .map(|editor| crate::agent::writing_workspace::EditorBuffer {
                        path: "main.tex".into(),
                        expected_disk_sha256: source_hash.clone(),
                        editor: editor.clone(),
                        text: editor_text.into(),
                    })
                    .into_iter()
                    .collect(),
            },
        )
        .await
        .map_err(|_| AcceptanceError::Admission)?;
    writing
        .approve_workspace(prepared)
        .await
        .map_err(|_| AcceptanceError::Admission)?;
    let model = record.record.input.model.clone();
    let profile = crate::agent::execution_profile::execution_profile(
        &model,
        crate::agent::execution_profile::AgentExecutionMode::ChatGptDefault,
    )
    .map_err(|_| AcceptanceError::Setup)?;
    let turn_id = record.record.turn_id;
    let context_snapshot_id = record.record.context_snapshot_id;
    let SubmissionAdmission::Started(owned) = writing
        .start_confirmed(
            service.clone(),
            record,
            WritingExecutionConfirmation {
                receipt,
                context_snapshot_id,
                model,
            },
            profile,
        )
        .await
        .map_err(|_| AcceptanceError::Admission)?
    else {
        return Err(AcceptanceError::Admission);
    };
    if owned.task.wait().await != TaskCompletion::Completed {
        return Err(AcceptanceError::Task);
    }
    let projection = service
        .open(ConversationId::from_uuid(conversation_id))
        .await
        .map_err(|_| AcceptanceError::Projection)?;
    let turn = projection
        .turn(turn_id)
        .ok_or(AcceptanceError::Projection)?;
    if turn.outcome != Some(TurnOutcome::Completed) {
        return Err(AcceptanceError::Projection);
    }
    if !turn.items.values().any(|item| {
        matches!(&item.kind, ItemKind::ToolCall { name, .. } if name == crate::agent::shell_tools::EXEC)
    }) {
        return Err(AcceptanceError::MissingShellCall);
    }
    if !turn.items.values().any(|item| {
        matches!(&item.kind, ItemKind::ToolCall { name, .. } if name == "research_review_workspace_changes")
    }) {
        return Err(AcceptanceError::MissingWorkspaceReview);
    }
    if !turn.items.values().any(|item| {
        matches!(item.kind, ItemKind::AssistantMessage) && item.text.contains("CHANGESET_READY")
    }) {
        return Err(AcceptanceError::MissingAnswer);
    }
    if std::fs::read(
        app_store
            .manuscript_directory()
            .join(manuscript_id.to_string())
            .join("source/main.tex"),
    )
    .map_err(|_| AcceptanceError::Setup)?
        != original
    {
        return Err(AcceptanceError::Application);
    }
    let review = writing
        .review_workspace(receipt, context_snapshot_id)
        .await
        .map_err(|_| AcceptanceError::WorkspaceReview)?;
    if !review
        .changes
        .iter()
        .any(|change| change.path == "main.tex")
    {
        return Err(AcceptanceError::WorkspaceReview);
    }
    std::fs::create_dir_all(app_store.content_directory())
        .map_err(|_| AcceptanceError::Proposal)?;
    let proposal = writing
        .publish_workspace_changes(review.binding.clone(), review.changes)
        .await
        .map_err(|_| AcceptanceError::Proposal)?;
    if persisted_fixture.is_some() {
        if writing
            .review_proposal(conversation_id, manuscript_id, proposal)
            .await
            .map_err(|_| AcceptanceError::Proposal)?
            .status
            != crate::agent::repository::change_sets::ChangeSetStatus::Pending
        {
            return Err(AcceptanceError::Proposal);
        }
        return Ok(());
    }
    let operation = Uuid::new_v4();
    if let Some(editor) = editor {
        let changed = research_agent_core::workspace::EditorVersion {
            session_id: editor.session_id,
            revision: Uuid::new_v4(),
        };
        let stale_result = writing
            .begin_proposal_application(
                conversation_id,
                proposal,
                operation,
                WritingWorkspaceRequest {
                    manuscript_id,
                    buffers: vec![crate::agent::writing_workspace::EditorBuffer {
                        path: "main.tex".into(),
                        expected_disk_sha256: source_hash,
                        editor: changed,
                        text: "User edit made after the Agent proposal.".into(),
                    }],
                },
            )
            .await;
        if stale_result.is_ok()
            || writing
                .find_proposal_application(conversation_id, manuscript_id, proposal)
                .await
                .map_err(|_| AcceptanceError::Application)?
                .is_some()
            || writing
                .review_proposal(conversation_id, manuscript_id, proposal)
                .await
                .map_err(|_| AcceptanceError::Application)?
                .status
                != crate::agent::repository::change_sets::ChangeSetStatus::Pending
            || std::fs::read(
                app_store
                    .manuscript_directory()
                    .join(manuscript_id.to_string())
                    .join("source/main.tex"),
            )
            .map_err(|_| AcceptanceError::Setup)?
                != original
        {
            return Err(AcceptanceError::Application);
        }
        return Ok(());
    }
    writing
        .begin_proposal_application(
            conversation_id,
            proposal,
            operation,
            WritingWorkspaceRequest {
                manuscript_id,
                buffers: vec![],
            },
        )
        .await
        .map_err(|_| AcceptanceError::Application)?;
    writing
        .apply_disk_proposal_file(
            crate::agent::repository::proposal_applications::ApplicationScope {
                conversation: conversation_id,
                manuscript: manuscript_id,
                proposal,
                operation,
            },
            "main.tex".into(),
        )
        .await
        .map_err(|_| AcceptanceError::Application)?;
    let application_scope = crate::agent::repository::proposal_applications::ApplicationScope {
        conversation: conversation_id,
        manuscript: manuscript_id,
        proposal,
        operation,
    };
    let undo_id = Uuid::new_v4();
    writing
        .prepare_proposal_undo(
            application_scope,
            undo_id,
            WritingWorkspaceRequest {
                manuscript_id,
                buffers: vec![],
            },
        )
        .await
        .map_err(|_| AcceptanceError::Undo)?;
    writing
        .start_proposal_undo(
            application_scope,
            undo_id,
            WritingWorkspaceRequest {
                manuscript_id,
                buffers: vec![],
            },
        )
        .await
        .map_err(|_| AcceptanceError::Undo)?;
    writing
        .apply_disk_undo_file(application_scope, undo_id, "main.tex".into())
        .await
        .map_err(|_| AcceptanceError::Undo)?;
    if std::fs::read(
        app_store
            .manuscript_directory()
            .join(manuscript_id.to_string())
            .join("source/main.tex"),
    )
    .map_err(|_| AcceptanceError::Setup)?
        != original
    {
        return Err(AcceptanceError::Undo);
    }
    Ok(())
}

async fn run_real_multi_step_changesets(
    bridge: CodexAppServerManager,
    work: &std::path::Path,
) -> Result<(), AcceptanceError> {
    let mut db = Connection::open_in_memory().map_err(|_| AcceptanceError::Setup)?;
    crate::storage::migrate(&mut db).map_err(|_| AcceptanceError::Setup)?;
    let app_store = AppStore::for_test(db, work.join("objects"));
    let service = Arc::new(
        AgentSessionService::from_app_store(&app_store).map_err(|_| AcceptanceError::Setup)?,
    );
    let (conversation_id, manuscript_id, source_id, mut source_hash, _) =
        seed_writing_manuscript(&app_store);
    let credentials =
        crate::ai::credential::JsonCredentialStore::from_path(work.join("credentials.json"));
    let models = crate::agent::model::AgentModelResolver::from_app_store(&app_store, credentials)
        .with_chatgpt(Arc::new(
            crate::agent::authentication::AgentChatGptAuthentication::new(bridge),
        ));
    let writing =
        WritingSubmissionService::from_app_store(&app_store).with_model_resolver(models.clone());
    let manuscript_source = || {
        app_store
            .manuscript_directory()
            .join(manuscript_id.to_string())
            .join("source/main.tex")
    };
    let original = std::fs::read(manuscript_source()).map_err(|_| AcceptanceError::Setup)?;
    std::fs::create_dir_all(app_store.content_directory()).map_err(|_| AcceptanceError::Setup)?;

    let first = run_changeset_turn(
        &writing,
        &models,
        service.clone(),
        ChangesetTurnRequest {
            conversation_id,
            manuscript_id,
            source_id: source_id.clone(),
            source_hash: source_hash.clone(),
            text: "Replace main.tex with a minimal valid LaTeX article containing exactly the sentence `Agent multi-step revision one.`. Call research_exec_command exactly once, then research_review_workspace_changes exactly once, and finally reply only CHANGESET_ONE_READY.",
        },
    )
    .await?;
    let first_scope = apply_changeset(&writing, conversation_id, manuscript_id, first.1).await?;
    let after_first = std::fs::read(manuscript_source()).map_err(|_| AcceptanceError::Setup)?;
    if !String::from_utf8_lossy(&after_first).contains("Agent multi-step revision one.") {
        return Err(AcceptanceError::Application);
    }
    source_hash = sha256_hex(&after_first);

    let second = run_changeset_turn(
        &writing,
        &models,
        service,
        ChangesetTurnRequest {
            conversation_id,
            manuscript_id,
            source_id,
            source_hash,
            text: "Update the current main.tex article so it retains `Agent multi-step revision one.` and adds `Agent multi-step revision two.`. Call research_exec_command exactly once, then research_review_workspace_changes exactly once, and finally reply only CHANGESET_TWO_READY.",
        },
    )
    .await?;
    let second_scope = apply_changeset(&writing, conversation_id, manuscript_id, second.1).await?;
    let final_bytes = std::fs::read(manuscript_source()).map_err(|_| AcceptanceError::Setup)?;
    let final_text = String::from_utf8_lossy(&final_bytes);
    if !final_text.contains("Agent multi-step revision one.")
        || !final_text.contains("Agent multi-step revision two.")
    {
        return Err(AcceptanceError::Application);
    }

    undo_changeset(&writing, second_scope).await?;
    undo_changeset(&writing, first_scope).await?;
    if std::fs::read(manuscript_source()).map_err(|_| AcceptanceError::Setup)? != original {
        return Err(AcceptanceError::Undo);
    }
    Ok(())
}

struct ChangesetTurnRequest {
    conversation_id: Uuid,
    manuscript_id: Uuid,
    source_id: String,
    source_hash: String,
    text: &'static str,
}

async fn run_changeset_turn(
    writing: &WritingSubmissionService,
    models: &crate::agent::model::AgentModelResolver,
    service: Arc<AgentSessionService>,
    request: ChangesetTurnRequest,
) -> Result<(WritingSubmissionReceipt, Uuid), AcceptanceError> {
    let request_id = Uuid::new_v4();
    let record = writing
        .prepare(
            models,
            WritingSubmissionRequest {
                resources: vec![],
                user_input: vec![],
                conversation_id: request.conversation_id,
                request_id,
                text: format!(
                    "{} The manuscript source is {}.",
                    request.text, request.source_id
                ),
                context: WritingContextRequest {
                    manuscript_id: request.manuscript_id,
                    files: vec![WritingContextFile {
                        relative_path: "main.tex".into(),
                        expected_disk_sha256: request.source_hash,
                        buffer: None,
                        selection: None,
                    }],
                },
                retrieval: None,
                session_config: None,
                turn_config: None,
                model: Some(crate::agent::model::AgentModelSelection {
                    provider_id: "openai".into(),
                    model_id: "gpt-5.6-terra".into(),
                    reasoning_effort: None,
                    service_tier: None,
                }),
            },
        )
        .await
        .map_err(|_| AcceptanceError::Setup)?;
    let receipt = WritingSubmissionReceipt {
        conversation_id: request.conversation_id,
        request_id,
        manuscript_id: request.manuscript_id,
    };
    let prepared = writing
        .prepare_workspace(
            receipt,
            record.record.context_snapshot_id,
            WritingWorkspaceRequest {
                manuscript_id: request.manuscript_id,
                buffers: vec![],
            },
        )
        .await
        .map_err(|_| AcceptanceError::Admission)?;
    writing
        .approve_workspace(prepared)
        .await
        .map_err(|_| AcceptanceError::Admission)?;
    let model = record.record.input.model.clone();
    let profile = crate::agent::execution_profile::execution_profile(
        &model,
        crate::agent::execution_profile::AgentExecutionMode::ChatGptDefault,
    )
    .map_err(|_| AcceptanceError::Setup)?;
    let turn_id = record.record.turn_id;
    let context_snapshot_id = record.record.context_snapshot_id;
    let SubmissionAdmission::Started(owned) = writing
        .start_confirmed(
            service.clone(),
            record,
            WritingExecutionConfirmation {
                receipt,
                context_snapshot_id,
                model,
            },
            profile,
        )
        .await
        .map_err(|_| AcceptanceError::Admission)?
    else {
        return Err(AcceptanceError::Admission);
    };
    if owned.task.wait().await != TaskCompletion::Completed {
        return Err(AcceptanceError::Task);
    }
    let projection = service
        .open(ConversationId::from_uuid(request.conversation_id))
        .await
        .map_err(|_| AcceptanceError::Projection)?;
    let turn = projection
        .turn(turn_id)
        .ok_or(AcceptanceError::Projection)?;
    if turn.outcome != Some(TurnOutcome::Completed)
        || !turn.items.values().any(|item| {
            matches!(&item.kind, ItemKind::ToolCall { name, .. } if name == crate::agent::shell_tools::EXEC)
        })
        || !turn.items.values().any(|item| {
            matches!(&item.kind, ItemKind::ToolCall { name, .. } if name == "research_review_workspace_changes")
        })
        || !turn.items.values().any(|item| {
            matches!(item.kind, ItemKind::AssistantMessage)
                && (item.text.contains("CHANGESET_ONE_READY")
                    || item.text.contains("CHANGESET_TWO_READY"))
        })
    {
        return Err(AcceptanceError::Projection);
    }
    let review = writing
        .review_workspace(receipt, context_snapshot_id)
        .await
        .map_err(|_| AcceptanceError::WorkspaceReview)?;
    if !review
        .changes
        .iter()
        .any(|change| change.path == "main.tex")
    {
        return Err(AcceptanceError::WorkspaceReview);
    }
    let proposal = writing
        .publish_workspace_changes(review.binding, review.changes)
        .await
        .map_err(|_| AcceptanceError::Proposal)?;
    Ok((receipt, proposal))
}

async fn apply_changeset(
    writing: &WritingSubmissionService,
    conversation_id: Uuid,
    manuscript_id: Uuid,
    proposal: Uuid,
) -> Result<crate::agent::repository::proposal_applications::ApplicationScope, AcceptanceError> {
    let scope = crate::agent::repository::proposal_applications::ApplicationScope {
        conversation: conversation_id,
        manuscript: manuscript_id,
        proposal,
        operation: Uuid::new_v4(),
    };
    writing
        .begin_proposal_application(
            conversation_id,
            proposal,
            scope.operation,
            WritingWorkspaceRequest {
                manuscript_id,
                buffers: vec![],
            },
        )
        .await
        .map_err(|_| AcceptanceError::Application)?;
    writing
        .apply_disk_proposal_file(scope, "main.tex".into())
        .await
        .map_err(|_| AcceptanceError::Application)?;
    Ok(scope)
}

async fn undo_changeset(
    writing: &WritingSubmissionService,
    scope: crate::agent::repository::proposal_applications::ApplicationScope,
) -> Result<(), AcceptanceError> {
    let undo_id = Uuid::new_v4();
    writing
        .prepare_proposal_undo(
            scope,
            undo_id,
            WritingWorkspaceRequest {
                manuscript_id: scope.manuscript,
                buffers: vec![],
            },
        )
        .await
        .map_err(|_| AcceptanceError::Undo)?;
    writing
        .start_proposal_undo(
            scope,
            undo_id,
            WritingWorkspaceRequest {
                manuscript_id: scope.manuscript,
                buffers: vec![],
            },
        )
        .await
        .map_err(|_| AcceptanceError::Undo)?;
    writing
        .apply_disk_undo_file(scope, undo_id, "main.tex".into())
        .await
        .map(|_| ())
        .map_err(|_| AcceptanceError::Undo)
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut digest = Sha256::new();
    digest.update(bytes);
    format!("{:x}", digest.finalize())
}

#[tokio::test]
#[ignore = "real model/tool calls; requires explicit opt-in, app-owned login, and closed development application"]
async fn authenticated_chatgpt_agent_execution_reads_frozen_paper_source() {
    let _owner = PROFILE_OWNER.lock().await;
    let (bridge, work) = development_bridge();
    let outcome = tokio::time::timeout(
        Duration::from_secs(180),
        run_real_paper_reading_agent(bridge.clone(), work.path(), false, true, true, false),
    )
    .await;
    bridge.stop().await;
    outcome
        .expect("shared paper Agent acceptance timed out")
        .expect("shared paper Agent acceptance failed");
}

#[tokio::test]
#[ignore = "real vision model/tool call; requires explicit opt-in, app-owned login, and closed development application"]
async fn authenticated_chatgpt_agent_execution_reads_paper_image() {
    let _owner = PROFILE_OWNER.lock().await;
    let (bridge, work) = development_bridge();
    let outcome = tokio::time::timeout(
        Duration::from_secs(180),
        run_real_paper_reading_agent(bridge.clone(), work.path(), true, true, true, false),
    )
    .await;
    bridge.stop().await;
    outcome
        .expect("shared paper image Agent acceptance timed out")
        .expect("shared paper image Agent acceptance failed");
}

#[tokio::test]
#[ignore = "real model/tool calls; requires explicit opt-in, app-owned login, and closed development application"]
async fn authenticated_chatgpt_agent_execution_spawns_paper_child() {
    let _owner = PROFILE_OWNER.lock().await;
    let (bridge, work) = development_bridge();
    let outcome = tokio::time::timeout(
        Duration::from_secs(180),
        run_real_paper_reading_agent(bridge.clone(), work.path(), false, false, false, true),
    )
    .await;
    bridge.stop().await;
    outcome
        .expect("shared paper child acceptance timed out")
        .expect("shared paper child acceptance failed");
}

#[tokio::test]
#[ignore = "real model call; requires explicit opt-in, app-owned login, and closed development application"]
async fn authenticated_chatgpt_agent_execution_spawns_note_child() {
    let _owner = PROFILE_OWNER.lock().await;
    let (bridge, work) = development_bridge();
    let outcome = tokio::time::timeout(
        Duration::from_secs(180),
        run_real_note_child_agent(bridge.clone(), work.path()),
    )
    .await;
    bridge.stop().await;
    outcome
        .expect("shared note child acceptance timed out")
        .expect("shared note child acceptance failed");
}

#[tokio::test]
#[ignore = "real model/tool calls; requires explicit opt-in, app-owned login, a fixture directory, and closed development application"]
async fn authenticated_chatgpt_persists_pending_note_rewrite_review() {
    let _owner = PROFILE_OWNER.lock().await;
    let fixture = std::env::var_os("RESEARCH_AGENT_NATIVE_NOTE_REVIEW_FIXTURE_DIRECTORY")
        .map(std::path::PathBuf::from)
        .ok_or(AcceptanceError::Setup)
        .expect("native note review fixture directory is required");
    let (bridge, work) = development_bridge();
    let outcome = tokio::time::timeout(
        Duration::from_secs(180),
        run_real_note_rewrite_agent(bridge.clone(), work.path(), &fixture),
    )
    .await;
    bridge.stop().await;
    outcome
        .expect("native note rewrite review fixture timed out")
        .expect("native note rewrite review fixture failed");
}

#[tokio::test]
#[ignore = "real model call; requires explicit opt-in, app-owned login, and closed development application"]
async fn authenticated_chatgpt_agent_execution_spawns_reading_note_child() {
    let _owner = PROFILE_OWNER.lock().await;
    let (bridge, work) = development_bridge();
    let outcome = tokio::time::timeout(
        Duration::from_secs(180),
        run_real_reading_note_child_agent(bridge.clone(), work.path()),
    )
    .await;
    bridge.stop().await;
    outcome
        .expect("shared reading-note child acceptance timed out")
        .expect("shared reading-note child acceptance failed");
}

#[tokio::test]
#[ignore = "real model call; requires explicit opt-in, app-owned login, a fixture directory, and closed development application"]
async fn authenticated_chatgpt_persists_scoped_reading_note_conversation() {
    let _owner = PROFILE_OWNER.lock().await;
    let fixture = std::env::var_os("RESEARCH_AGENT_NATIVE_READING_NOTE_FIXTURE_DIRECTORY")
        .map(std::path::PathBuf::from)
        .ok_or(AcceptanceError::Setup)
        .expect("native reading-note fixture directory is required");
    let (bridge, work) = development_bridge();
    let outcome = tokio::time::timeout(
        Duration::from_secs(180),
        run_real_reading_note_agent(bridge.clone(), work.path(), &fixture),
    )
    .await;
    bridge.stop().await;
    outcome
        .expect("native reading-note fixture timed out")
        .expect("native reading-note fixture failed");
}

#[tokio::test]
#[ignore = "real structured model call; requires explicit opt-in, app-owned login, and closed development application"]
async fn authenticated_chatgpt_persists_structured_paper_overview_candidate() {
    let _owner = PROFILE_OWNER.lock().await;
    let (bridge, work) = development_bridge();
    let outcome = tokio::time::timeout(
        Duration::from_secs(180),
        run_real_structured_paper_overview(bridge.clone(), work.path()),
    )
    .await;
    bridge.stop().await;
    outcome
        .expect("structured paper overview acceptance timed out")
        .expect("structured paper overview acceptance failed");
}

async fn run_real_note_child_agent(
    bridge: CodexAppServerManager,
    work: &std::path::Path,
) -> Result<(), AcceptanceError> {
    let mut db = Connection::open_in_memory().map_err(|_| AcceptanceError::Setup)?;
    crate::storage::migrate(&mut db).map_err(|_| AcceptanceError::Setup)?;
    let conversation_id = Uuid::new_v4();
    let page_id = Uuid::new_v4();
    create_conversation(
        &mut db,
        ConversationId::from_uuid(conversation_id),
        "Note child smoke",
    )
    .map_err(|_| AcceptanceError::Setup)?;
    db.execute(
        "insert into note_pages(id,title,created_at,updated_at) values(?1,'Synthetic note','now','now')",
        [page_id.to_string()],
    )
    .map_err(|_| AcceptanceError::Setup)?;
    let content = serde_json::json!({
        "kind": "inline",
        "items": [{"type": "text", "text": "Frozen note child evidence", "styles": []}]
    })
    .to_string();
    db.execute(
        "insert into note_blocks(id,page_id,scope_kind,block_type,content_json,created_at,updated_at) values('note-child-block',?1,'page','paragraph',?2,'now','now')",
        rusqlite::params![page_id.to_string(), content],
    )
    .map_err(|_| AcceptanceError::Setup)?;
    db.execute(
        "insert into agent_conversation_bindings(conversation_id,resource_kind,resource_id) values(?1,'note',?2)",
        rusqlite::params![conversation_id.to_string(), page_id.to_string()],
    )
    .map_err(|_| AcceptanceError::Setup)?;
    let store = AppStore::for_test(db, work.join("note-child-objects"));
    let credentials = crate::ai::credential::JsonCredentialStore::from_path(
        work.join("note-child-credentials.json"),
    );
    let models = crate::agent::model::AgentModelResolver::from_app_store(&store, credentials)
        .with_chatgpt(Arc::new(
            crate::agent::authentication::AgentChatGptAuthentication::new(bridge),
        ));
    let service =
        Arc::new(AgentSessionService::from_app_store(&store).map_err(|_| AcceptanceError::Setup)?);
    let parent = note_submission::prepare(
        &store,
        &models,
        note_submission::PrepareRequest {
            user_input: vec![],
            conversation_id,
            request_id: Uuid::new_v4(),
            text: "This parent submission only provides the frozen note scope.".into(),
            context: NoteContextRequest {
                page_id,
                expected_revision: 0,
                block_id: None,
            },
            retrieval: None,
            model: Some(crate::agent::model::ModelSelection {
                provider_id: "openai".into(),
                model_id: "gpt-5.6-terra".into(),
                reasoning_effort: None,
                service_tier: None,
            }),
            session_config: None,
            turn_config: None,
        },
    )
    .await
    .map_err(|_| AcceptanceError::Setup)?;
    let parent_owner = service
        .start_owned_turn(
            ConversationId::from_uuid(conversation_id),
            |_turn_id, signal| async move {
            tokio::select! {
                _ = signal.cancelled() => Err(research_agent_core::runner::RunError::Model(research_agent_core::model::ModelError::Canceled)),
                _ = tokio::time::sleep(Duration::from_secs(30)) => Ok(()),
            }
        },
        )
        .map_err(|_| AcceptanceError::Admission)?;
    let child = note_child_commands::spawn_note_child(
        store.clone(),
        SqliteConversationCatalog::new(store.shared_agent_connection()),
        models,
        service.clone(),
        note_child_commands::Request {
            parent_conversation_id: conversation_id,
            parent_turn_id: parent.turn_id,
            page_id,
            prompt: "Reply with only NOTE_CHILD_OK.".into(),
            approved_servers: std::collections::HashSet::new(),
        },
    )
    .await
    .map_err(|_| AcceptanceError::Admission)?;
    let mut child_turn = None;
    for _ in 0..120 {
        let projection = service
            .open(ConversationId::from_uuid(child.child_conversation_id))
            .await
            .map_err(|_| AcceptanceError::Projection)?;
        if let Some(turn) = projection.turn(child.turn_id).cloned()
            && turn.outcome.is_some()
        {
            child_turn = Some(turn);
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    let child_turn = child_turn.ok_or(AcceptanceError::ChildTurnTimeout)?;
    if child_turn.outcome != Some(TurnOutcome::Completed) {
        return Err(AcceptanceError::ChildOutcome);
    }
    if !child_turn.items.values().any(|item| {
        matches!(item.kind, ItemKind::AssistantMessage) && item.text.contains("NOTE_CHILD_OK")
    }) {
        return Err(AcceptanceError::ChildAnswerMissing);
    }
    service
        .tasks()
        .cancel(
            ConversationId::from_uuid(conversation_id),
            parent_owner.task.run_id(),
        )
        .map_err(|_| AcceptanceError::Task)?;
    let _ = parent_owner.task.wait().await;
    Ok(())
}

async fn run_real_note_rewrite_agent(
    bridge: CodexAppServerManager,
    work: &std::path::Path,
    fixture: &std::path::Path,
) -> Result<(), AcceptanceError> {
    if !fixture.is_absolute() || fixture.join("research-assistant.sqlite3").exists() {
        return Err(AcceptanceError::Setup);
    }
    std::fs::create_dir_all(fixture).map_err(|_| AcceptanceError::Setup)?;
    let mut db = Connection::open(fixture.join("research-assistant.sqlite3"))
        .map_err(|_| AcceptanceError::Setup)?;
    crate::storage::migrate(&mut db).map_err(|_| AcceptanceError::Setup)?;
    let conversation_id = Uuid::new_v4();
    let page_id = Uuid::new_v4();
    let other_page_id = Uuid::new_v4();
    create_conversation(
        &mut db,
        ConversationId::from_uuid(conversation_id),
        "Native note rewrite review",
    )
    .map_err(|_| AcceptanceError::Setup)?;
    for (id, title) in [
        (page_id, "Agent rewrite fixture"),
        (other_page_id, "Unrelated note fixture"),
    ] {
        db.execute(
            "insert into note_pages(id,title,created_at,updated_at)
             values(?1,?2,strftime('%Y-%m-%dT%H:%M:%fZ','now'),strftime('%Y-%m-%dT%H:%M:%fZ','now'))",
            rusqlite::params![id.to_string(), title],
        )
        .map_err(|_| AcceptanceError::Setup)?;
    }
    let selected = serde_json::json!({
        "kind": "inline",
        "items": [{"type": "text", "text": "The experiment has a clear result.", "styles": []}]
    })
    .to_string();
    let unrelated = serde_json::json!({
        "kind": "inline",
        "items": [{"type": "text", "text": "This unrelated note must remain unchanged.", "styles": []}]
    })
    .to_string();
    db.execute(
        "insert into note_blocks(id,page_id,scope_kind,block_type,position,content_json,created_at,updated_at)
         values('rewrite-target',?1,'page','paragraph',0,?2,
                strftime('%Y-%m-%dT%H:%M:%fZ','now'),strftime('%Y-%m-%dT%H:%M:%fZ','now'))",
        rusqlite::params![page_id.to_string(), selected],
    )
    .map_err(|_| AcceptanceError::Setup)?;
    db.execute(
        "insert into note_blocks(id,page_id,scope_kind,block_type,position,content_json,created_at,updated_at)
         values('unrelated-target',?1,'page','paragraph',0,?2,
                strftime('%Y-%m-%dT%H:%M:%fZ','now'),strftime('%Y-%m-%dT%H:%M:%fZ','now'))",
        rusqlite::params![other_page_id.to_string(), unrelated],
    )
    .map_err(|_| AcceptanceError::Setup)?;
    db.execute(
        "insert into agent_conversation_bindings(conversation_id,resource_kind,resource_id) values(?1,'note',?2)",
        rusqlite::params![conversation_id.to_string(), page_id.to_string()],
    )
    .map_err(|_| AcceptanceError::Setup)?;
    let store = AppStore::for_test(db, fixture.join("objects"));
    let credentials = crate::ai::credential::JsonCredentialStore::from_path(
        work.join("note-rewrite-credentials.json"),
    );
    let models = crate::agent::model::AgentModelResolver::from_app_store(&store, credentials)
        .with_chatgpt(Arc::new(
            crate::agent::authentication::AgentChatGptAuthentication::new(bridge),
        ));
    let context = NoteContextRequest {
        page_id,
        expected_revision: 0,
        block_id: Some("rewrite-target".into()),
    };
    let source = crate::notes::agent_context::materialize(&store, context.clone())
        .await
        .map_err(|_| AcceptanceError::Setup)?;
    let record = note_submission::prepare(
        &store,
        &models,
        note_submission::PrepareRequest {
            user_input: vec![],
            conversation_id,
            request_id: Uuid::new_v4(),
            text: format!(
                "Call research_propose_note_rewrite exactly once for sourceId `{}` and blockId `rewrite-target`. Replace the block with one plain unstyled text item whose complete text is `The experiment produced a clear result.`. Do not call any other tool, do not claim the note was saved, and finally reply with only NOTE_REWRITE_READY.",
                source.source_id
            ),
            context,
            retrieval: None,
            model: Some(crate::agent::model::ModelSelection {
                provider_id: "openai".into(),
                model_id: "gpt-5.6-terra".into(),
                reasoning_effort: None,
                service_tier: None,
            }),
            session_config: None,
            turn_config: None,
        },
    )
    .await
    .map_err(|_| AcceptanceError::Setup)?;
    let service =
        Arc::new(AgentSessionService::from_app_store(&store).map_err(|_| AcceptanceError::Setup)?);
    let SubmissionAdmission::Started(owned) = note_start::start(
        &store,
        &models,
        service.clone(),
        note_start::StartRequest {
            conversation_id,
            request_id: record.request_id,
            page_id,
            context_snapshot_id: record.context_snapshot_id,
            model: record.input.model.clone(),
            mode: crate::agent::execution_profile::AgentExecutionMode::ChatGptDefault,
        },
    )
    .await
    .map_err(|_| AcceptanceError::Admission)?
    else {
        return Err(AcceptanceError::Admission);
    };
    if owned.task.wait().await != TaskCompletion::Completed {
        return Err(AcceptanceError::Task);
    }
    let projection = service
        .open(ConversationId::from_uuid(conversation_id))
        .await
        .map_err(|_| AcceptanceError::Projection)?;
    let turn = projection
        .turn(record.turn_id)
        .ok_or(AcceptanceError::Projection)?;
    if turn.outcome != Some(TurnOutcome::Completed)
        || !turn.items.values().any(|item| {
            matches!(item.kind, ItemKind::AssistantMessage)
                && item.text.contains("NOTE_REWRITE_READY")
        })
    {
        return Err(AcceptanceError::Projection);
    }
    let result_item = turn
        .item_order
        .iter()
        .filter_map(|id| turn.items.get(id).map(|item| (*id, item)))
        .find_map(|(id, item)| match item.kind {
            ItemKind::ToolResult {
                is_error: Some(false),
                ..
            } if item.text.contains("\"kind\":\"noteInlineRewrite\"") => Some(id),
            _ => None,
        })
        .ok_or(AcceptanceError::MissingToolResult)?;
    let scope = note_review::Scope {
        conversation_id,
        page_id,
        turn_id: record.turn_id,
        item_id: result_item,
    };
    let review = note_review::review(&store, scope)
        .await
        .map_err(|_| AcceptanceError::NoteReview)?;
    if !review.baseline_matches
        || review.current_revision != 0
        || review.proposal["edits"][0]["after"]["items"][0]["text"]
            != "The experiment produced a clear result."
    {
        return Err(AcceptanceError::NoteReview);
    }
    let foreign_scope = note_review::Scope {
        page_id: other_page_id,
        ..scope
    };
    if !matches!(
        note_review::review(&store, foreign_scope).await,
        Err(note_review::Error::Unbound)
    ) || !matches!(
        note_application::accept(
            &store,
            note_application::Request {
                scope: foreign_scope,
                request_id: Uuid::new_v4(),
                expected_revision: 0,
            },
        )
        .await,
        Err(note_review::Error::Unbound)
    ) {
        return Err(AcceptanceError::ScopeIsolation);
    }
    let (page_revision, other_revision, applications): (i64, i64, i64) = store
        .connection()
        .map_err(|_| AcceptanceError::Setup)?
        .query_row(
            "select
             (select revision from note_pages where id=?1),
             (select revision from note_pages where id=?2),
             (select count(*) from agent_note_applications)",
            rusqlite::params![page_id.to_string(), other_page_id.to_string()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(|_| AcceptanceError::Setup)?;
    if (page_revision, other_revision, applications) != (0, 0, 0) {
        return Err(AcceptanceError::ScopeIsolation);
    }
    Ok(())
}

async fn run_real_reading_note_child_agent(
    bridge: CodexAppServerManager,
    work: &std::path::Path,
) -> Result<(), AcceptanceError> {
    let mut db = Connection::open_in_memory().map_err(|_| AcceptanceError::Setup)?;
    crate::storage::migrate(&mut db).map_err(|_| AcceptanceError::Setup)?;
    let conversation_id = Uuid::new_v4();
    let reading_note_id = Uuid::new_v4();
    create_conversation(
        &mut db,
        ConversationId::from_uuid(conversation_id),
        "Reading-note child smoke",
    )
    .map_err(|_| AcceptanceError::Setup)?;
    crate::notes::insert_reading_fixture(
        &db,
        &reading_note_id.to_string(),
        "Frozen reading-note evidence",
    );
    db.execute("update reading_note_sources set paper_title_snapshot='Synthetic paper',page_index=0,quote_exact='Frozen reading quote' where block_id=?1", [reading_note_id.to_string()]).map_err(|_| AcceptanceError::Setup)?;
    db.execute(
        "insert into agent_conversation_bindings(conversation_id,resource_kind,resource_id) values(?1,'readingNote',?2)",
        rusqlite::params![conversation_id.to_string(), reading_note_id.to_string()],
    )
    .map_err(|_| AcceptanceError::Setup)?;
    let store = AppStore::for_test(db, work.join("reading-note-child-objects"));
    let credentials = crate::ai::credential::JsonCredentialStore::from_path(
        work.join("reading-note-child-credentials.json"),
    );
    let models = crate::agent::model::AgentModelResolver::from_app_store(&store, credentials)
        .with_chatgpt(Arc::new(
            crate::agent::authentication::AgentChatGptAuthentication::new(bridge),
        ));
    let service =
        Arc::new(AgentSessionService::from_app_store(&store).map_err(|_| AcceptanceError::Setup)?);
    let parent = reading_note_submission::prepare(
        &store,
        &models,
        reading_note_submission::PrepareRequest {
            user_input: vec![],
            conversation_id,
            request_id: Uuid::new_v4(),
            text: "This parent submission only provides the frozen reading-note scope.".into(),
            context: reading_agent_context::Request {
                reading_note_id,
                expected_revision: 0,
            },
            retrieval: None,
            model: Some(crate::agent::model::ModelSelection {
                provider_id: "openai".into(),
                model_id: "gpt-5.6-terra".into(),
                reasoning_effort: None,
                service_tier: None,
            }),
            session_config: None,
            turn_config: None,
        },
    )
    .await
    .map_err(|_| AcceptanceError::Setup)?;
    let parent_owner = service
        .start_owned_turn(
            ConversationId::from_uuid(conversation_id),
            |_turn_id, signal| async move {
                tokio::select! {
                    _ = signal.cancelled() => Err(research_agent_core::runner::RunError::Model(research_agent_core::model::ModelError::Canceled)),
                    _ = tokio::time::sleep(Duration::from_secs(30)) => Ok(()),
                }
            },
        )
        .map_err(|_| AcceptanceError::Admission)?;
    let child = note_child_commands::spawn_reading_note_child(
        store.clone(),
        SqliteConversationCatalog::new(store.shared_agent_connection()),
        models,
        service.clone(),
        note_child_commands::ReadingRequest {
            parent_conversation_id: conversation_id,
            parent_turn_id: parent.turn_id,
            reading_note_id,
            prompt: "Reply with only READING_NOTE_CHILD_OK.".into(),
            approved_servers: std::collections::HashSet::new(),
        },
    )
    .await
    .map_err(|_| AcceptanceError::Admission)?;
    let mut child_turn = None;
    for _ in 0..120 {
        let projection = service
            .open(ConversationId::from_uuid(child.child_conversation_id))
            .await
            .map_err(|_| AcceptanceError::Projection)?;
        if let Some(turn) = projection.turn(child.turn_id).cloned()
            && turn.outcome.is_some()
        {
            child_turn = Some(turn);
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    let child_turn = child_turn.ok_or(AcceptanceError::ChildTurnTimeout)?;
    if child_turn.outcome != Some(TurnOutcome::Completed) {
        return Err(AcceptanceError::ChildOutcome);
    }
    if !child_turn.items.values().any(|item| {
        matches!(item.kind, ItemKind::AssistantMessage)
            && item.text.contains("READING_NOTE_CHILD_OK")
    }) {
        return Err(AcceptanceError::ChildAnswerMissing);
    }
    service
        .tasks()
        .cancel(
            ConversationId::from_uuid(conversation_id),
            parent_owner.task.run_id(),
        )
        .map_err(|_| AcceptanceError::Task)?;
    let _ = parent_owner.task.wait().await;
    Ok(())
}

async fn run_real_reading_note_agent(
    bridge: CodexAppServerManager,
    work: &std::path::Path,
    fixture: &std::path::Path,
) -> Result<(), AcceptanceError> {
    if !fixture.is_absolute() || fixture.join("research-assistant.sqlite3").exists() {
        return Err(AcceptanceError::Setup);
    }
    std::fs::create_dir_all(fixture).map_err(|_| AcceptanceError::Setup)?;
    let mut db = Connection::open(fixture.join("research-assistant.sqlite3"))
        .map_err(|_| AcceptanceError::Setup)?;
    crate::storage::migrate(&mut db).map_err(|_| AcceptanceError::Setup)?;
    let conversation_id = Uuid::new_v4();
    let reading_note_id = Uuid::new_v4();
    create_conversation(
        &mut db,
        ConversationId::from_uuid(conversation_id),
        "Native reading-note scope",
    )
    .map_err(|_| AcceptanceError::Setup)?;
    db.execute(
        "insert into note_pages(id,title,created_at,updated_at)
         values(?1,'Same-ID page fixture',strftime('%Y-%m-%dT%H:%M:%fZ','now'),strftime('%Y-%m-%dT%H:%M:%fZ','now'))",
        [reading_note_id.to_string()],
    )
    .map_err(|_| AcceptanceError::Setup)?;
    db.execute(
        "insert into note_blocks(id,page_id,scope_kind,block_type,position,content_json,created_at,updated_at)
         values('same-id-page-block',?1,'page','paragraph',0,?2,
                strftime('%Y-%m-%dT%H:%M:%fZ','now'),strftime('%Y-%m-%dT%H:%M:%fZ','now'))",
        rusqlite::params![
            reading_note_id.to_string(),
            serde_json::json!({"kind":"inline","items":[{"type":"text","text":"Independent page content.","styles":[]}]}).to_string()
        ],
    )
    .map_err(|_| AcceptanceError::Setup)?;
    crate::notes::insert_reading_fixture(
        &db,
        &reading_note_id.to_string(),
        "Observed response latency decreased.",
    );
    db.execute("update reading_note_sources set paper_title_snapshot='Scoped Reading Paper',page_index=1,page_label='2',quote_exact='Latency decreased after the intervention.' where block_id=?1", [reading_note_id.to_string()]).map_err(|_| AcceptanceError::Setup)?;
    db.execute(
        "insert into agent_conversation_bindings(conversation_id,resource_kind,resource_id)
         values(?1,'readingNote',?2)",
        rusqlite::params![conversation_id.to_string(), reading_note_id.to_string()],
    )
    .map_err(|_| AcceptanceError::Setup)?;
    let store = AppStore::for_test(db, fixture.join("objects"));
    let credentials = crate::ai::credential::JsonCredentialStore::from_path(
        work.join("reading-note-scope-credentials.json"),
    );
    let models = crate::agent::model::AgentModelResolver::from_app_store(&store, credentials)
        .with_chatgpt(Arc::new(
            crate::agent::authentication::AgentChatGptAuthentication::new(bridge),
        ));
    let record = reading_note_submission::prepare(
        &store,
        &models,
        reading_note_submission::PrepareRequest {
            user_input: vec![],
            conversation_id,
            request_id: Uuid::new_v4(),
            text: "Read the frozen reading-note body and quote. If both concern decreased latency, reply with only READING_NOTE_SCOPE_OK. Do not call any tool and do not claim to edit anything."
                .into(),
            context: reading_agent_context::Request {
                reading_note_id,
                expected_revision: 0,
            },
            retrieval: None,
            model: Some(crate::agent::model::ModelSelection {
                provider_id: "openai".into(),
                model_id: "gpt-5.6-terra".into(),
                reasoning_effort: None,
                service_tier: None,
            }),
            session_config: None,
            turn_config: None,
        },
    )
    .await
    .map_err(|_| AcceptanceError::Setup)?;
    let service =
        Arc::new(AgentSessionService::from_app_store(&store).map_err(|_| AcceptanceError::Setup)?);
    let SubmissionAdmission::Started(owned) = reading_note_start::start(
        &store,
        &models,
        service.clone(),
        reading_note_start::StartRequest {
            conversation_id,
            request_id: record.request_id,
            reading_note_id,
            context_snapshot_id: record.context_snapshot_id,
            model: record.input.model.clone(),
            mode: crate::agent::execution_profile::AgentExecutionMode::ChatGptDefault,
        },
    )
    .await
    .map_err(|_| AcceptanceError::Admission)?
    else {
        return Err(AcceptanceError::Admission);
    };
    if owned.task.wait().await != TaskCompletion::Completed {
        return Err(AcceptanceError::Task);
    }
    let projection = service
        .open(ConversationId::from_uuid(conversation_id))
        .await
        .map_err(|_| AcceptanceError::Projection)?;
    let turn = projection
        .turn(record.turn_id)
        .ok_or(AcceptanceError::Projection)?;
    if turn.outcome != Some(TurnOutcome::Completed)
        || !turn.items.values().any(|item| {
            matches!(item.kind, ItemKind::AssistantMessage)
                && item.text.contains("READING_NOTE_SCOPE_OK")
        })
        || turn.items.values().any(|item| {
            matches!(
                item.kind,
                ItemKind::ToolCall { .. } | ItemKind::ToolResult { .. }
            )
        })
    {
        return Err(AcceptanceError::Projection);
    }
    if !matches!(
        note_submission::restore(
            &store,
            note_submission::Receipt {
                conversation_id,
                request_id: record.request_id,
                page_id: reading_note_id,
            },
        )
        .await,
        Err(note_submission::Error::Unbound)
    ) {
        return Err(AcceptanceError::ScopeIsolation);
    }
    let (page_revision, reading_revision, applications): (i64, i64, i64) = store
        .connection()
        .map_err(|_| AcceptanceError::Setup)?
        .query_row(
            "select
             (select revision from note_pages where id=?1),
             (select p.revision from note_pages p join reading_note_sources s on s.page_id=p.id where s.block_id=?1),
             (select count(*) from agent_note_applications)",
            [reading_note_id.to_string()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(|_| AcceptanceError::Setup)?;
    if (page_revision, reading_revision, applications) != (0, 0, 0) {
        return Err(AcceptanceError::ScopeIsolation);
    }
    Ok(())
}

async fn run_real_structured_paper_overview(
    bridge: CodexAppServerManager,
    work: &std::path::Path,
) -> Result<(), AcceptanceError> {
    let persisted_fixture = std::env::var_os("RESEARCH_AGENT_NATIVE_REVIEW_FIXTURE_DIRECTORY")
        .map(std::path::PathBuf::from);
    let mut db = if let Some(directory) = &persisted_fixture {
        if !directory.is_absolute() || directory.join("research-assistant.sqlite3").exists() {
            return Err(AcceptanceError::Setup);
        }
        std::fs::create_dir_all(directory).map_err(|_| AcceptanceError::Setup)?;
        Connection::open(directory.join("research-assistant.sqlite3"))
            .map_err(|_| AcceptanceError::Setup)?
    } else {
        Connection::open_in_memory().map_err(|_| AcceptanceError::Setup)?
    };
    crate::storage::migrate(&mut db).map_err(|_| AcceptanceError::Setup)?;
    let content_directory = persisted_fixture.as_ref().map_or_else(
        || work.join("paper-overview-objects"),
        |root| root.join("objects"),
    );
    let app_store = AppStore::for_test(db, content_directory);
    std::fs::create_dir_all(app_store.content_directory()).map_err(|_| AcceptanceError::Setup)?;
    let paper_id = format!("overview-paper-{}", Uuid::new_v4());
    let attachment_id = format!("overview-attachment-{}", Uuid::new_v4());
    let parse_run_id = format!("overview-parse-{}", Uuid::new_v4());
    let content_hash = "d".repeat(64);
    let verification_code = Uuid::new_v4().simple().to_string();
    let docling_json = serde_json::json!({
        "pages": { "1": { "page_no": 1, "size": { "width": 600.0, "height": 800.0 } } },
        "texts": [{
            "self_ref": "#/texts/0",
            "label": "paragraph",
            "text": format!(
                "This paper asks whether durable research goals can survive changing conditions. It proposes a longitudinal comparison and reports verification code {verification_code}. The main limitation is that the evidence comes from one synthetic observation."
            ),
            "prov": [{
                "page_no": 1,
                "bbox": { "l": 40.0, "t": 60.0, "r": 560.0, "b": 180.0, "coord_origin": "TOPLEFT" },
                "charspan": [0, 220]
            }]
        }]
    });
    {
        let connection = app_store.connection().map_err(|_| AcceptanceError::Setup)?;
        connection
            .execute(
                "insert into bibliographic_items(id,item_type,title,created_at,updated_at) values(?1,'journalArticle','Structured overview smoke','now','now')",
                [&paper_id],
            )
            .map_err(|_| AcceptanceError::Setup)?;
        connection
            .execute(
                "insert into content_objects(sha256,relative_path,mime_type,byte_size,created_at) values(?1,?2,'application/pdf',1,'now')",
                rusqlite::params![&content_hash, format!("{paper_id}.pdf")],
            )
            .map_err(|_| AcceptanceError::Setup)?;
        connection
            .execute(
                "insert into attachments(id,item_id,content_sha256,display_name,attachment_kind,created_at) values(?1,?2,?3,'structured.pdf','pdf','now')",
                rusqlite::params![&attachment_id, &paper_id, &content_hash],
            )
            .map_err(|_| AcceptanceError::Setup)?;
        connection
            .execute(
                "insert into docling_parse_runs(id,item_id,attachment_id,content_sha256,service_base_url,docling_status,parse_status,json_content,raw_response,created_at,finished_at) values(?1,?2,?3,?4,'fixture','success','success',?5,'{}','now','now')",
                rusqlite::params![&parse_run_id, &paper_id, &attachment_id, &content_hash, docling_json.to_string()],
            )
            .map_err(|_| AcceptanceError::Setup)?;
        connection
            .execute(
                "insert into full_text_index_jobs(attachment_id,state,page_count,updated_at) values(?1,'completed',1,'index-v1')",
                [&attachment_id],
            )
            .map_err(|_| AcceptanceError::Setup)?;
        connection
            .execute(
                "insert into attachment_page_texts(attachment_id,page_index,page_text,indexed_at) values(?1,0,?2,'index-v1')",
                rusqlite::params![&attachment_id, format!("Indexed page for verification code {verification_code}")],
            )
            .map_err(|_| AcceptanceError::Setup)?;
        connection
            .execute(
                "insert into paper_reading_profile_selections(paper_id,profile_refs_json,revision,created_at,updated_at) values(?1,'[{\"profileId\":\"generic_overview\",\"profileVersion\":1}]',1,'now','now')",
                [&paper_id],
            )
            .map_err(|_| AcceptanceError::Setup)?;
        connection
            .execute(
                "insert into llm_provider_configs(provider_id,enabled,base_url,timeout_ms,updated_at) values('openai',1,'',120000,'now') on conflict(provider_id) do update set enabled=1,base_url='',timeout_ms=120000",
                [],
            )
            .map_err(|_| AcceptanceError::Setup)?;
    }
    let credentials =
        crate::ai::credential::JsonCredentialStore::from_path(work.join("credentials.json"));
    let sessions = Arc::new(
        crate::agent::service::AgentSessionService::from_app_store(&app_store)
            .map_err(|_| AcceptanceError::Setup)?,
    );
    let models =
        crate::agent::model::AgentModelResolver::from_app_store(&app_store, credentials.clone())
            .with_chatgpt(Arc::new(
                crate::agent::authentication::AgentChatGptAuthentication::new(bridge),
            ));
    let run = crate::ai::paper_overview_docling::run_docling_paper_overview(
        app_store.clone(),
        credentials,
        models,
        crate::ai::model::DoclingPaperOverviewDraft {
            paper_id: paper_id.clone(),
            attachment_ids: vec![attachment_id],
            context_snapshot_id: None,
            overview_profile_selection_revision: 1,
            paper_template_ref: None,
            model_selection: Some(crate::ai::model::LlmModelSelectionDraft {
                provider_id: "openai".into(),
                model_id: "gpt-5.6-terra".into(),
                reasoning_effort: None,
            }),
            image_assets: Vec::new(),
            shared_conversation_id: None,
            shared_request_id: None,
        },
        Some(crate::ai::provider::CancellationSignal::default()),
        Some(sessions),
    )
    .await
    .map_err(|_| AcceptanceError::PaperOverview)?;
    if run.status != "needs_review" || run.provider_id != "openai" {
        return Err(AcceptanceError::PaperOverview);
    }
    let connection = app_store.connection().map_err(|_| AcceptanceError::Setup)?;
    let (candidate_kind, review_state, payload_json): (String, String, String) = connection
        .query_row(
            "select candidate_kind,review_state,(select payload_json from ai_candidate_revisions where candidate_id=ai_candidates.id and revision=ai_candidates.current_revision) from ai_candidates where run_id=?1",
            [&run.id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(|_| AcceptanceError::PaperCandidate)?;
    if candidate_kind != "paper_overview_unit_v1"
        || review_state != "unreviewed"
        || !payload_json.contains("research_question")
        || !payload_json.contains("P1-T1")
    {
        return Err(AcceptanceError::PaperCandidate);
    }
    let evidence_count: i64 = connection
        .query_row(
            "select count(*) from ai_evidence_items item join ai_evidence_bundles bundle on bundle.id=item.bundle_id where bundle.run_id=?1 and item.citation_key='P1-T1'",
            [&run.id],
            |row| row.get(0),
        )
        .map_err(|_| AcceptanceError::PaperCandidate)?;
    if evidence_count != 1 {
        return Err(AcceptanceError::PaperCandidate);
    }
    Ok(())
}

async fn run_real_paper_reading_agent(
    bridge: CodexAppServerManager,
    work: &std::path::Path,
    with_image: bool,
    continue_conversation: bool,
    reopen_between_turns: bool,
    include_child: bool,
) -> Result<(), AcceptanceError> {
    let database_path = work.join("agent.sqlite");
    let objects_path = work.join("objects");
    let mut db = Connection::open(&database_path).map_err(|_| AcceptanceError::Setup)?;
    crate::storage::migrate(&mut db).map_err(|_| AcceptanceError::Setup)?;
    let app_store = AppStore::for_test(db, objects_path.clone());
    let selected = std::path::PathBuf::from(
        std::env::var_os("RESEARCH_ASSISTANT_TEST_LITERATURE_PACKAGE")
            .ok_or(AcceptanceError::Setup)?,
    );
    let catalog = crate::runtime_plugins::RuntimePluginCatalog::new(Some(
        selected
            .parent()
            .ok_or(AcceptanceError::Setup)?
            .to_path_buf(),
    ));
    let selected_key = app_store
        .native_note_trace()
        .ok_or(AcceptanceError::Setup)?
        .package_key
        .clone();
    let package = catalog
        .discover()
        .map_err(|_| AcceptanceError::Setup)?
        .packages
        .into_iter()
        .find(|package| package.package_key == selected_key)
        .ok_or(AcceptanceError::Setup)?;

    std::fs::create_dir_all(app_store.content_directory()).map_err(|_| AcceptanceError::Setup)?;
    let service = Arc::new(
        AgentSessionService::from_app_store(&app_store).map_err(|_| AcceptanceError::Setup)?,
    );
    let conversation_id = Uuid::new_v4();
    let paper_id = format!("smoke-paper-{}", Uuid::new_v4());
    let attachment_id = format!("smoke-attachment-{}", Uuid::new_v4());
    let pdf_hash = "b".repeat(64);
    let image_bytes: &[u8] = &[
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x04, 0x00, 0x00, 0x00, 0xb5,
        0x1c, 0x0c, 0x02, 0x00, 0x00, 0x00, 0x0b, 0x49, 0x44, 0x41, 0x54, 0x78, 0xda, 0x63, 0x64,
        0xf8, 0x0f, 0x00, 0x01, 0x05, 0x01, 0x01, 0x27, 0x18, 0xe3, 0x66, 0x00, 0x00, 0x00, 0x00,
        0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
    ];
    let image_hash = format!("{:x}", Sha256::digest(image_bytes));
    let parse_run_id = format!("smoke-parse-{}", Uuid::new_v4());
    let text = format!(
        "Frozen paper verification code: {}",
        Uuid::new_v4().simple()
    );
    let snapshot = {
        let mut connection = app_store.connection().map_err(|_| AcceptanceError::Setup)?;
        connection
        .execute(
            "insert into bibliographic_items(id,item_type,created_at,updated_at) values(?1,'document','now','now')",
            [&paper_id],
        )
        .map_err(|_| AcceptanceError::Setup)?;
        connection
        .execute(
            "insert into content_objects(sha256,relative_path,mime_type,byte_size,created_at) values(?1,?2,'application/pdf',1,'now')",
            rusqlite::params![&pdf_hash, format!("{paper_id}.pdf")],
        )
        .map_err(|_| AcceptanceError::Setup)?;
        if with_image {
            let image_relative_path = format!(
                "ai-reading-images/{}/{}/{}.png",
                &image_hash[..2],
                &image_hash[2..4],
                image_hash
            );
            let image_path = app_store.content_directory().join(&image_relative_path);
            std::fs::create_dir_all(image_path.parent().ok_or(AcceptanceError::Setup)?)
                .map_err(|_| AcceptanceError::Setup)?;
            std::fs::write(&image_path, image_bytes).map_err(|_| AcceptanceError::Setup)?;
            connection
            .execute(
                "insert into content_objects(sha256,relative_path,mime_type,byte_size,created_at) values(?1,?2,'image/png',?3,'now')",
                rusqlite::params![&image_hash, &image_relative_path, image_bytes.len() as i64],
            )
            .map_err(|_| AcceptanceError::Setup)?;
        }
        connection
        .execute(
            "insert into attachments(id,item_id,content_sha256,display_name,created_at) values(?1,?2,?3,'smoke.pdf','now')",
            rusqlite::params![&attachment_id, &paper_id, &pdf_hash],
        )
        .map_err(|_| AcceptanceError::Setup)?;
        connection
        .execute(
            "insert into docling_parse_runs(id,item_id,attachment_id,content_sha256,service_base_url,docling_status,parse_status,json_content,raw_response,created_at,finished_at) values(?1,?2,?3,?4,'http://127.0.0.1:1','success','success','{}','{}','now','now')",
            rusqlite::params![&parse_run_id, &paper_id, &attachment_id, &pdf_hash],
        )
        .map_err(|_| AcceptanceError::Setup)?;
        let mut context_items = vec![crate::ai::model::PaperAiReadingContextItem {
            id: "P1-T1".into(),
            paper_id: paper_id.clone(),
            attachment_id: attachment_id.clone(),
            content_sha256: pdf_hash.clone(),
            parse_run_id: parse_run_id.clone(),
            raw_ref: "#/text/0".into(),
            kind: "text".into(),
            label: Some("verification".into()),
            page_index: Some(0),
            regions: vec![],
            charspan: Some([0, text.chars().count() as u32]),
            source_text_hash: Some(format!("{:x}", Sha256::digest(text.as_bytes()))),
            text: Some(text.clone()),
            table_json: None,
        }];
        let mut image_requests = Vec::new();
        let mut image_assets =
            std::collections::BTreeMap::<String, crate::ai::paper_context::StoredImageAsset>::new();
        if with_image {
            context_items.push(crate::ai::model::PaperAiReadingContextItem {
                id: "P1-F1".into(),
                paper_id: paper_id.clone(),
                attachment_id: attachment_id.clone(),
                content_sha256: pdf_hash.clone(),
                parse_run_id: parse_run_id.clone(),
                raw_ref: "#/picture/0".into(),
                kind: "picture".into(),
                label: Some("synthetic figure".into()),
                page_index: Some(0),
                regions: vec![crate::ai::model::PaperAiReadingRegion {
                    page_index: 0,
                    rect: crate::ai::model::PaperAiReadingRect {
                        x: 0.1,
                        y: 0.1,
                        width: 0.8,
                        height: 0.8,
                    },
                }],
                charspan: None,
                source_text_hash: None,
                text: None,
                table_json: None,
            });
            image_requests.push(crate::ai::model::PaperAiReadingImageRequest {
                context_item_id: "P1-F1".into(),
                attachment_id: attachment_id.clone(),
                page_index: 0,
                regions: vec![],
                label: Some("synthetic figure".into()),
            });
            image_assets.insert(
                "P1-F1".into(),
                crate::ai::paper_context::StoredImageAsset {
                    context_item_id: "P1-F1".into(),
                    content_sha256: image_hash.clone(),
                    data_url: "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=".into(),
                },
            );
        }
        let snapshot = crate::runtime_plugins::literature_context::test_fixture::seed_snapshot(
            &catalog,
            &package,
            &mut connection,
            &paper_id,
            &crate::ai::model::PaperAiReadingPrepareResult {
                session: None,
                attachment_statuses: vec![crate::ai::model::PaperAiReadingAttachmentStatus {
                    attachment_id: attachment_id.clone(),
                    display_name: "smoke.pdf".into(),
                    content_sha256: pdf_hash.clone(),
                    parse_run_id: Some(parse_run_id.clone()),
                    parse_status: "available".into(),
                }],
                context_items,
                image_requests,
                text_item_count: 1,
                table_item_count: 0,
                picture_item_count: if with_image { 1 } else { 0 },
                model: None,
            },
            image_assets
                .iter()
                .map(|(id, asset)| (id.clone(), asset.content_sha256.clone()))
                .collect(),
        );
        crate::agent::repository::create_conversation(
            &mut connection,
            ConversationId::from_uuid(conversation_id),
            "Paper reading smoke",
        )
        .map_err(|_| AcceptanceError::Setup)?;
        connection
        .execute(
            "insert into agent_conversation_bindings(conversation_id,resource_kind,resource_id) values(?1,'paper',?2)",
            rusqlite::params![conversation_id.to_string(), &paper_id],
        )
        .map_err(|_| AcceptanceError::Setup)?;
        snapshot
    };

    let credentials =
        crate::ai::credential::JsonCredentialStore::from_path(work.join("credentials.json"));
    let models = crate::agent::model::AgentModelResolver::from_app_store(&app_store, credentials)
        .with_chatgpt(Arc::new(
            crate::agent::authentication::AgentChatGptAuthentication::new(bridge.clone()),
        ));
    let source_id = {
        let preview_connection = app_store.connection().map_err(|_| AcceptanceError::Setup)?;
        let preview = crate::runtime_plugins::literature_context::test_fixture::expected_preview(
            &catalog,
            &package,
            &preview_connection,
            &crate::ai::paper_reading_contracts::PaperContextRequest {
                paper_id: paper_id.clone(),
                snapshot_id: snapshot.id.clone(),
                expected_context_hash: snapshot.context_hash.clone(),
            },
        );
        let expected_context =
            crate::ai::paper_context::agent_preview::submission_context_from_preview(preview)
                .map_err(|_| AcceptanceError::Setup)?;
        expected_context
            .first()
            .map(|source| source.source_id.clone())
            .ok_or(AcceptanceError::Setup)?
    };
    let record = crate::agent::reading_submission::prepare(
        &app_store,
        &models,
        crate::agent::reading_submission::PrepareRequest {
            user_input: vec![],
            conversation_id,
            request_id: Uuid::new_v4(),
            paper_id: paper_id.clone(),
            paper_context_snapshot_id: snapshot.id.clone(),
            expected_context_hash: snapshot.context_hash.clone(),
            retrieval: None,
            text: format!("Call research_read_source exactly once for sourceId {source_id}. Then reply with only the verification code from that source."),
            model: Some(crate::agent::model::ModelSelection { provider_id: "openai".into(), model_id: "gpt-5.6-terra".into(), reasoning_effort: None, service_tier: None }),
            session_config: None,
            turn_config: None,
        },
    )
    .await
    .map_err(|_| AcceptanceError::Setup)?;
    let request_id = record.request_id;
    let turn_id = record.turn_id;
    let model = record.input.model.clone();
    let admission = crate::agent::reading_start::start(
        &app_store,
        &models,
        service.clone(),
        crate::agent::reading_start::StartRequest {
            conversation_id,
            request_id,
            paper_id: paper_id.clone(),
            paper_context_snapshot_id: snapshot.id.clone(),
            context_snapshot_id: record.context_snapshot_id,
            model,
            mode: crate::agent::execution_profile::AgentExecutionMode::ChatGptDefault,
        },
    )
    .await
    .map_err(|_| AcceptanceError::Admission)?;
    let SubmissionAdmission::Started(owned) = admission else {
        return Err(AcceptanceError::Admission);
    };
    let task_completion = owned.task.wait().await;
    if task_completion != TaskCompletion::Completed {
        return Err(AcceptanceError::Task);
    }
    let projection = service
        .open(ConversationId::from_uuid(conversation_id))
        .await
        .map_err(|_| AcceptanceError::Projection)?;
    let turn = projection
        .turn(turn_id)
        .ok_or(AcceptanceError::Projection)?;
    if turn.outcome != Some(TurnOutcome::Completed) {
        return Err(AcceptanceError::Projection);
    }
    if !turn.items.values().any(|item| matches!(&item.kind, ItemKind::ToolCall { name, .. } if name == "research_read_source")) { return Err(AcceptanceError::MissingToolCall); }
    if !turn.items.values().any(|item| matches!(&item.kind, ItemKind::ToolResult { is_error: Some(false), .. } if item.text.contains(&text))) { return Err(AcceptanceError::MissingToolResult); }
    let first_answer = turn
        .items
        .values()
        .find_map(|item| {
            matches!(item.kind, ItemKind::AssistantMessage)
                .then(|| item.text.trim().to_owned())
                .filter(|text| !text.is_empty())
        })
        .ok_or(AcceptanceError::MissingAnswer)?;

    let (app_store, models, service) = if reopen_between_turns {
        drop(projection);
        drop(service);
        drop(models);
        drop(app_store);
        let reopened_store = AppStore::for_test(
            Connection::open(&database_path).map_err(|_| AcceptanceError::Setup)?,
            objects_path,
        );
        let reopened_service = Arc::new(
            AgentSessionService::from_app_store(&reopened_store)
                .map_err(|_| AcceptanceError::Setup)?,
        );
        let credentials =
            crate::ai::credential::JsonCredentialStore::from_path(work.join("credentials.json"));
        let reopened_models =
            crate::agent::model::AgentModelResolver::from_app_store(&reopened_store, credentials)
                .with_chatgpt(Arc::new(
                    crate::agent::authentication::AgentChatGptAuthentication::new(bridge),
                ));
        (reopened_store, reopened_models, reopened_service)
    } else {
        (app_store, models, service)
    };

    let paper_id_for_child = paper_id.clone();
    let paper_snapshot_for_child = snapshot.id.clone();
    if continue_conversation {
        let second_record = crate::agent::reading_submission::prepare(
        &app_store,
        &models,
        crate::agent::reading_submission::PrepareRequest {
            user_input: vec![],
            conversation_id,
            request_id: Uuid::new_v4(),
            paper_id: paper_id.clone(),
            paper_context_snapshot_id: snapshot.id.clone(),
            expected_context_hash: snapshot.context_hash.clone(),
            retrieval: None,
            text: "Answer with only the exact verification code you returned in the previous turn. Do not call any tools.".into(),
            model: Some(crate::agent::model::ModelSelection {
                provider_id: "openai".into(),
                model_id: "gpt-5.6-terra".into(),
                reasoning_effort: None,
                service_tier: None,
            }),
            session_config: None,
            turn_config: None,
        },
    )
    .await
    .map_err(|_| AcceptanceError::Setup)?;
        let second_turn_id = second_record.turn_id;
        let second_admission = crate::agent::reading_start::start(
            &app_store,
            &models,
            service.clone(),
            crate::agent::reading_start::StartRequest {
                conversation_id,
                request_id: second_record.request_id,
                paper_id,
                paper_context_snapshot_id: snapshot.id,
                context_snapshot_id: second_record.context_snapshot_id,
                model: second_record.input.model,
                mode: crate::agent::execution_profile::AgentExecutionMode::ChatGptDefault,
            },
        )
        .await
        .map_err(|_| AcceptanceError::Admission)?;
        let SubmissionAdmission::Started(second_owned) = second_admission else {
            return Err(AcceptanceError::Admission);
        };
        if second_owned.task.wait().await != TaskCompletion::Completed {
            return Err(AcceptanceError::Task);
        }
        let second_projection = service
            .open(ConversationId::from_uuid(conversation_id))
            .await
            .map_err(|_| AcceptanceError::Projection)?;
        let second_turn = second_projection
            .turn(second_turn_id)
            .ok_or(AcceptanceError::Projection)?;
        if second_turn.outcome != Some(TurnOutcome::Completed)
            || !second_turn.items.values().any(|item| {
                matches!(item.kind, ItemKind::AssistantMessage)
                    && item.text.trim().contains(&first_answer)
            })
        {
            return Err(AcceptanceError::MissingAnswer);
        }
    }

    if include_child {
        let parent_record = crate::agent::reading_submission::prepare(
            &app_store,
            &models,
            crate::agent::reading_submission::PrepareRequest {
            user_input: vec![],
                conversation_id,
                request_id: Uuid::new_v4(),
                paper_id: paper_id_for_child.clone(),
                paper_context_snapshot_id: paper_snapshot_for_child.clone(),
                expected_context_hash: snapshot.context_hash.clone(),
                retrieval: None,
                text: "Call research_spawn_subtask exactly once with the prompt `Reply with only CHILD_OK.`. After the child task is started, reply with only CHILD_STARTED.".into(),
                model: Some(crate::agent::model::ModelSelection {
                    provider_id: "openai".into(),
                    model_id: "gpt-5.6-terra".into(),
                    reasoning_effort: None,
                    service_tier: None,
                }),
                session_config: None,
                turn_config: None,
            },
        )
        .await
        .map_err(|_| AcceptanceError::Setup)?;
        let parent_turn_id = parent_record.turn_id;
        let parent_admission = crate::agent::reading_start::start(
            &app_store,
            &models,
            service.clone(),
            crate::agent::reading_start::StartRequest {
                conversation_id,
                request_id: parent_record.request_id,
                paper_id: paper_id_for_child.clone(),
                paper_context_snapshot_id: paper_snapshot_for_child.clone(),
                context_snapshot_id: parent_record.context_snapshot_id,
                model: parent_record.input.model,
                mode: crate::agent::execution_profile::AgentExecutionMode::ChatGptDefault,
            },
        )
        .await
        .map_err(|_| AcceptanceError::Admission)?;
        let SubmissionAdmission::Started(parent_owned) = parent_admission else {
            return Err(AcceptanceError::Admission);
        };
        let parent_completion = parent_owned.task.wait().await;
        if parent_completion != TaskCompletion::Completed {
            if let Ok(projection) = service
                .open(ConversationId::from_uuid(conversation_id))
                .await
                && let Some(turn) = projection.turn(parent_turn_id)
            {
                eprintln!(
                    "paper child parent terminal {:?}; items: {:?}",
                    turn.outcome,
                    turn.items
                        .values()
                        .map(|item| format!("{:?}", item.kind))
                        .collect::<Vec<_>>()
                );
            }
            return Err(AcceptanceError::Task);
        }
        let parent_projection = service
            .open(ConversationId::from_uuid(conversation_id))
            .await
            .map_err(|_| AcceptanceError::Projection)?;
        let parent_turn = parent_projection
            .turn(parent_turn_id)
            .ok_or(AcceptanceError::ChildTurnMissing)?;
        let child_response = parent_turn.items.values().find_map(|item| {
            if !matches!(
                &item.kind,
                ItemKind::ToolResult {
                    is_error: Some(false),
                    ..
                }
            ) {
                return None;
            }
            serde_json::from_str::<Value>(&item.text).ok()
        });
        let child_conversation_id = child_response
            .as_ref()
            .and_then(|value| value.get("childConversationId"))
            .and_then(Value::as_str)
            .and_then(|value| Uuid::parse_str(value).ok())
            .ok_or(AcceptanceError::ChildToolResultMissing)?;
        let child_turn_id = child_response
            .as_ref()
            .and_then(|value| value.get("turnId"))
            .and_then(Value::as_str)
            .and_then(|value| Uuid::parse_str(value).ok())
            .ok_or(AcceptanceError::ChildTurnMissing)?;
        let mut child_turn = None;
        for _ in 0..120 {
            let projection = service
                .open(ConversationId::from_uuid(child_conversation_id))
                .await
                .map_err(|_| AcceptanceError::Projection)?;
            if let Some(turn) = projection.turn(child_turn_id).cloned()
                && turn.outcome.is_some()
            {
                child_turn = Some(turn);
                break;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        let child_turn = child_turn.ok_or(AcceptanceError::ChildTurnTimeout)?;
        if child_turn.outcome != Some(TurnOutcome::Completed) {
            eprintln!(
                "paper child terminal outcome: {:?}; items: {:?}",
                child_turn.outcome,
                child_turn
                    .items
                    .values()
                    .map(|item| format!("{:?}", item.kind))
                    .collect::<Vec<_>>()
            );
            return Err(AcceptanceError::ChildOutcome);
        }
        if !child_turn.items.values().any(|item| {
            matches!(item.kind, ItemKind::AssistantMessage) && item.text.contains("CHILD_OK")
        }) {
            return Err(AcceptanceError::ChildAnswerMissing);
        }
    }
    Ok(())
}

#[tokio::test]
#[ignore = "real model/tool calls; requires explicit opt-in, app-owned login, and closed development application"]
async fn authenticated_chatgpt_agent_execution_compiles_approved_copy() {
    let _owner = PROFILE_OWNER.lock().await;
    let (bridge, work) = development_bridge();
    let outcome = tokio::time::timeout(
        Duration::from_secs(180),
        run_real_compile_agent(bridge.clone(), work.path()),
    )
    .await;
    bridge.stop().await;
    outcome
        .expect("real writing compile acceptance timed out")
        .expect("real writing compile acceptance failed");
}

async fn run_real_compile_agent(
    bridge: CodexAppServerManager,
    work: &std::path::Path,
) -> Result<(), AcceptanceError> {
    use crate::runtime_plugins::{
        RuntimePluginCatalog, native::NativePreviewService, writing_projects::context::Binding,
    };
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .ok_or(AcceptanceError::Setup)?;
    let directory = std::env::var_os("RESEARCH_ASSISTANT_TEST_WRITING_V2_PACKAGES")
        .ok_or(AcceptanceError::Setup)
        .and_then(|path| std::fs::canonicalize(path).map_err(|_| AcceptanceError::Setup))?;
    if !directory.starts_with(repository.join(".tmp")) {
        return Err(AcceptanceError::Setup);
    }
    let catalog = Arc::new(RuntimePluginCatalog::new(Some(directory)));
    let listing = catalog.discover().map_err(|_| AcceptanceError::Setup)?;
    let [package] = listing.packages.as_slice() else {
        return Err(AcceptanceError::Setup);
    };
    let native_service = Arc::new(NativePreviewService::new(Arc::clone(&catalog)));
    let binding = Binding {
        service: Arc::clone(&native_service),
        catalog,
        package_key: package.package_key.clone(),
    };
    let outcome = run_real_compile_agent_with_binding(bridge, work, binding).await;
    let shutdown = native_service
        .shutdown()
        .await
        .map_err(|_| AcceptanceError::Setup);
    outcome.and(shutdown)
}

async fn run_real_compile_agent_with_binding(
    bridge: CodexAppServerManager,
    work: &std::path::Path,
    binding: crate::runtime_plugins::writing_projects::context::Binding,
) -> Result<(), AcceptanceError> {
    use crate::runtime_plugins::writing_projects::compile::workspace::AgentCompileSource;
    let mut db = Connection::open_in_memory().map_err(|_| AcceptanceError::Setup)?;
    crate::storage::migrate(&mut db).map_err(|_| AcceptanceError::Setup)?;
    let app_store = AppStore::for_test(db, work.join("objects"))
        .without_static_writing_context_sources()
        .without_static_writing_compile_sources();
    #[cfg(unix)]
    let app_store = app_store.without_static_writing_workspace_source();
    let app_store = app_store.with_native_writing_context(binding.clone());
    let service = Arc::new(
        AgentSessionService::from_app_store(&app_store).map_err(|_| AcceptanceError::Setup)?,
    );
    let (conversation_id, manuscript_id, _, source_hash, _) = seed_writing_manuscript(&app_store);
    let nested_source = app_store
        .manuscript_directory()
        .join(manuscript_id.to_string())
        .join("source/sections/introduction.tex");
    std::fs::create_dir_all(nested_source.parent().ok_or(AcceptanceError::Setup)?)
        .map_err(|_| AcceptanceError::Setup)?;
    let nested_bytes = b"\\section{Agent nested source.}\\n";
    std::fs::write(&nested_source, nested_bytes).map_err(|_| AcceptanceError::Setup)?;
    let nested_hash = format!("{:x}", Sha256::digest(nested_bytes));
    app_store
        .connection()
        .map_err(|_| AcceptanceError::Setup)?
        .execute(
            "insert into manuscript_files(manuscript_id,relative_path,file_kind,byte_size,sha256,modified_at) values (?1,'sections/introduction.tex','tex',?2,?3,'now')",
            rusqlite::params![manuscript_id.to_string(), nested_bytes.len(), nested_hash],
        )
        .map_err(|_| AcceptanceError::Setup)?;
    let credentials =
        crate::ai::credential::JsonCredentialStore::from_path(work.join("credentials.json"));
    let models = crate::agent::model::AgentModelResolver::from_app_store(&app_store, credentials)
        .with_chatgpt(Arc::new(
            crate::agent::authentication::AgentChatGptAuthentication::new(bridge),
        ));
    let writing =
        WritingSubmissionService::from_app_store(&app_store).with_model_resolver(models.clone());
    let source_path = app_store
        .manuscript_directory()
        .join(manuscript_id.to_string())
        .join("source/main.tex");
    let original = std::fs::read(&source_path).map_err(|_| AcceptanceError::Setup)?;
    let record = writing
        .prepare(
            &models,
            WritingSubmissionRequest {
                resources: vec![],
            user_input: vec![],
                conversation_id,
                request_id: Uuid::new_v4(),
                text: "Call research_compile_workspace exactly once with rootPath `main.tex` and engine `pdflatex`. After the tool returns, reply with only COMPILE_READY."
                    .into(),
                context: WritingContextRequest {
                    manuscript_id,
                    files: vec![WritingContextFile {
                        relative_path: "main.tex".into(),
                        expected_disk_sha256: source_hash,
                        buffer: None,
                        selection: None,
                    }],
                },
                retrieval: None,
                session_config: None,
                turn_config: None,
                model: Some(crate::agent::model::AgentModelSelection {
                    provider_id: "openai".into(),
                    model_id: "gpt-5.6-terra".into(),
                    reasoning_effort: None,
                    service_tier: None,
                }),
            },
        )
        .await
        .map_err(|_| AcceptanceError::Setup)?;
    let receipt = WritingSubmissionReceipt {
        conversation_id,
        request_id: record.record.request_id,
        manuscript_id,
    };
    let prepared = writing
        .prepare_workspace(
            receipt,
            record.record.context_snapshot_id,
            WritingWorkspaceRequest {
                manuscript_id,
                buffers: vec![],
            },
        )
        .await
        .map_err(|_| AcceptanceError::Admission)?;
    let approved = writing
        .approve_workspace(prepared)
        .await
        .map_err(|_| AcceptanceError::Admission)?;
    let approved_root = approved
        .lease
        .working_directory()
        .map_err(|_| AcceptanceError::Setup)?;
    std::fs::create_dir_all(approved_root.join("sections")).map_err(|_| AcceptanceError::Setup)?;
    std::fs::write(
        approved_root.join("main.tex"),
        b"\\documentclass{article}\n\\begin{document}\n\\input{sections/introduction}\n\\end{document}\n",
    )
    .map_err(|_| AcceptanceError::Setup)?;
    std::fs::write(
        approved_root.join("sections/introduction.tex"),
        b"\\section{Agent nested source.}\n",
    )
    .map_err(|_| AcceptanceError::Setup)?;
    drop(approved);
    let model = record.record.input.model.clone();
    let profile = crate::agent::execution_profile::execution_profile(
        &model,
        crate::agent::execution_profile::AgentExecutionMode::ChatGptDefault,
    )
    .map_err(|_| AcceptanceError::Setup)?;
    let request_id = record.record.request_id;
    let turn_id = record.record.turn_id;
    let context_snapshot_id = record.record.context_snapshot_id;
    let SubmissionAdmission::Started(owned) = writing
        .start_confirmed(
            service.clone(),
            record,
            WritingExecutionConfirmation {
                receipt,
                context_snapshot_id,
                model,
            },
            profile,
        )
        .await
        .map_err(|_| AcceptanceError::Admission)?
    else {
        return Err(AcceptanceError::Admission);
    };
    if owned.task.wait().await != TaskCompletion::Completed {
        return Err(AcceptanceError::Task);
    }
    let projection = service
        .open(ConversationId::from_uuid(conversation_id))
        .await
        .map_err(|_| AcceptanceError::Projection)?;
    let turn = projection
        .turn(turn_id)
        .ok_or(AcceptanceError::Projection)?;
    if turn.outcome != Some(TurnOutcome::Completed) {
        return Err(AcceptanceError::Projection);
    }
    if !turn.items.values().any(|item| {
        matches!(&item.kind, ItemKind::ToolCall { name, .. } if name == "research_compile_workspace")
    }) {
        return Err(AcceptanceError::MissingCompileCall);
    }
    let expected_conversation = conversation_id.to_string();
    let expected_request = request_id.to_string();
    let expected_turn = turn_id.to_string();
    let expected_snapshot = context_snapshot_id.to_string();
    let compile_result = turn.items.values().find_map(|item| {
        if !matches!(
            item.kind,
            ItemKind::ToolResult {
                is_error: Some(false),
                ..
            }
        ) {
            return None;
        }
        let value = serde_json::from_str::<Value>(&item.text).ok()?;
        (value.get("status").and_then(Value::as_str) == Some("succeeded")
            && value.get("resultId") == value.get("compileVersion")
            && value
                .get("identity")
                .and_then(|identity| identity.get("conversationId"))
                .and_then(Value::as_str)
                == Some(expected_conversation.as_str())
            && value
                .get("identity")
                .and_then(|identity| identity.get("requestId"))
                .and_then(Value::as_str)
                == Some(expected_request.as_str())
            && value
                .get("identity")
                .and_then(|identity| identity.get("turnId"))
                .and_then(Value::as_str)
                == Some(expected_turn.as_str())
            && value
                .get("identity")
                .and_then(|identity| identity.get("contextSnapshotId"))
                .and_then(Value::as_str)
                == Some(expected_snapshot.as_str())
            && value
                .get("artifacts")
                .and_then(|artifacts| artifacts.get("pdf"))
                .is_some_and(|value| !value.is_null())
            && value
                .get("artifacts")
                .and_then(|artifacts| artifacts.get("synctex"))
                .is_some_and(|value| !value.is_null()))
        .then_some(value)
    });
    if compile_result.is_none() {
        return Err(AcceptanceError::CompileResult);
    }
    let compile_result = compile_result.expect("checked compile result");
    let result_id = compile_result
        .get("resultId")
        .and_then(Value::as_str)
        .ok_or(AcceptanceError::CompileResult)?;
    let result_directory = app_store
        .writing_cache_directory()
        .join("agent-compile-results")
        .join(result_id);
    let compile_store = app_store.clone();
    let runtime = tokio::runtime::Handle::current();
    tokio::task::spawn_blocking(move || {
        let synctex = AgentCompileSource::new(binding, runtime)
            .map_err(|_| AcceptanceError::CompileResult)?;
        let toolchain_json = serde_json::to_string(
            &synctex
                .detect(&compile_store)
                .map_err(|_| AcceptanceError::CompileResult)?,
        )
        .map_err(|_| AcceptanceError::CompileResult)?;
        let runtime_home = compile_store
            .writing_cache_directory()
            .join("toolchain-home");
        let snapshot = result_directory.join("source");
        let pdf = result_directory.join("result.pdf");
        let query = crate::agent::compile_synctex::AgentCompileSynctexQuery {
            toolchain_json: &toolchain_json,
            runtime_home: &runtime_home,
            snapshot: &snapshot,
            output_directory: &result_directory,
            pdf: &pdf,
        };
        let nested_forward = synctex
            .forward(&query, "sections/introduction.tex", 1, 0)
            .map_err(|_| AcceptanceError::CompileResult)?;
        let nested_match = nested_forward
            .first()
            .ok_or(AcceptanceError::CompileResult)?;
        let nested_inverse = synctex
            .inverse(
                &query,
                nested_match.page,
                nested_match.x + nested_match.width / 2.0,
                nested_match.y + nested_match.height / 2.0,
            )
            .map_err(|_| AcceptanceError::CompileResult)?;
        if !nested_inverse
            .iter()
            .any(|location| location.relative_path == "sections/introduction.tex")
        {
            return Err(AcceptanceError::CompileResult);
        }
        Ok(())
    })
    .await
    .map_err(|_| AcceptanceError::CompileResult)??;
    if !turn.items.values().any(|item| {
        matches!(item.kind, ItemKind::AssistantMessage) && item.text.contains("COMPILE_READY")
    }) {
        return Err(AcceptanceError::MissingAnswer);
    }
    if std::fs::read(source_path).map_err(|_| AcceptanceError::Setup)? != original {
        return Err(AcceptanceError::Application);
    }
    Ok(())
}

async fn run_shared_agent(
    bridge: CodexAppServerManager,
    work: &std::path::Path,
    include_child: bool,
    dirty_buffer: bool,
    selected_context: bool,
    reopen_between_turns: bool,
) -> Result<(), AcceptanceError> {
    let database_path = work.join("writing-agent.sqlite");
    let objects_path = work.join("objects");
    let mut db = Connection::open(&database_path).map_err(|_| AcceptanceError::Setup)?;
    crate::storage::migrate(&mut db).map_err(|_| AcceptanceError::Setup)?;
    let app_store = AppStore::for_test(db, objects_path.clone());
    let service = Arc::new(
        AgentSessionService::from_app_store(&app_store).map_err(|_| AcceptanceError::Setup)?,
    );
    let (conversation_id, manuscript_id, source_id, source_hash, verification_code) =
        seed_writing_manuscript(&app_store);
    let include_buffer = dirty_buffer || selected_context;
    let buffer_revision = include_buffer.then(Uuid::new_v4);
    let expected_code = if include_buffer {
        format!("dirty-{}", Uuid::new_v4().simple())
    } else {
        verification_code.clone()
    };
    let buffer_text = include_buffer.then(|| {
        if selected_context {
            format!("prefix\nverification code: {expected_code}\nsuffix")
        } else {
            format!("Unsaved editor buffer\nverification code: {expected_code}")
        }
    });
    let selection = selected_context.then(|| {
        let start_utf16 = "prefix\nverification code: ".encode_utf16().count();
        WritingContextSelection {
            start_utf16,
            end_utf16: start_utf16 + expected_code.encode_utf16().count(),
        }
    });
    let credentials =
        crate::ai::credential::JsonCredentialStore::from_path(work.join("credentials.json"));
    let models = crate::agent::model::AgentModelResolver::from_app_store(&app_store, credentials)
        .with_chatgpt(Arc::new(
            crate::agent::authentication::AgentChatGptAuthentication::new(bridge.clone()),
        ));
    let writing =
        WritingSubmissionService::from_app_store(&app_store).with_model_resolver(models.clone());
    let record = writing
        .prepare(
            &models,
            WritingSubmissionRequest {
                resources: vec![],
            user_input: vec![],
                conversation_id,
                request_id: Uuid::new_v4(),
                text: format!(
                    "You must call research_read_source exactly once for sourceId {source_id} before answering. Do not answer from memory or metadata. After the tool returns, reply with only the verification code contained in that source."
                ),
                context: WritingContextRequest {
                    manuscript_id,
                    files: vec![WritingContextFile {
                        relative_path: "main.tex".into(),
                        expected_disk_sha256: source_hash.clone(),
                        buffer: buffer_text.as_ref().zip(buffer_revision).map(|(text, revision)| {
                            WritingContextBuffer { revision, text: text.clone() }
                        }),
                        selection: selection.clone(),
                    }],
                },
                retrieval: None,
                session_config: None,
                turn_config: None,
                model: Some(crate::agent::model::AgentModelSelection {
                    provider_id: "openai".into(),
                    model_id: "gpt-5.6-terra".into(),
                    reasoning_effort: None,
                    service_tier: None,
                }),
            },
        )
        .await
        .map_err(|_| AcceptanceError::Setup)?;
    let request_id = record.record.request_id;
    if record.record.input.model.provider_id != "openai"
        || record.record.input.model.model_id != "gpt-5.6-terra"
    {
        return Err(AcceptanceError::Setup);
    }
    let turn_id = record.record.turn_id;
    let context_snapshot_id = record.record.context_snapshot_id;
    let model = record.record.input.model.clone();
    let profile = crate::agent::execution_profile::execution_profile(
        &model,
        crate::agent::execution_profile::AgentExecutionMode::ChatGptDefault,
    )
    .map_err(|_| AcceptanceError::Setup)?;
    let admission = writing
        .start_confirmed(
            service.clone(),
            record,
            WritingExecutionConfirmation {
                receipt: WritingSubmissionReceipt {
                    conversation_id,
                    request_id,
                    manuscript_id,
                },
                context_snapshot_id,
                model,
            },
            profile,
        )
        .await
        .map_err(|_| AcceptanceError::Admission)?;
    let SubmissionAdmission::Started(owned) = admission else {
        return Err(AcceptanceError::Admission);
    };
    let task = owned.task.wait().await;
    let projection = service
        .open(ConversationId::from_uuid(conversation_id))
        .await
        .map_err(|_| AcceptanceError::Projection)?;
    let turn = projection
        .turn(turn_id)
        .ok_or(AcceptanceError::Projection)?;
    if !turn.items.values().any(|item| {
        matches!(&item.kind, ItemKind::ToolCall { name, .. } if name == "research_read_source")
    }) {
        return Err(AcceptanceError::MissingToolCall);
    }
    if !turn.items.values().any(|item| {
        matches!(
            &item.kind,
            ItemKind::ToolResult {
                is_error: Some(false),
                ..
            }
        ) && item.text.contains(&expected_code)
    }) {
        return Err(AcceptanceError::MissingToolResult);
    }
    if !turn
        .items
        .values()
        .any(|item| matches!(item.kind, ItemKind::AssistantMessage) && !item.text.trim().is_empty())
    {
        return Err(AcceptanceError::MissingAnswer);
    }
    if turn.outcome != Some(TurnOutcome::Completed) {
        if task == TaskCompletion::Failed {
            return Err(AcceptanceError::Task);
        }
        return Err(AcceptanceError::Projection);
    }
    if task != TaskCompletion::Completed {
        return Err(AcceptanceError::Task);
    }

    let (app_store, models, service, writing) = if reopen_between_turns {
        drop(projection);
        drop(service);
        drop(models);
        drop(writing);
        drop(app_store);
        let reopened_store = AppStore::for_test(
            Connection::open(&database_path).map_err(|_| AcceptanceError::Setup)?,
            objects_path,
        );
        let reopened_service = Arc::new(
            AgentSessionService::from_app_store(&reopened_store)
                .map_err(|_| AcceptanceError::Setup)?,
        );
        let credentials =
            crate::ai::credential::JsonCredentialStore::from_path(work.join("credentials.json"));
        let reopened_models =
            crate::agent::model::AgentModelResolver::from_app_store(&reopened_store, credentials)
                .with_chatgpt(Arc::new(
                    crate::agent::authentication::AgentChatGptAuthentication::new(bridge.clone()),
                ));
        let reopened_writing = WritingSubmissionService::from_app_store(&reopened_store)
            .with_model_resolver(reopened_models.clone());
        (
            reopened_store,
            reopened_models,
            reopened_service,
            reopened_writing,
        )
    } else {
        (app_store, models, service, writing)
    };

    let second = writing
        .prepare(
            &models,
            WritingSubmissionRequest {
                resources: vec![],
            user_input: vec![],
                conversation_id,
                request_id: Uuid::new_v4(),
                text: format!(
                    "You must call research_read_source exactly once for sourceId {source_id} before answering. Do not answer from memory or metadata. After the tool returns, reply with only the verification code contained in that source again."
                ),
                context: WritingContextRequest {
                    manuscript_id,
                    files: vec![WritingContextFile {
                        relative_path: "main.tex".into(),
                        expected_disk_sha256: source_hash.clone(),
                        buffer: buffer_text.as_ref().zip(buffer_revision).map(|(text, revision)| {
                            WritingContextBuffer { revision, text: text.clone() }
                        }),
                        selection: selection.clone(),
                    }],
                },
                retrieval: None,
                session_config: None,
                turn_config: None,
                model: Some(crate::agent::model::AgentModelSelection {
                    provider_id: "openai".into(),
                    model_id: "gpt-5.6-terra".into(),
                    reasoning_effort: None,
                    service_tier: None,
                }),
            },
        )
        .await
        .map_err(|_| AcceptanceError::Setup)?;
    let second_turn_id = second.record.turn_id;
    let second_request_id = second.record.request_id;
    let second_context_snapshot_id = second.record.context_snapshot_id;
    let second_model = second.record.input.model.clone();
    let second_profile = crate::agent::execution_profile::execution_profile(
        &second_model,
        crate::agent::execution_profile::AgentExecutionMode::ChatGptDefault,
    )
    .map_err(|_| AcceptanceError::Setup)?;
    let second_admission = writing
        .start_confirmed(
            service.clone(),
            second,
            WritingExecutionConfirmation {
                receipt: WritingSubmissionReceipt {
                    conversation_id,
                    request_id: second_request_id,
                    manuscript_id,
                },
                context_snapshot_id: second_context_snapshot_id,
                model: second_model,
            },
            second_profile,
        )
        .await
        .map_err(|_| AcceptanceError::Admission)?;
    let SubmissionAdmission::Started(second_owned) = second_admission else {
        return Err(AcceptanceError::Admission);
    };
    if second_owned.task.wait().await != TaskCompletion::Completed {
        return Err(AcceptanceError::Task);
    }
    let second_projection = service
        .open(ConversationId::from_uuid(conversation_id))
        .await
        .map_err(|_| AcceptanceError::Projection)?;
    let second_turn = second_projection
        .turn(second_turn_id)
        .ok_or(AcceptanceError::Projection)?;
    if second_turn.outcome != Some(TurnOutcome::Completed)
        || !second_turn.items.values().any(|item| {
            matches!(&item.kind, ItemKind::ToolCall { name, .. } if name == "research_read_source")
        })
        || !second_turn.items.values().any(|item| {
            matches!(item.kind, ItemKind::AssistantMessage) && !item.text.trim().is_empty()
        })
    {
        return Err(AcceptanceError::Projection);
    }

    if !include_child {
        let source_path = app_store
            .manuscript_directory()
            .join(manuscript_id.to_string())
            .join("source/main.tex");
        let disk_after = std::fs::read(source_path).map_err(|_| AcceptanceError::Setup)?;
        if format!("{:x}", Sha256::digest(disk_after)) != source_hash {
            return Err(AcceptanceError::Application);
        }
        return Ok(());
    }

    let child_request = writing
        .prepare(
            &models,
            WritingSubmissionRequest {
                resources: vec![],
            user_input: vec![],
                conversation_id,
                request_id: Uuid::new_v4(),
                text: "Call research_spawn_subtask exactly once with the prompt `Reply with only CHILD_OK.`. After the child task is started, reply with only CHILD_STARTED.".into(),
                context: WritingContextRequest {
                    manuscript_id,
                    files: vec![WritingContextFile {
                        relative_path: "main.tex".into(),
                        expected_disk_sha256: source_hash.clone(),
                        buffer: buffer_text.as_ref().zip(buffer_revision).map(|(text, revision)| {
                            WritingContextBuffer { revision, text: text.clone() }
                        }),
                        selection,
                    }],
                },
                retrieval: None,
                session_config: None,
                turn_config: None,
                model: Some(crate::agent::model::AgentModelSelection {
                    provider_id: "openai".into(),
                    model_id: "gpt-5.6-terra".into(),
                    reasoning_effort: None,
                    service_tier: None,
                }),
            },
        )
        .await
        .map_err(|_| AcceptanceError::Setup)?;
    let child_parent_turn_id = child_request.record.turn_id;
    let child_parent_request_id = child_request.record.request_id;
    let child_parent_context_snapshot_id = child_request.record.context_snapshot_id;
    let child_parent_model = child_request.record.input.model.clone();
    let child_parent_profile = crate::agent::execution_profile::execution_profile(
        &child_parent_model,
        crate::agent::execution_profile::AgentExecutionMode::ChatGptDefault,
    )
    .map_err(|_| AcceptanceError::Setup)?;
    let SubmissionAdmission::Started(child_parent_owned) = writing
        .start_confirmed(
            service.clone(),
            child_request,
            WritingExecutionConfirmation {
                receipt: WritingSubmissionReceipt {
                    conversation_id,
                    request_id: child_parent_request_id,
                    manuscript_id,
                },
                context_snapshot_id: child_parent_context_snapshot_id,
                model: child_parent_model,
            },
            child_parent_profile,
        )
        .await
        .map_err(|_| AcceptanceError::Admission)?
    else {
        return Err(AcceptanceError::Admission);
    };
    if child_parent_owned.task.wait().await != TaskCompletion::Completed {
        return Err(AcceptanceError::Task);
    }
    let child_parent_projection = service
        .open(ConversationId::from_uuid(conversation_id))
        .await
        .map_err(|_| AcceptanceError::Projection)?;
    let child_parent_turn = child_parent_projection
        .turn(child_parent_turn_id)
        .ok_or(AcceptanceError::ChildTurnMissing)?;
    let child_response = child_parent_turn.items.values().find_map(|item| {
        if !matches!(
            &item.kind,
            ItemKind::ToolResult {
                is_error: Some(false),
                ..
            }
        ) {
            return None;
        }
        serde_json::from_str::<Value>(&item.text).ok()
    });
    let child_conversation_id = child_response
        .as_ref()
        .and_then(|value| value.get("childConversationId"))
        .and_then(Value::as_str)
        .and_then(|value| Uuid::parse_str(value).ok())
        .ok_or(AcceptanceError::ChildToolResultMissing)?;
    let child_turn_id = child_response
        .as_ref()
        .and_then(|value| value.get("turnId"))
        .and_then(Value::as_str)
        .and_then(|value| Uuid::parse_str(value).ok())
        .ok_or(AcceptanceError::ChildTurnMissing)?;
    let mut child_turn = None;
    for _ in 0..120 {
        let projection = service
            .open(ConversationId::from_uuid(child_conversation_id))
            .await
            .map_err(|_| AcceptanceError::Projection)?;
        if let Some(turn) = projection.turn(child_turn_id).cloned()
            && turn.outcome.is_some()
        {
            child_turn = Some(turn);
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    let child_turn = child_turn.ok_or(AcceptanceError::ChildTurnTimeout)?;
    if child_turn.outcome != Some(TurnOutcome::Completed) {
        return Err(AcceptanceError::ChildOutcome);
    }
    if !child_turn
        .items
        .values()
        .any(|item| matches!(item.kind, ItemKind::AssistantMessage) && !item.text.trim().is_empty())
    {
        return Err(AcceptanceError::ChildAnswerMissing);
    }
    let source_path = app_store
        .manuscript_directory()
        .join(manuscript_id.to_string())
        .join("source/main.tex");
    let disk_after = std::fs::read(source_path).map_err(|_| AcceptanceError::Setup)?;
    if format!("{:x}", Sha256::digest(disk_after)) != source_hash {
        return Err(AcceptanceError::Application);
    }
    Ok(())
}

fn seed_writing_manuscript(store: &AppStore) -> (Uuid, Uuid, String, String, String) {
    let conversation_id = Uuid::new_v4();
    let manuscript_id = Uuid::new_v4();
    let verification_code = Uuid::new_v4().simple().to_string();
    let source = store
        .manuscript_directory()
        .join(manuscript_id.to_string())
        .join("source");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::write(
        source.join("main.tex"),
        format!("Frozen manuscript business source\nverification code: {verification_code}"),
    )
    .unwrap();
    let hash = format!(
        "{:x}",
        Sha256::digest(
            format!("Frozen manuscript business source\nverification code: {verification_code}")
                .as_bytes(),
        )
    );
    let mut db = store.connection().unwrap();
    crate::agent::repository::create_conversation(
        &mut db,
        ConversationId::from_uuid(conversation_id),
        "Writing",
    )
    .unwrap();
    db.execute(
        "insert into research_projects(id,title,summary,created_at) values (?1,'smoke','','now')",
        [conversation_id.to_string()],
    )
    .unwrap();
    db.execute(
        "insert into manuscripts(id,project_id,title,root_path,created_at,updated_at) values (?1,?2,'smoke','main.tex','now','now')",
        rusqlite::params![manuscript_id.to_string(), conversation_id.to_string()],
    )
    .unwrap();
    db.execute(
        "insert into agent_conversation_bindings(conversation_id,resource_kind,resource_id) values (?1,'manuscript',?2)",
        rusqlite::params![conversation_id.to_string(), manuscript_id.to_string()],
    )
    .unwrap();
    db.execute(
        "insert into manuscript_files(manuscript_id,relative_path,file_kind,byte_size,sha256,modified_at) values (?1,'main.tex','tex',?2,?3,'now')",
        rusqlite::params![manuscript_id.to_string(), std::fs::metadata(source.join("main.tex")).unwrap().len(), hash],
    )
    .unwrap();
    db.execute(
        "insert into llm_provider_configs(provider_id,enabled,base_url,timeout_ms,updated_at) values ('openai',1,'',60000,'now') on conflict(provider_id) do update set enabled=1,base_url='',timeout_ms=60000",
        [],
    )
    .unwrap();
    db.execute(
        "insert into llm_model_routes(route_kind,route_id,model_kind,provider_id,model_id,updated_at) values ('use_case','writing_agent','text','openai','gpt-5.6-terra','now') on conflict(route_kind,route_id) do update set provider_id='openai',model_id='gpt-5.6-terra'",
        [],
    )
    .unwrap();
    let source_id = format!(
        "writing:{:x}",
        Sha256::digest(format!("{manuscript_id}/main.tex").as_bytes())
    );
    (
        conversation_id,
        manuscript_id,
        source_id,
        hash,
        verification_code,
    )
}
