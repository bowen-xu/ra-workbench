//! Original attempt qualification through the independent package, before SQL admission.
use super::*;
use crate::ai::paper_analysis_contracts::{PaperAnalysisMessage, PaperAnalysisMessageKind};
use research_workbench_native_sdk::analysis_attempt::*;

#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_attempt_qualification_preserves_input_order_plan_binding_and_error_category()
 {
    let (catalog, key) = actual_package();
    let mut f = Fixture::new();
    f.bind_actual_package(catalog, key.clone());
    let before = f.store.connection().unwrap().total_changes();
    let invalid = f
        .service
        .validate_analysis_attempt_input(&key, "invalid", &[])
        .await
        .unwrap_err();
    assert_eq!(invalid.code(), "invalid_input");
    assert!(invalid.to_string().contains("未知论文分析阶段"));
    let crate::ai::error::AiError::NativeWorkflow { failure, .. } = invalid else {
        panic!("actual Native diagnostic");
    };
    assert_eq!(failure.package_key.as_deref(), Some(key.as_str()));
    let first_instance = failure.instance_id.unwrap();
    assert!(failure.request_id.is_some());
    let messages = vec![PaperAnalysisMessage::new(
        PaperAnalysisMessageKind::TaskPrompt,
        "bounded message Ω",
    )];
    let origin = f
        .service
        .validate_analysis_attempt_input(&key, "paper_overview", &messages)
        .await
        .unwrap();
    assert_eq!(origin.instance_id, first_instance);
    let mut workflow = AnalysisAttemptWorkflowObservation {
        status: "preparing".into(),
        turns: vec![
            AnalysisAttemptTurnObservation {
                id: "original-turn".into(),
                kind: "paper_overview".into(),
                status: "failed".into(),
                attempt_numbers: vec![9, 2],
            },
            AnalysisAttemptTurnObservation {
                id: "later-turn".into(),
                kind: "paper_overview".into(),
                status: "failed".into(),
                attempt_numbers: vec![100],
            },
        ],
    };
    let (plan, origin) = f
        .service
        .plan_analysis_attempt(&key, "paper_overview", workflow.clone())
        .await
        .unwrap();
    assert_eq!(
        plan,
        AnalysisAttemptPlan {
            turn_id: "original-turn".into(),
            next_attempt: 3
        }
    );
    assert_eq!(origin.instance_id, first_instance);
    assert!(!origin.native_request_id.is_empty());
    workflow.turns.clear();
    let missing = f
        .service
        .plan_analysis_attempt(&key, "paper_overview", workflow)
        .await
        .unwrap_err();
    assert_eq!(missing.code(), "temporary_unavailable");
    assert!(missing.to_string().contains("provider 暂时不可用"));
    assert_eq!(f.store.connection().unwrap().total_changes(), before);
    f.service.shutdown().await.unwrap();
}

