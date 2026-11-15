//! Short Core database attempts; no guard or content lock crosses a Native wait.
use super::*;
use crate::{
    agent::context_session::resource_tools::{
        FrozenReadingEvidence, frozen_source_snapshot_with, frozen_view_with,
    },
    ai::{authorized_pages, frozen_value, restrict_items},
    data_interface::guarded_transaction,
    runtime_plugins::literature_context::{BuildCapture, SnapshotCapture},
};
use research_workbench_native_sdk::paper_context::{
    PreparedPaperContextV1, SnapshotDigestInputV1, SnapshotProjectionInputV1, SnapshotProjectionV1,
};
use rusqlite::Connection;
use std::collections::BTreeMap;

#[derive(Default)]
pub(super) struct Cache {
    scope: Option<crate::agent::writing_resources::OwnedWritingScope>,
    pub prepared: Option<(Arc<BuildCapture>, PreparedPaperContextV1)>,
    pub projected: Option<(Arc<SnapshotProjectionInputV1>, SnapshotProjectionV1)>,
    pub digested: Option<(SnapshotCapture, String)>,
}
pub(super) enum Step {
    Complete(Value),
    Build(BuildCapture),
    Project(SnapshotProjectionInputV1),
    Digest {
        capture: Box<SnapshotCapture>,
        input: SnapshotDigestInputV1,
    },
}

pub(super) async fn authorize(
    store: AppStore,
    record: SubmissionRecord,
    call: ToolCall,
) -> Result<(), SourceReadError> {
    let scope = crate::agent::writing_resources::native_scope_for_context(&store, &record).await;
    tokio::task::spawn_blocking(move || {
        let mut db = store
            .connection()
            .map_err(|_| SourceReadError::context(context_session::Error::Unavailable))?;
        guarded_transaction::run_mut(&mut db, |db| {
            let result = autocommit(db).and_then(|_| {
                structured_runtime::capture(&store, &record, db, &call, scope.clone())
                    .map(|_| ())
                    .map_err(SourceReadError::context)
            });
            let unknown = !db.is_autocommit();
            (result, unknown)
        })
    })
    .await
    .map_err(|_| SourceReadError::unsettled())?
}

pub(super) async fn attempt(
    store: AppStore,
    record: SubmissionRecord,
    call: ToolCall,
    resources: PaperContextResources,
    mut cache: Cache,
) -> Result<(Cache, Result<Step, SourceReadError>), SourceReadError> {
    if cache.scope.is_none() {
        cache.scope =
            crate::agent::writing_resources::native_scope_for_context(&store, &record).await;
    }
    tokio::task::spawn_blocking(move || {
        let mut db=store.connection().map_err(|_|SourceReadError::context(context_session::Error::Unavailable))?;
        let result=guarded_transaction::run_mut(&mut db,|db| {
            let result=autocommit(db).and_then(|_|attempt_in(&store,&record,&call,&resources,&mut cache,db));
            let unknown=!db.is_autocommit()||matches!(&result,Err(SourceReadError::Data(error)) if error.outcome==HbsOutcome::Unknown || error.rollback_error.is_some())||matches!(&result,Err(SourceReadError::Published {outcome:HbsOutcome::Unknown,..}));
            (result,unknown)
        });
        Ok((cache,result))
    }).await.map_err(|_|SourceReadError::unsettled())?
}

fn autocommit(db: &Connection) -> Result<(), SourceReadError> {
    if db.is_autocommit() {
        return Ok(());
    }
    Err(SourceReadError::Data(Box::new(BoundaryFailure {
        failure: Failure::original(crate::ai::error::AiError::TemporaryUnavailable),
        phase: "source-owner",
        outcome: HbsOutcome::Unknown,
        rollback_error: None,
    })))
}

