//! Actual original creation and conversation transactions over the independent package.
use super::*;
use research_workbench_native_sdk::{
    analysis_history::contracts::PaperAnalysisWorkflow,
    analysis_workflow::PaperAnalysisWorkflowDraft,
};

fn seed(f: &Fixture) -> PaperAnalysisWorkflowDraft {
    let db = f.store.connection().unwrap();
    let provider = db
        .query_row(
            "select provider_id from ai_provider_settings order by provider_id limit 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    db.execute("insert into paper_context_snapshots(id,paper_id,context_hash,context_schema_version,created_at) values('create-snapshot','paper',?1,1,'old')",["f".repeat(64)]).unwrap();
    PaperAnalysisWorkflowDraft {
        paper_id: "paper".into(),
        context_snapshot_id: "create-snapshot".into(),
        provider_id: provider,
        model_id: "original-model".into(),
        reasoning_effort: Some("high".into()),
        prompt_set_version: "paper-analysis-tools-v3".into(),
    }
}
async fn create(
    f: &Fixture,
    draft: PaperAnalysisWorkflowDraft,
) -> Result<PaperAnalysisWorkflow, Value> {
    f.service
        .create_analysis_workflow(f.store.clone(), f.key.clone(), draft)
        .await
}
fn counts(f: &Fixture) -> (i64, i64, i64, i64) {
    f.store.connection().unwrap().query_row("select (select count(*) from paper_analysis_workflows),(select count(*) from paper_analysis_turns),(select count(*) from agent_paper_analysis_bindings),(select count(*) from paper_generation_conversations)",[],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?))).unwrap()
}

#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_creates_original_workflow_and_reuses_or_replaces_the_shared_conversation() {
    let (catalog, key) = actual_package();
    let mut f = Fixture::new();
    f.bind_actual_package(catalog, key);
    assert!(f.store.paper_generation_source().is_none());
    assert!(f.store.paper_analysis_source().is_none());
    let draft = seed(&f);
    let first = create(&f, draft.clone()).await.unwrap();
    assert_eq!(first.status, "preparing");
    assert_eq!(first.revision, 1);
    assert_eq!(first.context_snapshot_id, "create-snapshot");
    assert_eq!(first.reasoning_effort.as_deref(), Some("high"));
    assert_eq!(first.turns.len(), 1);
    assert_eq!(first.turns[0].kind, "paper_overview");
    assert_eq!(first.turns[0].status, "queued");
    assert!(first.turns[0].attempts.is_empty());
    let conversation = first.conversation_id.unwrap();
    assert_eq!(counts(&f), (1, 1, 1, 1));
    let active = create(&f, draft.clone()).await.unwrap_err();
    assert_eq!(active["code"], "invalid_input");
    assert_eq!(
        active["message"],
        "本文生成任务仍在运行，请先等待结束或取消。"
    );
    assert_eq!(active["analysisWorkflowOutcome"], "rolledBack");
    assert_eq!(counts(&f), (1, 1, 1, 1));
    f.store
        .connection()
        .unwrap()
        .execute(
            "update paper_analysis_workflows set status='failed' where id=?1",
            [&first.id],
        )
        .unwrap();
    let second = create(&f, draft.clone()).await.unwrap();
    assert_ne!(second.id, first.id);
    assert_eq!(second.conversation_id, Some(conversation));
    assert_eq!(
        second.supersedes_workflow_id.as_deref(),
        Some(first.id.as_str())
    );
    let old: (String, i64) = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "select status,revision from paper_analysis_workflows where id=?1",
            [&first.id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(old, ("superseded".into(), 2));
    {
        let db = f.store.connection().unwrap();
        db.execute(
            "update paper_analysis_workflows set status='failed' where id=?1",
            [&second.id],
        )
        .unwrap();
        db.execute(
            "update agent_conversations set archived=1 where id=?1",
            [conversation.to_string()],
        )
        .unwrap();
    }
    let third = create(&f, draft).await.unwrap();
    assert_ne!(third.conversation_id, Some(conversation));
    assert_eq!(counts(&f), (3, 3, 3, 1));
    let pointer: String = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "select conversation_id from paper_generation_conversations where paper_id='paper'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(pointer, third.conversation_id.unwrap().to_string());
    f.service.shutdown().await.unwrap();
}

#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_creation_rolls_back_the_original_conversation_binding_and_supersession() {
    let (catalog, key) = actual_package();
    let mut f = Fixture::new();
    f.bind_actual_package(catalog, key);
    let draft = seed(&f);
    let prior = create(&f, draft.clone()).await.unwrap();
    {
        let db = f.store.connection().unwrap();
        db.execute(
            "update paper_analysis_workflows set status='failed' where id=?1",
            [&prior.id],
        )
        .unwrap();
        db.execute(
            "update agent_conversations set archived=1 where id=?1",
            [prior.conversation_id.unwrap().to_string()],
        )
        .unwrap();
        db.execute_batch("create temp trigger test_analysis_creation_abort before insert on paper_analysis_turns begin select raise(abort,'synthetic creation write'); end;").unwrap();
    }
    let conversations: i64 = f
        .store
        .connection()
        .unwrap()
        .query_row("select count(*) from agent_conversations", [], |row| {
            row.get(0)
        })
        .unwrap();
    let failure = create(&f, draft.clone()).await.unwrap_err();
    assert_eq!(failure["analysisWorkflowOperation"], "createWorkflow");
    assert_eq!(failure["analysisWorkflowPhase"], "write");
    assert_eq!(failure["analysisWorkflowOutcome"], "rolledBack");
    assert_eq!(counts(&f), (1, 1, 1, 1));
    {
        let db = f.store.connection().unwrap();
        let status: String = db
            .query_row(
                "select status from paper_analysis_workflows where id=?1",
                [&prior.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(status, "failed");
        let after: i64 = db
            .query_row("select count(*) from agent_conversations", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(after, conversations);
        assert!(db.is_autocommit());
        db.execute_batch("drop trigger test_analysis_creation_abort;")
            .unwrap();
    }
    assert!(create(&f, draft).await.is_ok());
    f.service.shutdown().await.unwrap();
}

#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_creation_keeps_original_draft_and_malformed_conversation_error_order() {
    let (catalog, key) = actual_package();
    let mut f = Fixture::new();
    f.bind_actual_package(catalog, key);
    let draft = seed(&f);
    {
        let db = f.store.connection().unwrap();
        db.execute(
            "insert into agent_conversations(id,title) values('legacy-invalid','Legacy')",
            [],
        )
        .unwrap();
        db.execute("insert into paper_generation_conversations(paper_id,conversation_id) values('paper','legacy-invalid')",[]).unwrap();
    }
    let malformed = create(&f, draft.clone()).await.unwrap_err();
    assert_eq!(malformed["code"], "temporary_unavailable");
    assert_eq!(malformed["analysisWorkflowOutcome"], "rolledBack");
    assert_eq!(counts(&f), (0, 0, 0, 1));
    f.store
        .connection()
        .unwrap()
        .execute_batch(
            "create temp view paper_analysis_workflows as select 'poisoned' as unavailable;",
        )
        .unwrap();
    let mut invalid = draft;
    invalid.provider_id = " ".into();
    let validation = create(&f, invalid).await.unwrap_err();
    assert_eq!(validation["code"], "invalid_input");
    assert_eq!(validation["message"], "论文分析工作流参数不完整");
    f.service.shutdown().await.unwrap();
}