#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_attempt_admission_keeps_shared_input_rollback_retry_and_qualification() {
    use crate::{
        agent::repository::submissions::SqliteSubmissionStore,
        ai::paper_reading_contracts::{PaperAiReadingImageAssetDraft, PrepareRequest},
        runtime_plugins::literature_trace::service::StartAnalysisAttempt,
    };
    use research_agent_core::submission::SubmissionStore;
    use research_workbench_native_sdk::analysis_workflow::{PaperAnalysisWorkflowDraft, wire::*};
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
    let (catalog, key) = actual_package();
    let mut f = Fixture::new();
    let (_credentials, input) = analysis_overview::seeded_task(&f);
    let image_id = crate::runtime_plugins::literature_trace::service::context_metadata::tests::actual_package::prepared_context(
        &f.store.connection().unwrap(),
        "paper",
        &["selected".into()],
    )
    .unwrap()
    .image_requests[0]
        .context_item_id
        .clone();
    crate::agent::skills::bundled::install_defaults(f.store.agent_configuration_directory())
        .unwrap();
    f.bind_actual_package(catalog, key.clone());
    let (_, snapshot) = f
        .service
        .prepare_analysis_context(
            f.store.clone(),
            key.clone(),
            PrepareRequest {
                paper_id: "paper".into(),
                attachment_ids: vec!["selected".into()],
                image_assets: vec![PaperAiReadingImageAssetDraft {
                    context_item_id: image_id,
                    data_url: "data:image/png;base64,iVBORw0KGgpzeW50aGV0aWM=".into(),
                }],
            },
        )
        .await
        .unwrap();
    let profile = f
        .service
        .resolve_analysis_template(
            f.store.clone(),
            &key,
            "paper",
            input.template.as_ref().unwrap(),
        )
        .await
        .unwrap();
    let selection = input.model_selection.unwrap();
    let workflow = f
        .service
        .create_analysis_workflow(
            f.store.clone(),
            key.clone(),
            PaperAnalysisWorkflowDraft {
                paper_id: "paper".into(),
                context_snapshot_id: snapshot.snapshot_id,
                provider_id: selection.provider_id,
                model_id: selection.model_id,
                reasoning_effort: selection.reasoning_effort,
                prompt_set_version: "paper-analysis-tools-v3".into(),
            },
        )
        .await
        .unwrap();
    let (decision, _) = f
        .service
        .analysis_workflow_decision(
            &key,
            WorkflowInputV1::OverviewMessages {
                workflow: Box::new(workflow.clone()),
            },
        )
        .await
        .unwrap();
    let WorkflowDecisionV1::Messages { mut messages } = decision else {
        panic!("overview messages");
    };
    messages.push(PaperAnalysisMessage::new(
        PaperAnalysisMessageKind::TaskPrompt,
        serde_json::to_string(&profile).unwrap(),
    ));
    let request = || StartAnalysisAttempt {
        workflow_id: workflow.id.clone(),
        turn_kind: "paper_overview".into(),
        messages: messages.clone(),
        resources: (vec![], vec![], vec![]),
    };
    f.store.connection().unwrap().execute_batch("create temp trigger reject_attempt_turn before update on paper_analysis_turns begin select raise(abort,'private attempt body must remain private'); end;").unwrap();
    let error = f
        .service
        .start_analysis_attempt(f.store.clone(), key.clone(), request())
        .await
        .unwrap_err();
    assert_eq!(error.code(), "storage_error");
    assert!(!error.message().contains("private attempt body"));
    let wire = serde_json::to_value(error).unwrap();
    assert_eq!(wire["analysisWorkflowOperation"], "startAttempt");
    assert_eq!(wire["analysisWorkflowOutcome"], "rolledBack");
    assert_eq!(wire["workflowId"], workflow.id);
    assert!(wire["attemptId"].is_string());
    assert_eq!(wire["nativeFailure"]["packageKey"], key);
    let conversation = workflow.conversation_id.unwrap();
    let counts = || {
        f.store.connection().unwrap().query_row(
        "select (select count(*) from paper_analysis_turn_attempts where turn_id=?1),(select count(*) from agent_submissions where conversation_id=?2),(select count(*) from agent_paper_analysis_submissions where conversation_id=?2)",
        rusqlite::params![workflow.turns[0].id,conversation.to_string()],|row|Ok((row.get::<_,i64>(0)?,row.get::<_,i64>(1)?,row.get::<_,i64>(2)?)),
    ).unwrap()
    };
    assert_eq!(counts(), (0, 0, 0));
    assert!(f.store.connection().unwrap().is_autocommit());
    f.store
        .connection()
        .unwrap()
        .execute_batch("drop trigger reject_attempt_turn")
        .unwrap();
    let started = f
        .service
        .start_analysis_attempt(f.store.clone(), key.clone(), request())
        .await
        .unwrap();
    assert_eq!(started.status, "overview_running");
    assert_eq!(started.revision, workflow.revision + 1);
    assert_eq!(counts(), (1, 1, 1));
    let attempt = &started.turns[0].attempts[0];
    assert_eq!(attempt.attempt, 1);
    assert_eq!(attempt.status, "running");
    assert_eq!(
        serde_json::to_value(&attempt.normalized_messages).unwrap(),
        serde_json::to_value(&messages).unwrap()
    );
    let submission = SqliteSubmissionStore::new(f.store.shared_agent_connection())
        .get(conversation, attempt.shared_request_id.unwrap())
        .await
        .unwrap();
    assert_eq!(submission.request_id.to_string(), attempt.id);
    assert_eq!(submission.input.model.provider_id, workflow.provider_id);
    assert_eq!(submission.input.model.model_id, workflow.model_id);
    assert_eq!(submission.input.model.revision, None);
    assert!(!submission.input.text.is_empty());
    assert!(!submission.input.skills.is_empty());
    assert!(!submission.input.skill_instructions.is_empty());
    assert!(crate::agent::skills::application::validate(&submission.input).is_ok());
    let target = crate::structured_resources::find_by_paper(
        &f.store.connection().unwrap(),
        "paper",
        "structuredReading",
    )
    .unwrap();
    crate::ai::structured_generation_support::require_writable_target(
        &submission.input.context,
        &target.id,
    )
    .unwrap();
    f.store
        .connection()
        .unwrap()
        .authorizer(Some(|context: AuthContext<'_>| {
            if matches!(
                context.action,
                AuthAction::Read {
                    table_name: "paper_context_snapshots"
                        | "paper_context_items"
                        | "structured_resources",
                    ..
                }
            ) {
                Authorization::Deny
            } else {
                Authorization::Allow
            }
        }));
    let rejected = f
        .service
        .start_analysis_attempt(f.store.clone(), key, request())
        .await
        .unwrap_err();
    f.store
        .connection()
        .unwrap()
        .authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
    assert_eq!(rejected.code(), "invalid_input");
    assert!(
        rejected
            .message()
            .contains("该论文分析阶段正在运行或已经成功")
    );
    assert_eq!(counts(), (1, 1, 1));
    f.service.shutdown().await.unwrap();
}

