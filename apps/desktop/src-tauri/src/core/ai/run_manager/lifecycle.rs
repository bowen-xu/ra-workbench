//! Task ownership and sanitized persisted failure feedback.
use super::*;
use crate::agent::cancellation::CancellationRegistry;
use crate::ai::error::AiCommandError;
use std::{collections::HashMap, sync::Mutex};

#[derive(Clone, Default)]
pub(crate) struct AiRunManager {
    registry: CancellationRegistry,
    terminal_unknown: Arc<Mutex<HashMap<String, AiCommandError>>>,
}

impl AiRunManager {
    fn remember_terminal_unknown(&self, error: AiError) {
        let run_id = match &error {
            AiError::RunTerminalUnknown { run_id, .. } => run_id.clone(),
            AiError::GenerationStorageUnknown(detail) => detail.run_id().to_owned(),
            _ => return,
        };
        self.terminal_unknown
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .entry(run_id)
            .or_insert_with(|| AiCommandError::from(error));
    }

    pub(crate) fn terminal_unknown(&self, run_id: &str) -> Option<AiCommandError> {
        self.terminal_unknown
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(run_id)
            .cloned()
    }
    pub(crate) fn register(&self, run_id: &str) -> Result<CancellationSignal, AiError> {
        self.registry
            .register(run_id, false)
            .map_err(|_| AiError::InvalidInput("该 AI 运行已启动或运行 ID 无效".to_owned()))
    }

    pub(crate) fn cancel(&self, run_id: &str) -> Result<bool, AiError> {
        self.registry
            .cancel(run_id)
            .map_err(|_| AiError::TemporaryUnavailable)
    }

    fn remove(&self, run_id: &str) {
        let _ = self.registry.remove(run_id);
    }

    pub(crate) fn spawn(
        &self,
        app: AppHandle,
        store: AppStore,
        credentials: JsonCredentialStore,
        run: AiRun,
        signal: CancellationSignal,
    ) {
        let manager = self.clone();
        tauri::async_runtime::spawn(async move {
            let sequence = AtomicU64::new(0);
            let result = execute_run(&app, &store, &credentials, &run, &signal, &sequence).await;
            if let Err(error) = result {
                if matches!(
                    error,
                    AiError::RunTerminalUnknown { .. } | AiError::GenerationStorageUnknown(_)
                ) {
                    manager.remember_terminal_unknown(error);
                } else {
                    let (status, code) = if matches!(error, AiError::Canceled) {
                        ("canceled", "canceled")
                    } else {
                        ("failed", error.code())
                    };
                    let store_for_finish = store.clone();
                    let run_id = run.id.clone();
                    let code = code.to_owned();
                    let message = error.user_message();
                    let settlement = tauri::async_runtime::spawn_blocking(move || {
                        let mut connection = store_for_finish.connection()?;
                        run_state_repository::finish_run_guarded(
                            &mut connection,
                            &run_id,
                            status,
                            Some(&code),
                            Some(&message),
                            None,
                        )
                    })
                    .await;
                    if let Ok(Err(error)) = settlement {
                        manager.remember_terminal_unknown(error);
                    }
                }
            }
            manager.remove(&run.id);
            emit_progress(&app, &run.id, &sequence);
        });
    }
}

pub(super) async fn fail_stage(
    store: &AppStore,
    stage_id: &str,
    error: &ProviderError,
    started: Instant,
) -> Result<(), AiError> {
    let store = store.clone();
    let stage_id = stage_id.to_owned();
    let error_code = error.kind.code().to_owned();
    let message = provider_error(error.clone()).user_message();
    let duration_ms = elapsed_ms(started);
    tauri::async_runtime::spawn_blocking(move || {
        let connection = store.connection()?;
        run_state_repository::fail_stage(&connection, &stage_id, &error_code, &message, duration_ms)
    })
    .await
    .map_err(|_| AiError::TemporaryUnavailable)?
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn generation_storage_unknown_keeps_original_identity_and_first_diagnostic() {
        let manager = AiRunManager::default();
        let run = Uuid::new_v4().to_string();
        for cause in ["sql_commit_outcome_unknown", "later_failure"] {
            manager.remember_terminal_unknown(AiError::GenerationStorageUnknown(Box::new(
                crate::ai::generation_storage_failure::GenerationStorageUnknown::unsettled(
                    &run,
                    "evidence_bundle",
                    "commit",
                    cause,
                ),
            )));
        }
        let error = manager.terminal_unknown(&run).unwrap();
        assert_eq!(error.code(), "data_operation_outcome_unknown");
        assert!(error.message().contains(&run));
        assert!(error.message().contains("evidence_bundle"));
        assert!(error.message().contains("sql_commit_outcome_unknown"));
        assert!(!error.message().contains("later_failure"));
        assert!(manager.terminal_unknown("another-run").is_none());
    }

    #[test]
    fn terminal_unknown_keeps_original_diagnostic_for_only_its_run() {
        let manager = AiRunManager::default();
        let original = Uuid::new_v4().to_string();
        let other = Uuid::new_v4().to_string();
        manager.remember_terminal_unknown(AiError::RunTerminalUnknown {
            run_id: original.clone(),
            phase: "commit",
            safe_cause: "sql_commit_outcome_unknown",
            prior_code: None,
            sqlite_code: Some("ConstraintViolation".into()),
            sqlite_extended_code: Some(787),
        });
        manager.remember_terminal_unknown(AiError::RunTerminalUnknown {
            run_id: original.clone(),
            phase: "rollback",
            safe_cause: "later_failure",
            prior_code: None,
            sqlite_code: None,
            sqlite_extended_code: None,
        });
        let error = manager.clone().terminal_unknown(&original).unwrap();
        assert_eq!(error.code(), "ai_run_terminal_outcome_unknown");
        assert!(error.message().contains(&original));
        assert!(error.message().contains("sql_commit_outcome_unknown"));
        assert!(error.message().contains("787"));
        assert!(manager.terminal_unknown(&other).is_none());
    }

    #[test]
    fn legacy_run_projection_does_not_globally_serialize_conversations() {
        let manager = AiRunManager::default();
        let first = manager.register("run-1").expect("first run");
        let second = manager.register("run-2").expect("second run");
        assert!(manager.register("run-1").is_err());
        assert!(manager.cancel("run-1").expect("cancel first"));
        assert!(first.is_cancelled());
        assert!(!second.is_cancelled());
        manager.remove("run-1");
        manager.remove("run-2");
    }
}
