//! Original per-row publication pauses only after confirmed rollback.
use super::*;
use crate::{
    ai::{
        candidate_contracts::AiEvidenceBundle,
        docling_generation_support::{
            DoclingEvidenceIdentity, DoclingEvidenceSource, apply_docling_evidence,
        },
        generation_support::{AiRun, GenerationStorageUnknown},
        paper_reading_contracts::PaperAiReadingContextItem,
        reading_contracts::StoredImageAsset,
        structured_resource_validation::{Failure, FailureCause, Flow},
    },
    data_interface::guarded_transaction,
    hbs_storage::boundary::BoundaryError,
    runtime_plugins::literature_analysis::docling_resources::DoclingResources,
    storage::AppStore,
};
use std::collections::BTreeMap;

struct EvidenceInput {
    run: AiRun,
    items: Vec<PaperAiReadingContextItem>,
    assets: BTreeMap<String, StoredImageAsset>,
}
struct Projection {
    decision: DoclingRowDecisionV1,
    origin: DecisionOrigin,
}
#[derive(Default)]
struct EvidencePolicy {
    title: Option<String>,
    rows: Vec<Projection>,
    identity: DoclingEvidenceIdentity,
}
impl EvidencePolicy {
    fn project(
        title: &mut Option<String>,
        rows: &[Projection],
        index: usize,
        current_title: &str,
    ) -> Result<Option<DoclingEvidenceRow>, Flow<AiError, usize>> {
        let observed = title.get_or_insert_with(|| current_title.to_owned());
        if observed != current_title {
            let origin = &rows.first().expect("a Native pause precedes replay").origin;
            return Err(Flow::original(super::super::template::identified(
                "plugin_host_source_changed",
                "Docling 证据保存时文献标题已变化，请重新载入。",
                origin,
            )));
        }
        let Some(cached) = rows.get(index) else {
            return Err(Flow::Need(index));
        };
        match &cached.decision {
            DoclingRowDecisionV1::Ready { row } => Ok(Some(row.clone())),
            DoclingRowDecisionV1::Skipped {} => Ok(None),
            DoclingRowDecisionV1::Unavailable {} => Err(Flow::Fail(Failure {
                error: super::super::template::rejected(
                    AnalysisWorkflowError::TemporaryUnavailable,
                    &cached.origin,
                ),
                cause: FailureCause::NativeRejection(Box::new(cached.origin.clone())),
            })),
        }
    }
}

enum Step {
    Saved(AiEvidenceBundle),
    Need(usize),
}

impl LiteratureTraceService {
    pub(crate) async fn persist_analysis_docling_evidence(
        self: &Arc<Self>,
        store: AppStore,
        key: String,
        run: AiRun,
        items: Vec<PaperAiReadingContextItem>,
        assets: BTreeMap<String, StoredImageAsset>,
    ) -> Result<AiEvidenceBundle, AiError> {
        let identity = run.id.clone();
        let source = Arc::new(EvidenceInput { run, items, assets });
        let owner = Arc::clone(self);
        tokio::spawn(async move { owner.persist_evidence_owned(store, key, source).await })
            .await
            .map_err(|_| unsettled(&identity))?
    }

    async fn persist_evidence_owned(
        &self,
        store: AppStore,
        key: String,
        source: Arc<EvidenceInput>,
    ) -> Result<AiEvidenceBundle, AiError> {
        self.available()
            .map_err(super::super::template::technical)?;
        let _lifetime = self.doi_operations.read().await;
        let catalog = Arc::clone(&self.catalog);
        let selected = key.clone();
        let resources = Arc::new(
            tokio::task::spawn_blocking(move || {
                let package = package::accept_analysis_workflow(&catalog, &selected)?;
                DoclingResources::load(&catalog, &package).map_err(|error| {
                    LiteratureTraceService::error(error.code, error.message).package(&selected)
                })
            })
            .await
            .map_err(|_| unsettled(&source.run.id))?
            .map_err(super::super::template::technical)?,
        );
        let mut policy = EvidencePolicy::default();
        // Each original row may require one Native decision, followed by the
        // final publication. Owned source bytes cannot change during replay.
        for _ in 0..=source.items.len() {
            self.available()
                .map_err(super::super::template::technical)?;
            let (next, step) = attempt(
                store.clone(),
                Arc::clone(&resources),
                Arc::clone(&source),
                policy,
            )
            .await?;
            policy = next;
            match step {
                Step::Saved(bundle) => return Ok(bundle),
                Step::Need(index) => {
                    let item = source.items.get(index).ok_or_else(|| mismatch(&key))?;
                    let (decision, origin) = self
                        .project_docling_evidence_row(
                            &key,
                            DoclingRowInputV1 {
                                run_id: source.run.id.clone(),
                                item: item.clone(),
                            },
                        )
                        .await?;
                    if index != policy.rows.len() {
                        return Err(mismatch(&key));
                    }
                    policy.rows.push(Projection { decision, origin });
                }
            }
        }
        Err(mismatch(&key))
    }
}

async fn attempt(
    store: AppStore,
    resources: Arc<DoclingResources>,
    source: Arc<EvidenceInput>,
    mut policy: EvidencePolicy,
) -> Result<(EvidencePolicy, Step), AiError> {
    let run = source.run.id.clone();
    tokio::task::spawn_blocking(move || {
        let mut db = store.connection().map_err(AiError::from)?;
        let result = guarded_transaction::run_mut(&mut db, |db| {
            if !db.is_autocommit() {
                return (Err(AiError::GenerationStorageUnknown(Box::new(GenerationStorageUnknown::unsettled(
                    &source.run.id, "evidence_bundle", "begin", "sql_transaction_already_open",
                )))), true);
            }
            let input = DoclingEvidenceSource { run: &source.run, items: &source.items, assets: &source.assets };
            let result = apply_docling_evidence(db, &resources.statements(), &input, &mut policy.identity, true,
                |index, title, _, _| EvidencePolicy::project(&mut policy.title, &policy.rows, index, title)).map_err(|error| *error);
            let unknown = !db.is_autocommit() || matches!(&result, Err(BoundaryError::Failure(failure)) if failure.needs_recovery());
            let result = match result {
                Ok(bundle) => Ok(Step::Saved(bundle)),
                Err(BoundaryError::Pause(pause)) => Ok(Step::Need(pause.need())),
                Err(BoundaryError::Failure(failure)) if failure.needs_recovery() => Err(AiError::GenerationStorageUnknown(Box::new(
                    GenerationStorageUnknown::from_failure(&source.run.id, "evidence_bundle", policy.identity.bundle_id.clone(), failure),
                ))),
                Err(BoundaryError::Failure(failure)) => Err(failure.failure.error),
            };
            (result, unknown)
        });
        result.map(|step| (policy, step))
    }).await.map_err(|_| unsettled(&run))?
}

fn unsettled(run: &str) -> AiError {
    AiError::GenerationStorageUnknown(Box::new(GenerationStorageUnknown::unsettled(
        run,
        "evidence_bundle",
        "execution",
        "sql_worker_unsettled",
    )))
}
fn mismatch(key: &str) -> AiError {
    super::super::template::technical(
        LiteratureTraceService::error(
            "plugin_host_response_mismatch",
            "Docling 证据决定未按原有限次序完成。",
        )
        .package(key),
    )
}
