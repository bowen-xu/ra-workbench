//! Re-read the original preparation prefix between finite Native decisions.
use super::*;
use crate::{
    ai::{
        reading_contracts::{LlmModelSelection, StoredImageAsset},
        reading_support::{PaperReadingFacts, PaperReadingImages},
    },
    data_interface::guarded_transaction,
    runtime_plugins::literature_context::{
        BuildCapture, SnapshotCapture, preparation::SessionCapture,
    },
};
use research_workbench_native_sdk::paper_context::{
    PreparedPaperContextV1, SnapshotDigestInputV1, SnapshotProjectionInputV1, SnapshotProjectionV1,
};
use research_workbench_native_sdk::paper_context_prepare::{
    PaperPreparationDecisionV1 as Decision, PaperPreparationInputV1 as Policy,
};
use rusqlite::Connection;
use std::collections::{BTreeMap, BTreeSet};
#[derive(Clone, Debug, PartialEq)]
pub(super) struct PolicyCapture {
    pub input: Policy,
    hint: Option<(SessionCapture, Option<LlmModelSelection>)>,
}
struct Decided {
    capture: PolicyCapture,
    decision: Decision,
    origin: DecisionOrigin,
}
#[derive(Default)]
pub(super) struct Cache {
    pub bootstrap: Option<Box<super::bootstrap::BootstrapCache>>,
    pub built: Option<(Arc<BuildCapture>, Arc<PreparedPaperContextV1>)>,
    policies: Vec<Decided>,
    assets: BTreeMap<String, StoredImageAsset>,
    image_started: bool,
    pub projected: Option<(Arc<SnapshotProjectionInputV1>, SnapshotProjectionV1)>,
    pub digested: Option<(SnapshotCapture, String)>,
}
pub(super) enum Need {
    ReadingHistory(research_workbench_native_sdk::reading_history::ReadingHistoryInputV1),
    AnalysisHistory(Box<research_workbench_native_sdk::analysis_history::HistoryInputV1>),
    Build(Arc<BuildCapture>),
    Policy(Box<PolicyCapture>),
    Project(Arc<SnapshotProjectionInputV1>),
    Digest {
        capture: Box<SnapshotCapture>,
        input: SnapshotDigestInputV1,
    },
}
pub(super) enum Step {
    Complete(Output),
    Need(Box<Need>),
}
impl Cache {
    pub fn progress(&self) -> Progress {
        Progress {
            image_write_started: self.image_started,
            saved_image_asset_count: self.assets.len(),
        }
    }
    pub fn push(&mut self, capture: PolicyCapture, decision: Decision, origin: DecisionOrigin) {
        self.policies.push(Decided {
            capture,
            decision,
            origin,
        });
    }
    fn policy(&self, index: usize, capture: &PolicyCapture) -> Result<Option<&Decided>, Error> {
        let Some(decided) = self.policies.get(index) else {
            return Ok(None);
        };
        if &decided.capture != capture {
            return Err(Error::conflict(None));
        }
        if let Decision::Rejected { message } = &decided.decision {
            return Err(Error::rejected(message.clone(), decided.origin.clone()));
        }
        Ok(Some(decided))
    }
}
pub(super) async fn attempt(
    store: AppStore,
    operation: Operation,
    resources: Arc<PreparationResources>,
    mut cache: Cache,
) -> Result<(Cache, Result<Step, Error>), Error> {
    tokio::task::spawn_blocking(move || {
        let mut db = store
            .connection()
            .map_err(|error| Error::original(AiError::Storage(error), "source-read"))?;
        let result = guarded_transaction::run_mut(&mut db, |db| {
            let result = if db.is_autocommit() {
                attempt_in(&store, db, &operation, &resources, &mut cache)
            } else {
                Err(Error::unsettled())
            };
            let unknown = !db.is_autocommit() || result.as_ref().is_err_and(Error::needs_recovery);
            (result, unknown)
        });
        Ok((cache, result))
    })
    .await
    .map_err(|_| Error::unsettled())?
}
fn need(input: Need) -> Step {
    Step::Need(Box::new(input))
}
fn attempt_in(
    store: &AppStore,
    db: &mut Connection,
    operation: &Operation,
    resources: &PreparationResources,
    cache: &mut Cache,
) -> Result<Step, Error> {
    let (paper, attachments) = operation.scope();
    let captured = resources
        .context
        .capture_build(db, paper, attachments)
        .map_err(Error::boundary)?;
    let Some((expected, prepared)) = &cache.built else {
        return Ok(need(Need::Build(Arc::new(captured))));
    };
    if **expected != captured {
        return Err(Error::conflict(None));
    }
    let prepared = Arc::clone(prepared);
    match operation {
        Operation::Bootstrap { .. } => super::bootstrap::attempt(db, paper, &prepared, cache),
        Operation::Reading { .. } => reading(db, paper, resources, cache, &prepared),
        Operation::PortraitSources { .. } => Ok(Step::Complete(Output::Reading(Box::new(
            PreparedPaperContext::from((*prepared).clone()).into_reading_result(None, None),
        )))),
        Operation::Snapshot(request) => snapshot(store, db, request, resources, cache, &prepared),
        Operation::Analysis(request) => analysis(store, db, request, resources, cache, &prepared),
        Operation::Docling(request) => {
            match images_and_publish(
                store,
                db,
                request,
                resources,
                cache,
                &prepared,
                ImageRequirement::LegacyExtraction,
            )? {
                Step::Complete(Output::Snapshot(snapshot)) => Ok(Step::Complete(Output::Docling {
                    prepared: Box::new(
                        PreparedPaperContext::from((*prepared).clone())
                            .into_reading_result(None, None),
                    ),
                    snapshot,
                    assets: cache.assets.clone(),
                })),
                step @ Step::Need(_) => Ok(step),
                _ => Err(Error::mismatch()),
            }
        }
    }
}
fn reading(
    db: &Connection,
    paper: &str,
    resources: &PreparationResources,
    cache: &Cache,
    prepared: &PreparedPaperContextV1,
) -> Result<Step, Error> {
    let facts = PaperReadingFacts::new(db);
    let model = facts
        .selection_for_route("use_case", "paper_reading_chat")
        .map_err(|error| Error::original(error.into_ai_error(), "source-read"))?
        .or_else(|| {
            facts
                .selection_for_route("default", "vision")
                .ok()
                .flatten()
        });
    let session = resources
        .capture_session(db, paper)
        .map_err(Error::boundary)?;
    let summary = if let Some(input) = session.input.clone() {
        let capture = PolicyCapture {
            input,
            hint: Some((session, model.clone())),
        };
        let Some(decided) = cache.policy(0, &capture)? else {
            return Ok(need(Need::Policy(Box::new(capture))));
        };
        let Decision::Session { session } = &decided.decision else {
            return Err(Error::mismatch());
        };
        Some((**session).clone())
    } else {
        None
    };
    let result = PreparedPaperContext::from(prepared.clone()).into_reading_result(summary, model);
    Ok(Step::Complete(Output::Reading(Box::new(result))))
}
fn snapshot(
    store: &AppStore,
    db: &mut Connection,
    request: &PrepareRequest,
    resources: &PreparationResources,
    cache: &mut Cache,
    prepared: &PreparedPaperContextV1,
) -> Result<Step, Error> {
    let ready = PolicyCapture {
        input: Policy::SnapshotReady {
            attachment_statuses: prepared.attachment_statuses.clone(),
        },
        hint: None,
    };
    let Some(decided) = cache.policy(0, &ready)? else {
        return Ok(need(Need::Policy(Box::new(ready))));
    };
    match decided.decision {
        Decision::Ready { available: true } => {}
        Decision::Ready { available: false } => {
            return Err(Error::conflict(Some(decided.origin.clone())));
        }
        _ => return Err(Error::mismatch()),
    }
    images_and_publish(
        store,
        db,
        request,
        resources,
        cache,
        prepared,
        ImageRequirement::WhenProvided,
    )
}

