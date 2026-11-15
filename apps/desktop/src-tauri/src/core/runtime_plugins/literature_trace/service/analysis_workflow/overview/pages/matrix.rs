//! Preserve the original matrix bundle-first replay and frozen source order.
use super::*;
use research_workbench_native_sdk::analysis_workflow::matrix::MatrixEvidenceInputV1;
impl EvidencePolicy {
    pub(super) fn capture_matrix(
        &mut self,
        db: &mut Connection,
        key: &str,
        run: &str,
    ) -> Result<AiEvidenceBundle, CaptureError> {
        let facts = TaskEvidenceFacts::new(db);
        let hash = facts.matrix_scope_hash(run)?;
        let context = facts
            .matrix_context(run)?
            .ok_or_else(|| AiError::InvalidInput("多文献综合运行缺少冻结上下文".into()))?;
        let scope = (hash, context);
        match &self.matrix_scope {
            Some(previous) if previous != &scope => return Err(self.changed(key).into()),
            None => self.matrix_scope = Some(scope.clone()),
            _ => (),
        }
        let mut cursor = 0;
        let (sources, rows) =
            self.decisions
                .matrix_sources(db, &scope.1.evidence_anchor_ids, &mut cursor)?;
        for source in &sources {
            self.decisions.matrix_evidence(
                &mut cursor,
                MatrixEvidenceInputV1::FrozenHash {
                    expected: facts.frozen_anchor_hash(run, &source.anchor_id)?,
                    actual: source.content_sha256.clone(),
                },
            )?;
        }
        crate::ai::task_evidence_support::persist_matrix_evidence(
            db, run, &scope.0, &sources, &rows, true,
        )
        .map_err(Into::into)
    }
}
impl LiteratureTraceService {
    pub(crate) async fn persist_matrix_task_evidence(
        self: &Arc<Self>,
        store: AppStore,
        key: String,
        run: String,
    ) -> Result<AiEvidenceBundle, AiError> {
        let identity = run.clone();
        let owner = Arc::clone(self);
        tokio::spawn(async move {
            owner
                .persist_task_evidence_owned(store, key, run, TaskEvidenceKind::Matrix)
                .await
        })
        .await
        .map_err(|_| unsettled(&identity))?
    }
}
