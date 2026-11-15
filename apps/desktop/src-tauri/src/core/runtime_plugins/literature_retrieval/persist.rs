//! Original trace writes under the original Deferred transaction.
use super::{failure::Error, resources::RetrievalResources};
use crate::{
    ai::structured_resource_validation::{Failure, FailureCause, Flow},
    data_interface::plugin_resources::PluginDataError,
    hbs_storage::boundary::{self, BoundaryError, BoundaryFailure, HbsOutcome},
};
use research_workbench_native_sdk::concept_retrieval::{
    ConceptRetrievalEdge, ConceptRetrievalSeed, ConceptRetrievalSource,
};
use rusqlite::{Transaction, params};
use std::convert::Infallible;

fn persist_seeds(
    transaction: &Transaction<'_>,
    resources: &RetrievalResources,
    trace_id: &str,
    seeds: &[ConceptRetrievalSeed],
) -> Result<(), Failure<String>> {
    for (ordinal, seed) in seeds.iter().enumerate() {
        resources
            .execute(
                transaction,
                "trace-seed-insert",
                params![
                    trace_id,
                    seed.concept_id,
                    seed.concept_revision,
                    i64::try_from(ordinal).unwrap_or(i64::MAX),
                    seed.lexical_rank,
                    seed.vector_rank,
                    seed.vector_distance,
                    seed.rrf_score,
                    seed.label,
                    seed.preferred_term,
                    serde_json::to_string(&seed.matched_by).map_err(|_| {
                        Failure::original("检索匹配方式无效".to_owned())
                    })?,
                ],
            )
            .map_err(data)?;
    }
    Ok(())
}

fn persist_edges(
    transaction: &Transaction<'_>,
    resources: &RetrievalResources,
    trace_id: &str,
    edges: &[ConceptRetrievalEdge],
) -> Result<(), Failure<String>> {
    for (ordinal, edge) in edges.iter().enumerate() {
        resources
            .execute(
                transaction,
                "trace-edge-insert",
                params![
                    trace_id,
                    edge.relation_assertion_id,
                    edge.relation_revision,
                    edge.subject_concept_id,
                    edge.subject_label,
                    edge.predicate_kind,
                    edge.object_concept_id,
                    edge.object_label,
                    edge.hop,
                    i64::try_from(ordinal).unwrap_or(i64::MAX),
                ],
            )
            .map_err(data)?;
    }
    Ok(())
}

fn persist_sources(
    transaction: &Transaction<'_>,
    resources: &RetrievalResources,
    trace_id: &str,
    sources: &[ConceptRetrievalSource],
) -> Result<(), Failure<String>> {
    for (ordinal, source) in sources.iter().enumerate() {
        resources
            .execute(
                transaction,
                "trace-source-insert",
                params![
                    trace_id,
                    source.anchor_id,
                    source.concept_id,
                    source.paper_id,
                    source.source_kind,
                    source.source_subject_id,
                    source.source_subject_revision,
                    source.paper_title,
                    source.concept_label,
                    source.quote_exact,
                    source.context_text,
                    source.page_index,
                    source.page_label,
                    i64::try_from(ordinal).unwrap_or(i64::MAX),
                    source.selected,
                    source.omission_reason,
                ],
            )
            .map_err(data)?;
    }
    Ok(())
}

pub(super) struct TraceWrite {
    pub trace_id: String,
    pub paper_id: Option<String>,
    pub query_sha256: String,
    pub target: research_workbench_native_sdk::concept_retrieval::VectorTargetV1,
    pub seeds: Vec<ConceptRetrievalSeed>,
    pub edges: Vec<ConceptRetrievalEdge>,
    pub sources: Vec<ConceptRetrievalSource>,
    pub context_text: String,
    pub context_sha256: String,
    pub config_json: String,
    pub selected_source_count: i64,
    pub external_paper_count: i64,
}
pub(super) fn publish(
    db: &mut rusqlite::Connection,
    resources: &RetrievalResources,
    input: TraceWrite,
    write_started: &mut bool,
) -> Result<research_workbench_native_sdk::concept_retrieval::ConceptRetrievalPreview, Error> {
    let tx = db.transaction().map_err(|error| {
        Error::Boundary(Box::new(BoundaryFailure {
            failure: Failure::sql("概念网络数据库操作失败".into(), "begin", &error),
            phase: "begin",
            outcome: HbsOutcome::NotStarted,
            rollback_error: None,
        }))
    })?;
    *write_started = true;
    let result: Result<_, Flow<String, Infallible>> =
        write(&tx, resources, input).map_err(Flow::Fail);
    let settled = boundary::settle_with(
        tx,
        result,
        "write",
        true,
        true,
        |_| "概念网络数据库操作失败".into(),
        |_| "检索预览事务未确认结算".into(),
    );
    boundary::confirm_with(db, settled, true, |_| "检索预览事务未确认结算".into()).map_err(
        |error| match error {
            BoundaryError::Failure(error) => Error::Boundary(Box::new(error)),
            BoundaryError::Pause(pause) => match pause.need() {},
        },
    )
}
fn write(
    tx: &Transaction<'_>,
    resources: &RetrievalResources,
    input: TraceWrite,
) -> Result<
    research_workbench_native_sdk::concept_retrieval::ConceptRetrievalPreview,
    Failure<String>,
> {
    resources
        .execute(
            tx,
            "trace-insert",
            params![
                input.trace_id,
                input.paper_id,
                input.query_sha256,
                Some(input.target.embedding_space_id.as_str()),
                Some(input.target.index_generation_id.as_str()),
                input.config_json,
                input.context_text,
                input.context_sha256,
                i64::try_from(input.seeds.len()).unwrap_or(i64::MAX),
                i64::try_from(input.edges.len()).unwrap_or(i64::MAX),
                i64::try_from(
                    input
                        .sources
                        .iter()
                        .filter(|source| source.selected)
                        .count()
                )
                .unwrap_or(i64::MAX),
                input.external_paper_count
            ],
        )
        .map_err(data)?;
    persist_seeds(tx, resources, &input.trace_id, &input.seeds)?;
    persist_edges(tx, resources, &input.trace_id, &input.edges)?;
    persist_sources(tx, resources, &input.trace_id, &input.sources)?;
    let rows = resources
        .read(
            tx,
            "trace-expiry",
            vec![research_workbench_native_sdk::data::DataValue::Text(
                input.trace_id.clone(),
            )],
        )
        .map_err(data)?;
    let [row] = rows.rows.as_slice() else {
        return Err(data(resources.shape_error("trace-expiry")));
    };
    let [research_workbench_native_sdk::data::DataValue::Text(expires_at)] = row.as_slice() else {
        return Err(data(resources.shape_error("trace-expiry")));
    };
    Ok(
        research_workbench_native_sdk::concept_retrieval::ConceptRetrievalPreview {
            trace_id: input.trace_id,
            semantic_status: "used".into(),
            embedding_space_id: Some(input.target.embedding_space_id),
            index_generation_id: Some(input.target.index_generation_id),
            seeds: input.seeds,
            edges: input.edges,
            sources: input.sources,
            selected_source_count: input.selected_source_count,
            external_paper_count: input.external_paper_count,
            context_sha256: input.context_sha256,
            expires_at: expires_at.clone(),
        },
    )
}
fn data(error: Box<PluginDataError>) -> Failure<String> {
    Failure {
        error: "概念网络数据库操作失败".into(),
        cause: FailureCause::SourceData(error),
    }
}