fn analysis(
    store: &AppStore,
    db: &mut Connection,
    request: &PrepareRequest,
    resources: &PreparationResources,
    cache: &mut Cache,
    prepared: &PreparedPaperContextV1,
) -> Result<Step, Error> {
    let capture = PolicyCapture {
        input: Policy::AnalysisSources {
            statuses: prepared
                .attachment_statuses
                .iter()
                .map(|status| status.parse_status.clone())
                .collect(),
            picture_count: prepared.picture_item_count,
        },
        hint: None,
    };
    let Some(decided) = cache.policy(0, &capture)? else {
        return Ok(need(Need::Policy(Box::new(capture))));
    };
    if !matches!(decided.decision, Decision::Validated {}) {
        return Err(Error::mismatch());
    }
    match images_and_publish(
        store,
        db,
        request,
        resources,
        cache,
        prepared,
        ImageRequirement::AllRequested,
    )? {
        Step::Complete(Output::Snapshot(snapshot)) => Ok(Step::Complete(Output::Analysis {
            prepared: Box::new(
                PreparedPaperContext::from(prepared.clone()).into_reading_result(None, None),
            ),
            snapshot,
        })),
        step @ Step::Need(_) => Ok(step),
        _ => Err(Error::mismatch()),
    }
}

enum ImageRequirement {
    WhenProvided,
    AllRequested,
    LegacyExtraction,
}

