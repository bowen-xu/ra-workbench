use super::model::{
    ReconciliationCounts, ReconciliationGeneration, ReconciliationSummary, SemanticIndexError,
};
use crate::{ai::CancellationSignal, semantic_engine::SemanticVectorStore};
use research_workbench_native_sdk::semantic_index::rules::{
    IndexDecisionV1, IndexInputV1, ReconcileActionV1, StoredIndexRowV1,
};
use std::collections::HashMap;
pub(crate) async fn reconcile<H: super::host::IndexHost>(
    host: &H,
    run_id: &str,
    cancellation: &CancellationSignal,
) -> Result<ReconciliationSummary, H::Error> {
    let generations = host.reconciliation_targets().await?;
    let mut counts = ReconciliationCounts::default();
    for generation in generations {
        if cancellation.is_cancelled() {
            return host
                .finalize(run_id, "canceled", counts, Some("canceled"))
                .await;
        }
        if let Err(error) = reconcile_generation(host, &generation, cancellation, &mut counts).await
        {
            let status = if H::is_canceled(&error) {
                "canceled"
            } else {
                "failed"
            };
            let _ = host
                .finalize(run_id, status, counts, Some(H::error_code(&error)))
                .await;
            return Err(error);
        }
    }
    host.finalize(run_id, "succeeded", counts, None).await
}

async fn reconcile_generation<H: super::host::IndexHost>(
    host: &H,
    generation: &ReconciliationGeneration,
    cancellation: &CancellationSignal,
    counts: &mut ReconciliationCounts,
) -> Result<(), H::Error> {
    let lancedb = host
        .open(
            &generation.relative_database_path,
            generation.namespace,
            generation.dimension,
        )
        .await?;
    let metadata = lancedb
        .list_metadata()
        .await
        .map_err(SemanticIndexError::from)?
        .into_iter()
        .map(|row| (row.row_id.clone(), row))
        .collect::<HashMap<_, _>>();
    let expected = generation
        .records
        .iter()
        .map(|record| record.lance_row_id.as_str())
        .collect::<std::collections::HashSet<_>>();

    for record in &generation.records {
        if cancellation.is_cancelled() {
            return Err(SemanticIndexError::Canceled.into());
        }
        counts.checked = counts.checked.saturating_add(1);
        let decision = host
            .decide(IndexInputV1::ReconcileRecord {
                record: record.clone(),
                stored: metadata
                    .get(&record.lance_row_id)
                    .map(|stored| StoredIndexRowV1 {
                        input_sha256: stored.input_sha256.clone(),
                        dataset_version: stored.dataset_version,
                        vector_norm_bits: stored.vector_norm.to_bits(),
                    }),
            })
            .await?;
        let IndexDecisionV1::Reconcile { action } = decision else {
            return Err(SemanticIndexError::EngineProtocol(
                "index reconciliation decision mismatch".into(),
            )
            .into());
        };
        match action {
            ReconcileActionV1::Keep => {}
            ReconcileActionV1::Complete {
                dataset_version,
                vector_norm_bits,
            } => {
                confirm_reconciled_receipt(
                    metadata.get(&record.lance_row_id),
                    &record.input_sha256,
                    dataset_version,
                    vector_norm_bits,
                )?;
                host.complete_write(
                    record.clone(),
                    dataset_version,
                    f32::from_bits(vector_norm_bits),
                )
                .await?;
                counts.repaired = counts.repaired.saturating_add(1);
            }
            ReconcileActionV1::Replace => {
                lancedb
                    .delete(&record.lance_row_id)
                    .await
                    .map_err(SemanticIndexError::from)?;
                host.repair_missing(record.clone()).await?;
                counts.missing = counts.missing.saturating_add(1);
                counts.repaired = counts.repaired.saturating_add(1);
            }
            ReconcileActionV1::Missing { count_repaired } => {
                host.repair_missing(record.clone()).await?;
                counts.missing = counts.missing.saturating_add(1);
                if count_repaired {
                    counts.repaired = counts.repaired.saturating_add(1);
                }
            }
        }
    }
    for row_id in metadata
        .keys()
        .filter(|row_id| !expected.contains(row_id.as_str()))
    {
        if cancellation.is_cancelled() {
            return Err(SemanticIndexError::Canceled.into());
        }
        lancedb
            .delete(row_id)
            .await
            .map_err(SemanticIndexError::from)?;
        counts.orphan = counts.orphan.saturating_add(1);
        counts.repaired = counts.repaired.saturating_add(1);
    }
    Ok(())
}

/// Native chooses the repair action. Only the observed Core engine receipt can
/// acknowledge the actual vector version, content hash and exact norm bits.
fn confirm_reconciled_receipt(
    stored: Option<&crate::semantic_engine::StoredVectorMetadata>,
    expected_hash: &str,
    version: u64,
    norm_bits: u32,
) -> Result<(), SemanticIndexError> {
    if stored.is_some_and(|stored| {
        stored.input_sha256 == expected_hash
            && stored.dataset_version == version
            && stored.vector_norm.to_bits() == norm_bits
    }) {
        Ok(())
    } else {
        Err(SemanticIndexError::EngineProtocol(
            "index receipt differs from the observed vector".into(),
        ))
    }
}

#[cfg(test)]
mod tests;
