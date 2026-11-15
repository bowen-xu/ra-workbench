//! Legacy two-stage completion keeps the original candidate and transaction semantics.
use super::*;
use crate::{
    ai::paper_analysis_contracts::{PaperAnalysisMessage, PaperAnalysisMessageKind},
    runtime_plugins::literature_trace::service::StartAnalysisAttempt,
};

#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_attempt_completion_keeps_old_concept_stage_rollback_retry_and_candidate() {
    let (catalog, key) = actual_package();
    let mut f = Fixture::new();
    let (credentials, input) = analysis_overview::seeded_task(&f);
    analysis_history::seed(&f);
    f.bind_actual_package(catalog, key.clone());
    let started = f
        .service
        .start_analysis_attempt(
            f.store.clone(),
            key.clone(),
            StartAnalysisAttempt {
                workflow_id: "history-b".into(),
                turn_kind: "paper_overview".into(),
                messages: vec![PaperAnalysisMessage::new(
                    PaperAnalysisMessageKind::TaskPrompt,
                    "Legacy attempt Ω",
                )],
                resources: (vec![], vec![], vec![]),
            },
        )
        .await
        .unwrap();
    assert!(started.conversation_id.is_none());
    let attempt = started.turns[0].attempts.last().unwrap();
    assert_eq!(attempt.attempt, 3);
    let mut task = f
        .service
        .build_analysis_overview_task(&key, input)
        .await
        .unwrap();
    let preview = f
        .service
        .preview_analysis_overview(f.store.clone(), credentials.clone(), &key, task.clone())
        .await
        .unwrap();
    task.confirmed_scope_hash = Some(preview.scope_hash);
    task.cloud_consent_version = Some("ai-cloud-consent-v1".into());
    let run = f
        .service
        .admit_analysis_overview(f.store.clone(), credentials, key.clone(), task, None)
        .await
        .unwrap();
    let missing = f
        .service
        .complete_analysis_overview(
            f.store.clone(),
            key.clone(),
            started.id.clone(),
            run.id.clone(),
        )
        .await
        .unwrap_err();
    assert_eq!(missing.code(), "invalid_input");
    assert!(missing.message().contains("AI 运行没有生成预期候选"));
    let bundle = analysis_candidate::evidence(&f, &key, &run).await;
    let text = analysis_candidate::content(&run, &bundle);
    let candidate = f
        .service
        .save_analysis_candidate(f.store.clone(), key.clone(), run.id.clone(), bundle, text)
        .await
        .unwrap();
    let complete = || {
        f.service.complete_analysis_overview(
            f.store.clone(),
            key.clone(),
            started.id.clone(),
            run.id.clone(),
        )
    };
    f.store.connection().unwrap().execute_batch("create temp trigger reject_complete_turn before update on paper_analysis_turns begin select raise(abort,'private completion body'); end;").unwrap();
    let error = complete().await.unwrap_err();
    assert_eq!(error.code(), "storage_error");
    assert!(!error.message().contains("private completion body"));
    let wire = json!(error);
    assert_eq!(wire["analysisWorkflowOperation"], "completeOverview");
    assert_eq!(wire["analysisWorkflowOutcome"], "rolledBack");
    assert_eq!(wire["workflowId"], started.id);
    assert_eq!(wire["attemptId"], attempt.id);
    assert_eq!(wire["runId"], run.id);
    assert_eq!(wire["nativeFailure"]["packageKey"], key);
    let status: (String, Option<String>) = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "select status,candidate_id from paper_analysis_turn_attempts where id=?1",
            [&attempt.id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(status, ("running".into(), None));
    f.store
        .connection()
        .unwrap()
        .execute_batch("drop trigger reject_complete_turn")
        .unwrap();
    let completed = complete().await.unwrap();
    assert_eq!(completed.status, "overview_ready");
    assert_eq!(completed.revision, started.revision + 1);
    assert_eq!(completed.turns[0].status, "succeeded");
    assert_eq!(
        completed.turns[0]
            .attempts
            .last()
            .unwrap()
            .candidate_id
            .as_deref(),
        Some(candidate.id.as_str())
    );
    assert_eq!(completed.turns[1].status, "interrupted");
    let repeated = complete().await.unwrap_err();
    assert_eq!(repeated.code(), "invalid_input");
    assert!(
        repeated
            .message()
            .contains("该论文分析阶段没有运行中的尝试")
    );
    assert!(f.store.connection().unwrap().is_autocommit());
    f.service.shutdown().await.unwrap();
}