fn images_and_publish(
    store: &AppStore,
    db: &mut Connection,
    request: &PrepareRequest,
    resources: &PreparationResources,
    cache: &mut Cache,
    prepared: &PreparedPaperContextV1,
    requirement: ImageRequirement,
) -> Result<Step, Error> {
    let policy_offset = match requirement {
        ImageRequirement::LegacyExtraction => 0,
        _ => 1,
    };
    if matches!(
        requirement,
        ImageRequirement::AllRequested | ImageRequirement::LegacyExtraction
    ) || !request.image_assets.is_empty()
    {
        let mut seen = BTreeSet::new();
        for (index, draft) in request.image_assets.iter().enumerate() {
            let capture = PolicyCapture {
                input: Policy::ImageIdentity {
                    context_item_id: draft.context_item_id.clone(),
                    existing_ids: seen.iter().cloned().collect(),
                },
                hint: None,
            };
            if cache.policy(index + policy_offset, &capture)?.is_none() {
                return Ok(need(Need::Policy(Box::new(capture))));
            }
            if cache.assets.len() == index {
                cache.image_started = true;
                let sha = PaperReadingImages::new(db, store)
                    .save(&draft.data_url)
                    .map_err(|error| Error::original(error.into_ai_error(), "write"))?;
                cache.assets.insert(
                    draft.context_item_id.clone(),
                    StoredImageAsset {
                        context_item_id: draft.context_item_id.clone(),
                        content_sha256: sha,
                        data_url: draft.data_url.clone(),
                    },
                );
            }
            seen.insert(draft.context_item_id.clone());
        }
        let capture = PolicyCapture {
            input: Policy::ImageSet {
                requested_ids: prepared
                    .image_requests
                    .iter()
                    .map(|image| image.context_item_id.clone())
                    .collect(),
                available_ids: cache.assets.keys().cloned().collect(),
            },
            hint: None,
        };
        if cache
            .policy(request.image_assets.len() + policy_offset, &capture)?
            .is_none()
        {
            return Ok(need(Need::Policy(Box::new(capture))));
        }
    }
    publish(db, request, resources, cache, prepared)
}
fn publish(
    db: &mut Connection,
    request: &PrepareRequest,
    resources: &PreparationResources,
    cache: &mut Cache,
    prepared: &PreparedPaperContextV1,
) -> Result<Step, Error> {
    let input = SnapshotProjectionInputV1 {
        paper_id: request.paper_id.clone(),
        attachment_statuses: prepared.attachment_statuses.clone(),
        context_items: prepared.context_items.clone(),
        image_hashes: cache
            .assets
            .iter()
            .map(|(id, asset)| (id.clone(), asset.content_sha256.clone()))
            .collect(),
    };
    let Some((expected, projection)) = &cache.projected else {
        return Ok(need(Need::Project(Arc::new(input))));
    };
    if **expected != input {
        return Err(Error::conflict(None));
    }
    let captured = resources
        .context
        .capture_snapshot(db, projection)
        .map_err(Error::boundary)?;
    let digest = if let Some(existing) = &captured.existing {
        match &cache.digested {
            Some((expected, digest)) if expected == &captured => Some(digest.as_str()),
            _ => {
                return Ok(need(Need::Digest {
                    input: SnapshotDigestInputV1 {
                        context_schema_version: existing.context_schema_version,
                        paper_id: existing.paper_id.clone(),
                        attachments: existing.attachments.clone(),
                        items: existing.items.clone(),
                    },
                    capture: Box::new(captured),
                }));
            }
        }
    } else {
        None
    };
    let snapshot = resources
        .context
        .apply_snapshot(db, &request.paper_id, projection, &captured, digest)
        .map_err(Error::boundary)?;
    Ok(Step::Complete(Output::Snapshot(Preparation {
        schema_version: 1,
        paper_id: snapshot.paper_id,
        snapshot_id: snapshot.id,
        context_hash: snapshot.context_hash,
    })))
}
