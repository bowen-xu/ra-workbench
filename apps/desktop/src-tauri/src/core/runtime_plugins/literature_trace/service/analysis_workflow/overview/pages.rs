//! Original page selection reads are rechecked before the same Core evidence publication.
use super::*;
use crate::{
    ai::{
        generation_support::GenerationStorageUnknown,
        task_evidence_support::{
            AiEvidenceBundle, TaskEvidenceFacts, TaskEvidenceKind, persist_page_evidence,
        },
    },
    data_interface::guarded_transaction,
};
use rusqlite::Connection;
mod matrix;

#[derive(Default)]
struct EvidencePolicy {
    scope: Option<(String, Vec<(String, String)>)>,
    decisions: OverviewPolicy,
    selection_scope: Option<(String, String)>,
    matrix_scope: Option<(
        String,
        crate::ai::task_evidence_support::AiMatrixSynthesisContext,
    )>,
}
impl EvidencePolicy {
    fn changed(&self, key: &str) -> AiError {
        match self.decisions.latest_origin() {
            Some(origin) => template::identified(
                "plugin_host_source_changed",
                "页证据保存时运行范围已变化，请重新载入。",
                origin,
            ),
            None => template::technical(
                LiteratureTraceService::error(
                    "plugin_host_source_changed",
                    "页证据保存时运行范围已变化，请重新载入。",
                )
                .package(key),
            ),
        }
    }

    fn capture(
        &mut self,
        db: &mut Connection,
        key: &str,
        run: &str,
        kind: TaskEvidenceKind,
    ) -> Result<AiEvidenceBundle, CaptureError> {
        let facts = TaskEvidenceFacts::new(db);
        if let Some(bundle) = facts.existing_bundle(run)? {
            return facts.load_bundle(&bundle).map_err(Into::into);
        }
        if matches!(kind, TaskEvidenceKind::Selection) {
            return self.capture_selection(db, key, run);
        }
        if matches!(kind, TaskEvidenceKind::Matrix) {
            return self.capture_matrix(db, key, run);
        }
        let scope = facts.pages_scope(run)?;
        match &self.scope {
            Some(previous) if previous != &scope => return Err(self.changed(key).into()),
            None => self.scope = Some(scope.clone()),
            _ => (),
        }
        let mut rows = Vec::new();
        for (index, (attachment_id, expected_sha)) in scope.1.iter().enumerate() {
            let attachment = facts.attachment(attachment_id)?;
            self.decisions.validated(
                index * 2,
                WorkflowInputV1::PageAttachment {
                    attachment: attachment.clone(),
                },
            )?;
            let attachment = attachment.ok_or(AiError::TemporaryUnavailable)?;
            let input = WorkflowInputV1::PageSources {
                attachment_id: attachment_id.clone(),
                attachment,
                pages: facts.page_texts(attachment_id)?,
            };
            let (decision, origin) = self.decisions.decision(index * 2 + 1, input)?;
            let WorkflowDecisionV1::PageSources { result } = decision else {
                return Err(template::identified(
                    "plugin_host_response_mismatch",
                    "页证据结果种类不匹配。",
                    origin,
                )
                .into());
            };
            let selected = result
                .clone()
                .map_err(|error| template::rejected(error, origin))?;
            if selected
                .iter()
                .any(|row| row.source.content_sha256 != *expected_sha)
            {
                return Err(
                    AiError::InvalidInput("PDF 附件版本已变化，请重新预览概要范围".into()).into(),
                );
            }
            rows.extend(selected);
        }
        persist_page_evidence(db, run, &scope.0, &rows, true).map_err(Into::into)
    }
    fn capture_selection(
        &mut self,
        db: &mut Connection,
        key: &str,
        run: &str,
    ) -> Result<AiEvidenceBundle, CaptureError> {
        let facts = TaskEvidenceFacts::new(db);
        let scope = facts.selection_scope(run)?;
        match &self.selection_scope {
            Some(previous) if previous != &scope => return Err(self.changed(key).into()),
            None => self.selection_scope = Some(scope.clone()),
            _ => (),
        }
        let source = facts.anchor(&scope.1)?;
        let (decision, origin) = self.decisions.decision(
            0,
            WorkflowInputV1::SelectionEvidence {
                source: source.clone(),
            },
        )?;
        let WorkflowDecisionV1::SelectionEvidence { result } = decision else {
            return Err(template::identified(
                "plugin_host_response_mismatch",
                "选区证据结果种类不匹配。",
                origin,
            )
            .into());
        };
        let rows = result
            .clone()
            .map_err(|error| template::rejected(error, origin))?;
        let source = source.ok_or(AiError::TemporaryUnavailable)?;
        let context = &rows
            .get(1)
            .ok_or(AiError::TemporaryUnavailable)?
            .context_text;
        crate::ai::task_evidence_support::persist_selection_evidence(
            db, run, &scope.0, &source, context, true,
        )
        .map_err(Into::into)
    }
}