#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_attempt_unknown_settlement_fences_and_reopens_without_new_attempt() {
    use crate::runtime_plugins::literature_trace::service::StartAnalysisAttempt;
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization, TransactionOperation};
    for fail_commit in [true, false] {
        let (catalog, key) = actual_package();
        let mut f = Fixture::new();
        analysis_history::seed(&f);
        f.bind_actual_package(catalog, key.clone());
        if fail_commit {
            f.store.connection().unwrap().commit_hook(Some(|| true));
        } else {
            f.store.connection().unwrap().execute_batch("create temp trigger reject_attempt_turn before update on paper_analysis_turns begin select raise(abort,'synthetic attempt failure'); end;").unwrap();
            f.store
                .connection()
                .unwrap()
                .authorizer(Some(|context: AuthContext<'_>| {
                    if matches!(
                        context.action,
                        AuthAction::Transaction {
                            operation: TransactionOperation::Rollback
                        }
                    ) {
                        Authorization::Deny
                    } else {
                        Authorization::Allow
                    }
                }));
        }
        let failure = f
            .service
            .start_analysis_attempt(
                f.store.clone(),
                key.clone(),
                StartAnalysisAttempt {
                    workflow_id: "history-b".into(),
                    turn_kind: "paper_overview".into(),
                    messages: vec![PaperAnalysisMessage::new(
                        PaperAnalysisMessageKind::TaskPrompt,
                        "Owned attempt",
                    )],
                    resources: (vec![], vec![], vec![]),
                },
            )
            .await
            .unwrap_err();
        assert_eq!(failure.code(), "data_operation_outcome_unknown");
        let wire = json!(failure);
        assert_eq!(wire["workflowId"], "history-b");
        assert!(wire["attemptId"].is_string());
        assert_eq!(wire["analysisWorkflowOperation"], "startAttempt");
        assert_eq!(wire["analysisWorkflowOutcome"], "unknown");
        assert_eq!(wire["nativeFailure"]["packageKey"], key);
        assert_eq!(
            wire["analysisWorkflowPhase"],
            if fail_commit { "commit" } else { "write" }
        );
        assert!(f.store.connection().is_err());
        let f = reopen_actual_fixture(f, true).await;
        let count: i64 = f
            .store
            .connection()
            .unwrap()
            .query_row(
                "select count(*) from paper_analysis_turn_attempts where turn_id='history-turn-0'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 2);
        f.service.shutdown().await.unwrap();
    }
}
