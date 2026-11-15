//! The real Core transport executes the package's original preparation rules.
use super::*;
use crate::runtime_plugins::literature_trace::service::AttemptFailure;
use research_workbench_native_sdk::analysis_workflow::{AnalysisWorkflowError, wire::*};

fn seed_running(f: &Fixture, kind: &str) {
    super::analysis_history::seed(f);
    let db = f.store.connection().unwrap();
    let turn = if kind == "paper_overview" {
        "history-turn-0"
    } else {
        "history-turn-1"
    };
    let status = if kind == "paper_overview" {
        "overview_running"
    } else {
        "concepts_running"
    };
    db.execute(
        "update paper_analysis_workflows set status=?1,active_turn_kind=?2 where id='history-b'",
        rusqlite::params![status, kind],
    )
    .unwrap();
    db.execute(
        "update paper_analysis_turns set status='running' where id=?1",
        [turn],
    )
    .unwrap();
    db.execute("update paper_analysis_turn_attempts set status='running',finished_at=null,turn_id=?1 where id='history-attempt-2'",[turn]).unwrap();
    db.execute("insert into agent_paper_analysis_submissions(attempt_id,conversation_id,request_id,turn_id,context_snapshot_id,paper_id,created_at) values('history-attempt-2',?1,?2,?3,'history-snapshot','paper','old')",rusqlite::params![f.record.conversation_id.to_string(),f.record.request_id.to_string(),f.record.turn_id.to_string()]).unwrap();
}
async fn fail(
    f: &Fixture,
    kind: &str,
    code: &str,
) -> Result<research_workbench_native_sdk::analysis_history::contracts::PaperAnalysisWorkflow, Value>
{
    f.service
        .fail_analysis_attempt(
            f.store.clone(),
            f.key.clone(),
            AttemptFailure {
                workflow_id: "history-b".into(),
                turn_kind: kind.into(),
                error_code: code.into(),
                error_message: "Safe synthetic provider failure Ω".into(),
            },
        )
        .await
}