impl LiteratureTraceService {
    pub(crate) async fn persist_page_task_evidence(
        self: &Arc<Self>,
        store: AppStore,
        key: String,
        run: String,
    ) -> Result<AiEvidenceBundle, AiError> {
        let identity = run.clone();
        let owner = Arc::clone(self);
        tokio::spawn(async move {
            owner
                .persist_task_evidence_owned(store, key, run, TaskEvidenceKind::Pages)
                .await
        })
        .await
        .map_err(|_| unsettled(&identity))?
    }

    pub(crate) async fn persist_selection_task_evidence(
        self: &Arc<Self>,
        store: AppStore,
        key: String,
        run: String,
    ) -> Result<AiEvidenceBundle, AiError> {
        let identity = run.clone();
        let owner = Arc::clone(self);
        tokio::spawn(async move {
            owner
                .persist_task_evidence_owned(store, key, run, TaskEvidenceKind::Selection)
                .await
        })
        .await
        .map_err(|_| unsettled(&identity))?
    }

    async fn persist_task_evidence_owned(
        &self,
        store: AppStore,
        key: String,
        run: String,
        kind: TaskEvidenceKind,
    ) -> Result<AiEvidenceBundle, AiError> {
        self.available().map_err(template::technical)?;
        let _lifetime = self.doi_operations.read().await;
        let catalog = Arc::clone(&self.catalog);
        let selected = key.clone();
        tokio::task::spawn_blocking(move || package::accept_analysis_workflow(&catalog, &selected))
            .await
            .map_err(|_| unsettled(&run))?
            .map_err(template::technical)?;
        let mut policy = EvidencePolicy::default();
        // Each wait adds the next per-attachment observation. The original
        // scope is frozen on first capture, so later source changes reject.
        loop {
            self.available().map_err(template::technical)?;
            let source = store.clone();
            let selected = key.clone();
            let identity = run.clone();
            let (next, result) = tokio::task::spawn_blocking(move || {
                let mut db = source.connection().map_err(AiError::from)?;
                let result = guarded_transaction::run_mut(&mut db, |db| {
                    if !db.is_autocommit() {
                        let error = AiError::GenerationStorageUnknown(Box::new(GenerationStorageUnknown::unsettled(
                            &identity, "evidence_bundle", "begin", "sql_transaction_already_open",
                        )));
                        return (Err(error.into()), true);
                    }
                    let result = policy.capture(db, &selected, &identity, kind);
                    let unknown = !db.is_autocommit() || matches!(&result, Err(CaptureError::Original(error)) if matches!(error.as_ref(), AiError::GenerationStorageUnknown(_)));
                    (result, unknown)
                });
                Ok::<_, AiError>((policy, result))
            }).await.map_err(|_| unsettled(&run))??;
            policy = next;
            match result {
                Ok(bundle) => return Ok(bundle),
                Err(CaptureError::Original(error)) => return Err(*error),
                Err(CaptureError::Need(input)) => {
                    let (decision, origin) = self
                        .analysis_workflow_decision(&key, *input.clone())
                        .await
                        .map_err(template::technical)?;
                    policy.decisions.record(*input, decision, origin);
                }
                Err(CaptureError::MatrixResources) => {
                    return Err(policy.decisions.incomplete_error(&key));
                }
            }
        }
    }
}

fn unsettled(run: &str) -> AiError {
    AiError::GenerationStorageUnknown(Box::new(GenerationStorageUnknown::unsettled(
        run,
        "evidence_bundle",
        "execution",
        "sql_worker_unsettled",
    )))
}
