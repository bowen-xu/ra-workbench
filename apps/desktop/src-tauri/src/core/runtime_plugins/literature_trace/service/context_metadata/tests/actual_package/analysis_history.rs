//! Real Core schema and independent Native package, with synthetic historical rows.
use super::*;
use crate::runtime_plugins::literature_analysis::{
    capture, commands, manager::PaperAnalysisWorkflowManager, resources::HistoryResources,
};
use research_workbench_native_sdk::analysis_history::{
    HistoryInputV1, HistoryRequestV1, contracts::PaperAnalysisWorkflow,
};
use rusqlite::params;

pub(super) fn seed(f: &Fixture) {
    let db = f.store.connection().unwrap();
    let provider: String = db
        .query_row(
            "select provider_id from ai_provider_settings order by provider_id limit 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    db.execute("insert into paper_context_snapshots(id,paper_id,context_hash,context_schema_version,created_at) values('history-snapshot','paper',?1,1,'old')", ["e".repeat(64)]).unwrap();
    for (id, created, status) in [
        ("history-old", "old", "failed"),
        ("history-a", "same", "overview_ready"),
        ("history-b", "same", "partial"),
        ("history-superseded", "zz", "superseded"),
    ] {
        db.execute("insert into paper_analysis_workflows(id,paper_id,context_snapshot_id,status,provider_id,model_id,prompt_set_version,revision,created_at,updated_at) values(?1,'paper','history-snapshot',?2,?3,'historical-model','v1',3,?4,'now')", params![id,status,provider,created]).unwrap();
    }
    for (id, ordinal, kind) in [
        ("history-turn-1", 1, "paper_concept_map"),
        ("history-turn-0", 0, "paper_overview"),
    ] {
        db.execute("insert into paper_analysis_turns(id,workflow_id,ordinal,turn_kind,status,created_at,finished_at) values(?1,'history-b',?2,?3,'interrupted','old','now')",params![id,ordinal,kind]).unwrap();
    }
    for (id, attempt, status) in [
        ("history-attempt-2", 2, "interrupted"),
        ("history-attempt-1", 1, "failed"),
    ] {
        db.execute("insert into paper_analysis_turn_attempts(id,turn_id,attempt,status,normalized_messages_json,provider_id,model_id,error_code,error_message,created_at,finished_at) values(?1,'history-turn-0',?2,?3,?4,?5,'historical-model','old_failure','Safe historical cause','old','now')", params![id,attempt,status,json!([{"kind":"task_prompt","content":"Historical Ω\nsource"}]).to_string(),provider]).unwrap();
    }
}
fn latest() -> HistoryRequestV1 {
    HistoryRequestV1::Latest {
        paper_id: "paper".into(),
    }
}
fn expected_latest(
    f: &Fixture,
) -> Result<Option<PaperAnalysisWorkflow>, crate::ai::error::AiError> {
    let catalog = &f.service.catalog;
    let package =
        crate::runtime_plugins::native::package::accept_analysis_history(catalog, &f.key).unwrap();
    let resources = HistoryResources::load(catalog, &package).unwrap();
    let request = latest();
    let workflow = capture::capture(
        &f.store.connection().unwrap(),
        &request,
        &resources.statements(),
    )?;
    Ok(protocol::history(HistoryInputV1 { request, workflow })
        .expect("latest history projects an optional captured workflow"))
}
async fn read(
    f: &Fixture,
    manager: &PaperAnalysisWorkflowManager,
    request: HistoryRequestV1,
) -> Result<Option<PaperAnalysisWorkflow>, Value> {
    commands::read_selected(f.store.clone(), manager.clone(), request).await
}

#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_analysis_history_restores_latest_and_exact_request_owner() {
    let (catalog, key) = actual_package();
    let mut f = Fixture::new();
    f.bind_actual_package(catalog, key);
    let manager = PaperAnalysisWorkflowManager::default();
    assert!(f.store.require_paper_analysis_source().is_err());
    assert!(read(&f, &manager, latest()).await.unwrap().is_none());
    seed(&f);
    let expected = expected_latest(&f).unwrap().unwrap();
    let actual = read(&f, &manager, latest()).await.unwrap().unwrap();
    assert_eq!(
        serde_json::to_value(&actual).unwrap(),
        serde_json::to_value(&expected).unwrap()
    );
    assert_eq!(actual.id, "history-b");
    assert_eq!(actual.turns[0].id, "history-turn-0");
    assert_eq!(actual.turns[0].attempts[0].id, "history-attempt-1");
    assert_eq!(
        actual.turns[0].attempts[1].normalized_messages[0].content,
        "Historical Ω\nsource"
    );
    let lease = manager
        .register_for_paper("history-client-request", "paper")
        .unwrap();
    let active = read(&f, &manager, latest()).await.unwrap().unwrap();
    assert_eq!(
        active.active_client_request_id.as_deref(),
        Some("history-client-request")
    );
    assert!(manager.cancel("history-client-request").unwrap());
    assert!(lease.signal().is_cancelled());
    assert_eq!(
        read(&f, &manager, latest())
            .await
            .unwrap()
            .unwrap()
            .active_client_request_id,
        active.active_client_request_id
    );
    drop(lease);
    assert!(
        read(&f, &manager, latest())
            .await
            .unwrap()
            .unwrap()
            .active_client_request_id
            .is_none()
    );
    let by_id = read(
        &f,
        &manager,
        HistoryRequestV1::Get {
            workflow_id: "history-old".into(),
        },
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(by_id.id, "history-old");
    assert!(by_id.turns.is_empty());
    f.service.shutdown().await.unwrap();
}

#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_analysis_history_missing_and_json_errors_keep_original_precedence() {
    let (catalog, key) = actual_package();
    let mut f = Fixture::new();
    f.bind_actual_package(catalog, key);
    let manager = PaperAnalysisWorkflowManager::default();
    let missing = read(
        &f,
        &manager,
        HistoryRequestV1::Get {
            workflow_id: "missing".into(),
        },
    )
    .await
    .unwrap_err();
    assert_eq!(missing["code"], "invalid_input");
    assert_eq!(missing["message"], "找不到论文分析工作流");
    assert_eq!(missing["workflowId"], "missing");
    assert_eq!(missing["nativeFailure"]["kind"], "pureRejected");
    assert!(missing["nativeFailure"]["instanceId"].as_str().is_some());
    assert!(
        missing["nativeFailure"]["nativeRequestId"]
            .as_str()
            .is_some()
    );
    f.service.shutdown().await.unwrap();
    let (catalog, key) = actual_package();
    f.bind_actual_package(catalog, key);
    seed(&f);
    f.store.connection().unwrap().execute("update paper_analysis_turn_attempts set normalized_messages_json='[{\"unknown\":1}]' where id='history-attempt-1'",[]).unwrap();
    let expected = crate::ai::error::AiCommandError::from(expected_latest(&f).unwrap_err());
    let failure = read(&f, &manager, latest()).await.unwrap_err();
    assert_eq!(failure["code"], expected.code());
    assert_eq!(failure["message"], expected.message());
    assert_eq!(failure["nativeFailure"]["phase"], "source-read");
    assert_eq!(failure["nativeFailure"]["outcome"], "notStarted");
    assert!(
        f.service.host.lock().await.is_none(),
        "original decode fails before Native starts"
    );
    f.service.shutdown().await.unwrap();
}

