//! The original Core Deferred admission rechecks all prepared observations before writes.
use super::*;
use crate::ai::generation_support::{AiRun, RunCreateIdentities, insert_prepared_run_guarded};
use uuid::Uuid;

impl LiteratureTraceService {
    pub(crate) async fn admit_analysis_overview(
        self: &Arc<Self>,
        store: AppStore,
        credentials: JsonCredentialStore,
        key: String,
        draft: AiTaskSpecDraft,
        shared: Option<(Uuid, Uuid)>,
    ) -> Result<AiRun, AiError> {
        let run = Uuid::new_v4();
        let owner = Arc::clone(self);
        tokio::spawn(async move {
            owner
                .admit_overview_owned(store, credentials, key, draft, shared, run)
                .await
        })
        .await
        .map_err(|_| unsettled(run))?
    }

    async fn admit_overview_owned(
        &self,
        store: AppStore,
        credentials: JsonCredentialStore,
        key: String,
        draft: AiTaskSpecDraft,
        shared: Option<(Uuid, Uuid)>,
        run: Uuid,
    ) -> Result<AiRun, AiError> {
        self.available().map_err(template::technical)?;
        let _lifetime = self.doi_operations.read().await;
        self.available().map_err(template::technical)?;
        let task = draft.clone();
        let mut identities =
            tokio::task::spawn_blocking(move || RunCreateIdentities::new(&task, run, shared))
                .await
                .map_err(|_| {
                    template::technical(
                        Self::error(
                            "plugin_host_source_worker_unsettled",
                            "结构化阅读运行身份准备未确认完成。",
                        )
                        .package(&key),
                    )
                })??;
        let (policy, preview) = self
            .prepare_overview_policy(store.clone(), credentials.clone(), &key, draft.clone())
            .await?;
        let conversation = if draft.task_kind == "paper_profile_classification" {
            let paper = preview
                .scope_items
                .first()
                .and_then(|item| item.parent_object_id.clone())
                .ok_or_else(|| AiError::InvalidInput("画像任务缺少所属文献".into()))?;
            Some(
                self.prepare_portrait_conversation(store.clone(), key.clone(), paper)
                    .await?,
            )
        } else {
            None
        };
        self.available().map_err(template::technical)?;
        tokio::task::spawn_blocking(move || {
            let mut db = store.connection()?;
            insert_prepared_run_guarded(
                &mut db,
                &draft,
                shared,
                &mut identities,
                |db| {
                    policy
                        .capture(db, &credentials, &draft)
                        .map_err(|error| match error {
                            CaptureError::Original(error) => *error,
                            CaptureError::Need(_) | CaptureError::MatrixResources => {
                                policy.incomplete_error(&key)
                            }
                        })
                },
                |tx, paper, candidate| {
                    conversation
                        .as_ref()
                        .ok_or(AiError::CapabilityUnavailable(
                            "literature_library.paper_generation_source",
                        ))?
                        .resolve(tx, paper, candidate)
                },
            )
        })
        .await
        .map_err(|_| unsettled(run))?
    }
}

fn unsettled(run_id: Uuid) -> AiError {
    AiError::RunCreationUnknown {
        run_id,
        phase: "execution",
        safe_cause: "sql_worker_unsettled",
        prior_code: None,
        sqlite_code: None,
        sqlite_extended_code: None,
    }
}
