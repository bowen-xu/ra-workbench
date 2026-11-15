//! A fork starts an estimated prefill in its own remapped window.
use super::*;
use crate::agent::repository::prefill::SqlitePrefillEstimator;
use research_agent_core::{
    context::accounting::{ContextTokenEstimator, PrefillBaseline, Utf8JsonTokenEstimator},
    model::ModelUsage,
};

fn estimator(
    db: Arc<Mutex<Connection>>,
    child: u128,
    model: &ModelIdentity,
) -> SqlitePrefillEstimator {
    SqlitePrefillEstimator::new(
        db,
        *id(child).as_uuid(),
        model,
        Arc::new(Utf8JsonTokenEstimator),
    )
    .unwrap()
}

#[tokio::test]
async fn fork_reestimates_prefill_then_preserves_its_first_observed_input() {
    let (db, catalog, first, second) =
        seed_windows(CompactionOutput::TextSummary("summary".into()));
    let model = first.binding.model.clone();
    let model_json = serde_json::to_string(&model).unwrap();
    for window in [None, Some(first.window_id), Some(second.window_id)] {
        let baseline = PrefillBaseline {
            window,
            tokens: 90,
            observed: true,
        };
        db.lock().unwrap().execute(
            "insert into agent_prefill_baselines(conversation_id,model_json,window_key,rollback_epoch,baseline_json) values(?1,?2,?3,0,?4)",
            params![id(1).as_uuid().to_string(), model_json, window.map(|value| value.to_string()).unwrap_or_default(), serde_json::to_string(&baseline).unwrap()],
        ).unwrap();
    }
    for (child, turn) in [(2, 10), (3, 20), (4, 30)] {
        let mut fork = request(child);
        fork.through_turn_id = Uuid::from_u128(turn);
        catalog.fork_legacy_for_tests(fork.clone()).await.unwrap();
        let copied =
            context_windows::read_active(&db.lock().unwrap(), *id(child).as_uuid()).unwrap();
        let value = estimator(db.clone(), child, &model);
        value
            .prepare_auto_compaction(Some(200), copied.as_ref())
            .await
            .unwrap();
        // Copying the parent's observed 90 would incorrectly count 110 tokens.
        assert_eq!(
            value
                .auto_compaction_tokens(Some(200), copied.as_ref())
                .unwrap(),
            Some(0)
        );
        value
            .observe(
                &[],
                &[],
                ModelUsage {
                    input_tokens: Some(180),
                    output_tokens: Some(20),
                },
                copied.as_ref(),
            )
            .unwrap();
        value
            .persist_auto_compaction(copied.as_ref())
            .await
            .unwrap();
        assert_eq!(
            value
                .auto_compaction_tokens(Some(200), copied.as_ref())
                .unwrap(),
            Some(20)
        );
        // Fork replay cannot reset a child baseline already observed by its model.
        catalog.fork_legacy_for_tests(fork).await.unwrap();
        drop(value);
        let resumed = estimator(db.clone(), child, &model);
        resumed
            .prepare_auto_compaction(Some(280), copied.as_ref())
            .await
            .unwrap();
        assert_eq!(
            resumed
                .auto_compaction_tokens(Some(280), copied.as_ref())
                .unwrap(),
            Some(100)
        );
        resumed
            .observe(
                &[],
                &[],
                ModelUsage {
                    input_tokens: Some(260),
                    output_tokens: Some(20),
                },
                copied.as_ref(),
            )
            .unwrap();
        resumed
            .persist_auto_compaction(copied.as_ref())
            .await
            .unwrap();
        let saved: String = db.lock().unwrap().query_row(
            "select baseline_json from agent_prefill_baselines where conversation_id=?1 and model_json=?2",
            params![id(child).as_uuid().to_string(), model_json], |row| row.get(0),
        ).unwrap();
        let saved: PrefillBaseline = serde_json::from_str(&saved).unwrap();
        assert_eq!(
            saved.window,
            copied.as_ref().map(|checkpoint| checkpoint.window_id)
        );
        assert_eq!(saved.tokens, 180);
        assert!(saved.observed);
    }
    let guard = db.lock().unwrap();
    let unchanged: u64 = guard.query_row(
        "select count(*) from agent_prefill_baselines where conversation_id=?1 and json_extract(baseline_json,'$.tokens')=90 and json_extract(baseline_json,'$.observed')=1",
        [id(1).as_uuid().to_string()], |row| row.get(0),
    ).unwrap();
    assert_eq!(unchanged, 3);
}