#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_analysis_history_survives_original_core_close_and_reopen() {
    let (catalog, key) = actual_package();
    let mut f = Fixture::new();
    f.bind_actual_package(catalog, key);
    seed(&f);
    let manager = PaperAnalysisWorkflowManager::default();
    let expected = read(&f, &manager, latest()).await.unwrap().unwrap();
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
    let actual = read(&f, &PaperAnalysisWorkflowManager::default(), latest())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        serde_json::to_value(actual).unwrap(),
        serde_json::to_value(expected).unwrap()
    );
    f.service.shutdown().await.unwrap();
    let failed = read(&f, &manager, latest()).await.unwrap_err();
    assert_eq!(failed["code"], "plugin_host_stopping");
    assert_eq!(failed["paperId"], "paper");
}

#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_analysis_history_preserves_recovery_and_agent_terminal_ownership() {
    use research_agent_core::{
        conversation::ConversationId,
        protocol::{AgentEvent, EventPayload, ModelIdentity, TaskKind, TurnOutcome, TurnPhase},
    };
    let (catalog, key) = actual_package();
    let mut f = Fixture::new();
    f.bind_actual_package(catalog, key);
    seed(&f);
    let conversation = ConversationId::from_uuid(uuid::Uuid::new_v4());
    let request = uuid::Uuid::new_v4();
    let turn = uuid::Uuid::new_v4();
    {
        let db = f.store.connection().unwrap();
        db.execute(
            "insert into agent_conversations(id,title) values(?1,'Historical analysis')",
            [conversation.as_uuid().to_string()],
        )
        .unwrap();
        db.execute("insert into agent_paper_analysis_bindings(workflow_id,conversation_id,paper_id,context_snapshot_id,created_at) values('history-b',?1,'paper','history-snapshot','old')",[conversation.as_uuid().to_string()]).unwrap();
        db.execute("insert into agent_paper_analysis_submissions(attempt_id,conversation_id,request_id,turn_id,context_snapshot_id,paper_id,created_at,terminal_status,updated_at) values('history-attempt-2',?1,?2,?3,'history-snapshot','paper','old','interrupted','now')",params![conversation.as_uuid().to_string(),request.to_string(),turn.to_string()]).unwrap();
        for (index, payload) in [
            EventPayload::TurnStarted {
                task_kind: TaskKind::Regular,
                turn_id: turn,
                context_snapshot_id: uuid::Uuid::new_v4(),
                model: ModelIdentity {
                    provider_id: "fixture".into(),
                    model_id: "historical".into(),
                    revision: None,
                },
            },
            EventPayload::PhaseChanged {
                turn_id: turn,
                phase: TurnPhase::RunningModel,
            },
            EventPayload::TurnFinished {
                turn_id: turn,
                outcome: TurnOutcome::Completed,
            },
        ]
        .into_iter()
        .enumerate()
        {
            let event =
                AgentEvent::new(conversation, (index + 1) as u64, 1, request, payload).unwrap();
            db.execute("insert into agent_items(conversation_id,ordinal,schema_version,payload_json) values(?1,?2,1,?3)",params![conversation.as_uuid().to_string(),index as i64,serde_json::to_string(&event).unwrap()]).unwrap();
        }
        let provider: String = db
            .query_row(
                "select provider_id from paper_analysis_workflows where id='history-b'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        db.execute("insert into ai_runs(id,task_kind,status,provider_id,model_id,pipeline_version,prompt_version,output_schema_version,scope_hash,max_input_tokens,max_output_tokens,timeout_ms,created_at,updated_at) values('history-run','paper_overview','needs_review',?1,'historical','v1','v1',1,?2,1000,1000,1000,'old','now')",params![provider,"e".repeat(64)]).unwrap();
        db.execute("insert into ai_evidence_bundles(id,run_id,scope_hash,retrieval_config_version,validation_status,created_at) values('history-bundle','history-run',?1,'v1','valid','old')",["e".repeat(64)]).unwrap();
        db.execute("insert into ai_candidates(id,run_id,evidence_bundle_id,candidate_kind,schema_version,validation_status,original_payload_hash,created_at,updated_at) values('history-candidate','history-run','history-bundle','paper_overview_unit_v1',1,'valid',?1,'old','now')",["e".repeat(64)]).unwrap();
        db.execute_batch("insert into ai_candidate_recovery_audits(id,candidate_id,run_id,workflow_id,recovery_kind,recovery_reason,recovery_version,original_run_status,recovered_at) values('history-recovery','history-candidate','history-run','history-b','paper_overview','original-repair',2,'interrupted','now'); update paper_analysis_turn_attempts set candidate_id='history-candidate',ai_run_id='history-run' where id='history-attempt-2';").unwrap();
    }
    let actual = read(&f, &PaperAnalysisWorkflowManager::default(), latest())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(actual.conversation_id, Some(*conversation.as_uuid()));
    let attempt = &actual.turns[0].attempts[1];
    assert_eq!(attempt.shared_turn_outcome.as_deref(), Some("completed"));
    assert_eq!(
        attempt.shared_terminal_status.as_deref(),
        Some("interrupted")
    );
    assert_eq!(attempt.status, "interrupted");
    assert_eq!(attempt.shared_request_id, Some(request));
    assert_eq!(attempt.shared_turn_id, Some(turn));
    assert_eq!(attempt.recovery_reason.as_deref(), Some("original-repair"));
    assert_eq!(attempt.recovery_version, Some(2));
    assert_eq!(attempt.recovered_at.as_deref(), Some("now"));
    f.service.shutdown().await.unwrap();
}