#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_workflow_rules_use_the_selected_host_and_preserve_rejections() {
    let (catalog, key) = actual_package();
    let mut f = Fixture::new();
    f.bind_actual_package(catalog, key.clone());
    assert!(f.store.paper_analysis_source().is_none());
    assert!(f.store.paper_reading_source().is_none());
    assert!(f.store.generation_policy_source().is_none());
    let (invalid, first) = f
        .service
        .analysis_workflow_decision(
            &key,
            WorkflowInputV1::ValidateStart {
                revision: 0,
                has_template: false,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        invalid,
        WorkflowDecisionV1::Validated {
            rejection: Some(AnalysisWorkflowError::InvalidInput(
                "阅读框架尚未确认，无法启动论文分析".into()
            ))
        }
    );
    let (valid, next) = f
        .service
        .analysis_workflow_decision(
            &key,
            WorkflowInputV1::ValidateSources {
                statuses: vec!["available".into()],
                picture_count: 128,
            },
        )
        .await
        .unwrap();
    assert_eq!(valid, WorkflowDecisionV1::Validated { rejection: None });
    assert_eq!(first.package_key, key);
    assert_eq!(first.instance_id, next.instance_id);
    assert_ne!(first.native_request_id, next.native_request_id);
    let (statuses, _) = f
        .service
        .analysis_workflow_decision(
            &key,
            WorkflowInputV1::FailureStatuses {
                turn_kind: "paper_concept_map".into(),
                error_code: "interrupted".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(
        statuses,
        WorkflowDecisionV1::Failure {
            statuses: research_workbench_native_sdk::analysis_workflow::FailureStatuses {
                attempt: "interrupted".into(),
                turn: "interrupted".into(),
                workflow: "partial".into()
            }
        }
    );
    let (packed, _) = f
        .service
        .analysis_workflow_decision(&key, WorkflowInputV1::PackSources { sources: vec![] })
        .await
        .unwrap();
    assert_eq!(packed, WorkflowDecisionV1::Sources { result: Ok(vec![]) });
    f.service.shutdown().await.unwrap();
    assert!(f.service.host.lock().await.is_none());
}

#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_workflow_failure_keeps_shared_terminal_and_all_original_status_branches() {
    for kind in ["paper_overview", "paper_concept_map"] {
        for code in ["canceled", "interrupted", "provider_failed"] {
            let (catalog, key) = actual_package();
            let mut f = Fixture::new();
            f.bind_actual_package(catalog, key);
            seed_running(&f, kind);
            let value = fail(&f, kind, code).await.unwrap();
            let terminal = if code == "canceled" {
                "canceled"
            } else if code == "interrupted" {
                "interrupted"
            } else {
                "failed"
            };
            assert_eq!(
                value.status,
                if code == "canceled" {
                    "canceled"
                } else if kind == "paper_overview" {
                    "failed"
                } else {
                    "partial"
                }
            );
            assert_eq!(value.revision, 4);
            assert!(value.active_turn_kind.is_none());
            let turn = value.turns.iter().find(|turn| turn.kind == kind).unwrap();
            assert_eq!(turn.status, terminal);
            let attempt = turn
                .attempts
                .iter()
                .find(|attempt| attempt.id == "history-attempt-2")
                .unwrap();
            assert_eq!(attempt.status, terminal);
            assert_eq!(attempt.error_code.as_deref(), Some(code));
            assert_eq!(
                attempt.error_message.as_deref(),
                Some("Safe synthetic provider failure Ω")
            );
            assert_eq!(attempt.shared_terminal_status.as_deref(), Some(terminal));
            assert!(attempt.finished_at.is_some());
            assert!(value.completed_at.is_some());
            let duplicate = fail(&f, kind, code).await.unwrap_err();
            assert_eq!(duplicate["code"], "invalid_input");
            assert_eq!(duplicate["message"], "该论文分析阶段没有运行中的尝试");
            assert_eq!(duplicate["analysisWorkflowOutcome"], "notStarted");
            f.service.shutdown().await.unwrap();
        }
    }
}

#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_workflow_failure_rolls_back_original_writes_and_reopens_committed_history()
{
    let (catalog, key) = actual_package();
    let mut f = Fixture::new();
    f.bind_actual_package(catalog, key);
    seed_running(&f, "paper_overview");
    f.store.connection().unwrap().execute_batch("create temp trigger test_analysis_failure_abort before update of status on paper_analysis_workflows begin select raise(abort,'synthetic failure write'); end;").unwrap();
    let failure = fail(&f, "paper_overview", "canceled").await.unwrap_err();
    assert_eq!(failure["analysisWorkflowPhase"], "write");
    assert_eq!(failure["analysisWorkflowOutcome"], "rolledBack");
    assert_eq!(failure["workflowId"], "history-b");
    {
        let db = f.store.connection().unwrap();
        let values:(String,String,Option<String>)=db.query_row("select a.status,t.status,s.terminal_status from paper_analysis_turn_attempts a join paper_analysis_turns t on t.id=a.turn_id join agent_paper_analysis_submissions s on s.attempt_id=a.id where a.id='history-attempt-2'",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
        assert_eq!(values, ("running".into(), "running".into(), None));
        assert!(db.is_autocommit());
        db.execute_batch("drop trigger test_analysis_failure_abort;")
            .unwrap();
    }
    let saved = fail(&f, "paper_overview", "canceled").await.unwrap();
    f.service.shutdown().await.unwrap();
    f.store.shared_agent_connection().shutdown().unwrap();
    drop(f.store);
    f.store = crate::storage::AppStore::open_with_paths(
        crate::storage::AppStorePaths {
            data_directory: f._data.path().into(),
            agent_configuration_directory: f._data.path().join("configuration"),
        },
        crate::storage::AppStorePorts::with_events(
            crate::runtime_events::RuntimeEvents::new(|_, _| Ok(())),
            crate::storage::AppStoreSources::default(),
        ),
    )
    .unwrap();
    let (catalog, key) = actual_package();
    f.bind_actual_package(catalog, key);
    let restored = f
        .service
        .read_analysis_history(
            f.store.clone(),
            f.key.clone(),
            research_workbench_native_sdk::analysis_history::HistoryRequestV1::Get {
                workflow_id: "history-b".into(),
            },
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        serde_json::to_value(restored).unwrap(),
        serde_json::to_value(saved).unwrap()
    );
    f.service.shutdown().await.unwrap();
}

#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_workflow_turn_validation_precedes_the_original_storage_read() {
    let (catalog, key) = actual_package();
    let mut f = Fixture::new();
    f.bind_actual_package(catalog, key);
    f.store
        .connection()
        .unwrap()
        .execute_batch(
            "create temp view paper_analysis_turns as select 'poisoned shape' as unavailable;",
        )
        .unwrap();
    let invalid = fail(&f, "unknown-stage", "canceled").await.unwrap_err();
    assert_eq!(invalid["code"], "invalid_input");
    assert_eq!(invalid["message"], "未知论文分析阶段");
    assert_eq!(invalid["nativeFailure"]["kind"], "pureRejected");
    let storage = fail(&f, "paper_overview", "canceled").await.unwrap_err();
    assert_eq!(storage["analysisWorkflowPhase"], "source-read");
    assert_eq!(storage["analysisWorkflowOutcome"], "notStarted");
    f.service.shutdown().await.unwrap();
}
