//! Original Docling physical evidence publication on the Core connection.
use crate::ai::paper_reading_contracts::PaperAiReadingContextItem;
use crate::ai::{
    candidate_contracts::AiEvidenceBundle, error::AiError, generation_support::AiRun,
    reading_contracts::StoredImageAsset,
};
use research_workbench_native_sdk::analysis_workflow::docling::DoclingEvidenceRow;
use rusqlite::{Connection, OptionalExtension, params};
use std::collections::BTreeMap;
use uuid::Uuid;
const RETRIEVAL_CONFIG_VERSION: &str = "docling-structured-context-v1";

pub(crate) struct DoclingStatements<'a> {
    pub existing: &'a str,
    pub title: &'a str,
    pub bundle: &'a str,
    pub anchor: &'a str,
    pub item: &'a str,
    pub context: &'a str,
}

use crate::ai::structured_resource_validation::{Failure, FailureCause, Flow};
use crate::hbs_storage::boundary::{self, BoundaryError, BoundaryFailure, HbsOutcome};
use crate::runtime_plugins::native::SafeDatabaseError;
#[cfg(test)]
use std::convert::Infallible;

pub(crate) struct DoclingEvidenceSource<'a> {
    pub run: &'a AiRun,
    pub items: &'a [PaperAiReadingContextItem],
    pub assets: &'a BTreeMap<String, StoredImageAsset>,
}
#[derive(Default)]
pub(crate) struct DoclingEvidenceIdentity {
    pub bundle_id: Option<String>,
    item_ids: BTreeMap<usize, String>,
}

#[cfg(test)]
pub(crate) fn persist_docling_evidence(
    connection: &mut Connection,
    statements: &DoclingStatements<'_>,
    mut project: impl FnMut(
        &str,
        &PaperAiReadingContextItem,
    ) -> Result<Option<DoclingEvidenceRow>, AiError>,
    run: &AiRun,
    items: &[PaperAiReadingContextItem],
    assets: &BTreeMap<String, StoredImageAsset>,
) -> Result<AiEvidenceBundle, AiError> {
    let source = DoclingEvidenceSource { run, items, assets };
    apply_docling_evidence::<Infallible>(
        connection,
        statements,
        &source,
        &mut DoclingEvidenceIdentity::default(),
        false,
        |_, _, run, item| project(run, item).map_err(Flow::original),
    )
    .map_err(|error| (*error).into_static())
}

pub(crate) fn apply_docling_evidence<P>(
    connection: &mut Connection,
    statements: &DoclingStatements<'_>,
    source: &DoclingEvidenceSource<'_>,
    identity: &mut DoclingEvidenceIdentity,
    explicit: bool,
    mut project: impl FnMut(
        usize,
        &str,
        &str,
        &PaperAiReadingContextItem,
    ) -> Result<Option<DoclingEvidenceRow>, Flow<AiError, P>>,
) -> Result<AiEvidenceBundle, Box<BoundaryError<P, AiError>>> {
    let DoclingEvidenceSource { run, items, assets } = source;
    if let Some(bundle_id) = connection
        .query_row(statements.existing, [&run.id], |row| {
            row.get::<_, String>(0)
        })
        .optional()
        .map_err(|error| original(error.into(), "source-read", HbsOutcome::NotStarted))?
    {
        return crate::ai::generation_support::load_bundle(connection, &bundle_id)
            .map_err(|error| original(error, "source-read", HbsOutcome::NotStarted));
    }
    let bundle_id = identity
        .bundle_id
        .get_or_insert_with(|| Uuid::new_v4().to_string())
        .clone();
    let paper_title = connection
        .query_row(
            statements.title,
            [items
                .first()
                .map(|item| item.paper_id.as_str())
                .unwrap_or_default()],
            |row| row.get::<_, String>(0),
        )
        .map_err(|error| original(error.into(), "source-read", HbsOutcome::NotStarted))?;
    let transaction = connection
        .transaction()
        .map_err(|error| original(error.into(), "begin", HbsOutcome::NotStarted))?;
    let result = (|| {
        transaction
            .execute(
                statements.bundle,
                params![bundle_id, run.id, run.scope_hash, RETRIEVAL_CONFIG_VERSION],
            )
            .map_err(write_error)?;
        let mut ordinal = 0_i64;
        for (index, item) in items.iter().enumerate() {
            let Some(row) = project(index, &paper_title, &run.id, item)? else {
                continue;
            };
            let evidence_item_id = identity
                .item_ids
                .entry(index)
                .or_insert_with(|| Uuid::new_v4().to_string())
                .clone();
            let anchor_id = row.anchor_id;
            let page_index = row.page_index;
            let page_label = row.page_label;
            let rects_json = row.rects_json;
            let quote = row.quote;
            let context = row.context;
            transaction
                .execute(
                    statements.anchor,
                    params![
                        anchor_id,
                        item.paper_id,
                        item.attachment_id,
                        item.content_sha256,
                        page_index,
                        page_label,
                        rects_json,
                        quote,
                    ],
                )
                .map_err(write_error)?;
            transaction
                .execute(
                    statements.item,
                    params![
                        evidence_item_id,
                        bundle_id,
                        item.id,
                        ordinal,
                        anchor_id,
                        item.paper_id,
                        paper_title,
                        item.attachment_id,
                        item.content_sha256,
                        page_index,
                        page_label,
                        quote,
                        context,
                    ],
                )
                .map_err(write_error)?;
            transaction
                .execute(
                    statements.context,
                    params![
                        evidence_item_id,
                        item.id,
                        item.parse_run_id,
                        item.raw_ref,
                        item.kind,
                        item.label,
                        serde_json::to_string(&item.regions)
                            .map_err(|_| Flow::original(AiError::TemporaryUnavailable))?,
                        item.charspan
                            .map(|span| serde_json::to_string(&span)
                                .unwrap_or_else(|_| "[]".to_owned())),
                        item.source_text_hash,
                        assets
                            .get(&item.id)
                            .map(|asset| asset.content_sha256.as_str()),
                    ],
                )
                .map_err(write_error)?;
            ordinal = ordinal.checked_add(1).ok_or_else(|| {
                Flow::original(AiError::InvalidInput(
                    "Docling 证据数量超出支持范围".to_owned(),
                ))
            })?;
        }

        Ok(())
    })();
    let settled = boundary::settle_with(
        transaction,
        result,
        "write",
        explicit,
        true,
        AiError::from,
        |_| AiError::TemporaryUnavailable,
    );
    boundary::confirm_with(connection, settled, explicit, |_| {
        AiError::TemporaryUnavailable
    })
    .map_err(Box::new)?;
    crate::ai::generation_support::load_bundle(connection, &bundle_id)
        .map_err(|error| original(error, "postcommit-read", HbsOutcome::Committed))
}

fn original<P>(
    error: AiError,
    phase: &'static str,
    outcome: HbsOutcome,
) -> Box<BoundaryError<P, AiError>> {
    Box::new(BoundaryError::Failure(BoundaryFailure {
        failure: Failure::original(error),
        phase,
        outcome,
        rollback_error: None,
    }))
}
fn write_error<P>(error: rusqlite::Error) -> Flow<AiError, P> {
    let cause = FailureCause::SourceDatabase(Box::new(SafeDatabaseError::sql("write", &error)));
    Flow::Fail(Failure {
        error: error.into(),
        cause,
    })
}
