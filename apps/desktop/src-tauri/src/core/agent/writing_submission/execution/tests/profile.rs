//! Shared constructor checks use synthetic credentials and never start transport.
use super::*;

#[tokio::test]
async fn shared_transport_rejects_mismatched_model_reasoning_and_native_budgets() {
    let f = fixture(true);
    crate::storage::migrate(&mut f.store.connection().unwrap()).unwrap();
    let service = WritingSubmissionService::from_app_store(&f.store);
    for case in 0..7 {
        let prepared = service
            .prepare(&f.models, request(&f, Uuid::new_v4()))
            .await
            .unwrap();
        let mut record = prepared.record;
        let mut selected = profile(record.input.model.clone());
        match case {
            0 => selected.model.model_id.push_str("-other"),
            1 => {
                record.input.model.model_id.push_str("-other");
                selected.model = record.input.model.clone();
            }
            2 => record.input.reasoning_effort = Some("medium".into()),
            3 => selected.max_input_bytes = 0,
            4 => selected.max_output_tokens = 0,
            5 => selected.capabilities.tools = false,
            _ => {}
        }
        let result = selected.transport(&f.store, &record, prepared.model);
        if case == 6 {
            assert!(result.is_ok());
        } else {
            assert!(matches!(
                result,
                Err(AgentModelPreparationError::Transport(
                    ModelError::UnsupportedCapability
                ))
            ));
        }
    }
    let db = f.store.connection().unwrap();
    assert_eq!(
        db.query_row("select count(*) from agent_items", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn shared_execution_mode_preserves_writing_wire_names() {
    use crate::agent::{
        execution_profile::AgentExecutionMode, writing_start::WritingExecutionMode,
    };
    for value in ["deepSeekNonThinking", "chatGptDefault"] {
        assert!(serde_json::from_value::<AgentExecutionMode>(serde_json::json!(value)).is_ok());
        assert!(serde_json::from_value::<WritingExecutionMode>(serde_json::json!(value)).is_ok());
    }
    assert!(serde_json::from_value::<AgentExecutionMode>(serde_json::json!("automatic")).is_err());
}