fn attempt_in(
    store: &AppStore,
    record: &SubmissionRecord,
    call: &ToolCall,
    resources: &PaperContextResources,
    cache: &mut Cache,
    db: &mut Connection,
) -> Result<Step, SourceReadError> {
    let authorized = structured_runtime::capture(store, record, db, call, cache.scope.clone())
        .map_err(SourceReadError::context)?;
    if let Some(snapshot) = frozen_source_snapshot_with(resources, db, record, &authorized.paper_id)
        .map_err(SourceReadError::Context)?
    {
        let value = frozen_view_with(
            resources,
            db,
            &authorized.paper_id,
            &snapshot,
            &authorized.attachment_id,
            authorized.page_index,
            &authorized.scopes,
        )
        .map_err(SourceReadError::Context)?;
        return publish(db, record, call, &authorized, &snapshot, value);
    }
    let pages = authorized_pages(&authorized.scopes)
        .map_err(|_| SourceReadError::context(context_session::Error::SourceUnavailable))?;
    let attachments = pages.keys().cloned().collect::<Vec<_>>();
    let capture = resources
        .capture_build(db, &authorized.paper_id, &attachments)
        .map_err(SourceReadError::Data)?;
    if cache
        .prepared
        .as_ref()
        .is_none_or(|(old, _)| old.as_ref() != &capture)
    {
        cache.projected = None;
        cache.digested = None;
        return Ok(Step::Build(capture));
    }
    let mut prepared = cache
        .prepared
        .as_ref()
        .ok_or_else(SourceReadError::unsettled)?
        .1
        .clone();
    if prepared
        .attachment_statuses
        .iter()
        .any(|source| source.parse_status != "available")
    {
        return Err(SourceReadError::context(
            context_session::Error::SourceUnavailable,
        ));
    }
    restrict_items(&mut prepared.context_items, &pages);
    let input = SnapshotProjectionInputV1 {
        paper_id: authorized.paper_id.clone(),
        attachment_statuses: prepared.attachment_statuses,
        context_items: prepared.context_items.clone(),
        image_hashes: BTreeMap::new(),
    };
    if cache
        .projected
        .as_ref()
        .is_none_or(|(old, _)| old.as_ref() != &input)
    {
        cache.digested = None;
        return Ok(Step::Project(input));
    }
    let projection = &cache
        .projected
        .as_ref()
        .ok_or_else(SourceReadError::unsettled)?
        .1;
    let captured = resources
        .capture_snapshot(db, projection)
        .map_err(SourceReadError::Data)?;
    let digest = if let Some(existing) = &captured.existing {
        if cache
            .digested
            .as_ref()
            .is_none_or(|(old, _)| old != &captured)
        {
            let input = SnapshotDigestInputV1 {
                context_schema_version: existing.context_schema_version,
                paper_id: existing.paper_id.clone(),
                attachments: existing.attachments.clone(),
                items: existing.items.clone(),
            };
            return Ok(Step::Digest {
                capture: Box::new(captured),
                input,
            });
        }
        Some(
            cache
                .digested
                .as_ref()
                .ok_or_else(SourceReadError::unsettled)?
                .1
                .as_str(),
        )
    } else {
        None
    };
    // Authority, actual build inputs and snapshot raw rows have all just been
    // re-read on this same guard. Fresh protection and both original repair
    // transactions run inside this call without another Native await.
    let snapshot = resources
        .apply_snapshot(db, &authorized.paper_id, projection, &captured, digest)
        .map_err(SourceReadError::Data)?;
    // Preserve the original first-freeze JSON boundary. This is the existing
    // returned text projection, distinct from lossless Native transport and
    // the original snapshot canonicalization/persistence.
    let value = serde_json::to_string_pretty(&frozen_value(&snapshot.id, &prepared.context_items))
        .map_err(|_| context_session::Error::Unavailable)
        .and_then(|text| {
            serde_json::from_str(&text).map_err(|_| context_session::Error::Unavailable)
        })
        .and_then(|value| filter_read_value(value, &authorized))
        .map_err(|error| SourceReadError::Published {
            snapshot_id: snapshot.id.clone(),
            proof_confirmed: false,
            phase: "source-result",
            outcome: HbsOutcome::Committed,
            failure: Box::new(Failure::original(error)),
        })?;
    publish(db, record, call, &authorized, &snapshot.id, value)
}

fn publish(
    db: &Connection,
    record: &SubmissionRecord,
    call: &ToolCall,
    authorized: &structured_runtime::AuthorizedStructuredRead,
    snapshot: &str,
    value: Value,
) -> Result<Step, SourceReadError> {
    structured_runtime::publish(db, record, call, authorized, snapshot, value)
        .map(Step::Complete)
        .map_err(|error| SourceReadError::Published {
            snapshot_id: snapshot.into(),
            proof_confirmed: error.confirmed,
            phase: if error.confirmed {
                "source-result"
            } else {
                "source-proof"
            },
            outcome: if error.confirmed {
                HbsOutcome::Committed
            } else {
                HbsOutcome::Unknown
            },
            failure: Box::new(error.failure),
        })
}

fn filter_read_value(
    mut value: Value,
    authorized: &structured_runtime::AuthorizedStructuredRead,
) -> Result<Value, context_session::Error> {
    value["items"]
        .as_array_mut()
        .ok_or(context_session::Error::Unavailable)?
        .retain(|item| {
            item["attachmentId"] == authorized.attachment_id
                && authorized.page_index.is_none_or(|page| {
                    item["pageIndex"] == page
                        && item["regions"].as_array().is_some_and(|regions| {
                            regions.iter().all(|region| region["pageIndex"] == page)
                        })
                })
        });
    Ok(value)
}

impl FrozenReadingEvidence for PaperContextResources {
    fn frozen_belongs(
        &self,
        db: &Connection,
        snapshot: &str,
        paper: &str,
    ) -> Result<bool, Failure<context_session::Error>> {
        PaperContextResources::frozen_belongs(self, db, snapshot, paper).map_err(|error| {
            error
                .failure
                .map_error(|_| context_session::Error::SourceUnavailable)
        })
    }
    fn item_geometry(
        &self,
        db: &Connection,
        snapshot: &str,
        item: &str,
    ) -> Result<
        Option<crate::ai::paper_reading_contracts::SnapshotItemGeometry>,
        Failure<context_session::Error>,
    > {
        PaperContextResources::item_geometry(self, db, snapshot, item).map_err(|error| {
            error
                .failure
                .map_error(|_| context_session::Error::Unavailable)
        })
    }
}
