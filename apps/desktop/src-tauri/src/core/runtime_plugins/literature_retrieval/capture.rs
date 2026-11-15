//! Decode the original fixed queries without retaining a database guard.
use super::resources::{ReadSet, RetrievalResources};
use crate::data_interface::plugin_resources::PluginDataError;
use research_workbench_native_sdk::{
    concept_retrieval::{
        ConceptIdentityV1, GraphEdgeObservationV1, SourceObservationV1, VectorObservationV1,
        VectorTargetV1,
    },
    data::{DataRows, DataValue as V},
};
use rusqlite::Connection;
type Result<T> = std::result::Result<T, Box<PluginDataError>>;
fn text(value: &str) -> V {
    V::Text(value.into())
}

pub(super) fn paper(
    db: &Connection,
    resources: &RetrievalResources,
    reads: &mut ReadSet,
    id: &str,
) -> Result<bool> {
    let rows = resources.capture(db, reads, "paper-exists", vec![text(id)])?;
    boolean(resources, "paper-exists", rows)
}
pub(super) fn lexical(
    db: &Connection,
    resources: &RetrievalResources,
    reads: &mut ReadSet,
    query: &str,
) -> Result<Vec<String>> {
    if query.is_empty() {
        return Ok(Vec::new());
    }
    resources
        .capture(
            db,
            reads,
            "lexical-search",
            vec![text(query), V::Integer(24)],
        )?
        .rows
        .into_iter()
        .map(|row| match row.as_slice() {
            [V::Text(id)] => Ok(id.clone()),
            _ => Err(resources.shape_error("lexical-search")),
        })
        .collect()
}
pub(super) fn vector_target(
    db: &Connection,
    resources: &RetrievalResources,
    reads: &mut ReadSet,
    model: &str,
) -> Result<Option<VectorTargetV1>> {
    let rows = resources.capture(db, reads, "vector-target", vec![text(model)])?;
    rows.rows
        .first()
        .map(|row| {
            let [
                V::Text(space),
                V::Text(generation),
                V::Text(path),
                V::Integer(dimension),
                V::Text(model),
            ] = row.as_slice()
            else {
                return Err(resources.shape_error("vector-target"));
            };
            Ok(VectorTargetV1 {
                embedding_space_id: space.clone(),
                index_generation_id: generation.clone(),
                relative_database_path: path.clone(),
                dimension: usize::try_from(*dimension)
                    .map_err(|_| resources.shape_error("vector-target"))?,
                model_id: model.clone(),
            })
        })
        .transpose()
}
pub(super) fn vector_hits(
    db: &Connection,
    resources: &RetrievalResources,
    reads: &mut ReadSet,
    target: &VectorTargetV1,
    hits: Vec<crate::semantic_engine::SemanticVectorSearchHit>,
) -> Result<Vec<VectorObservationV1>> {
    hits.into_iter()
        .map(|hit| {
            let valid = boolean(
                resources,
                "vector-valid",
                resources.capture(
                    db,
                    reads,
                    "vector-valid",
                    vec![
                        text(&target.index_generation_id),
                        text(&hit.row_id),
                        text(&hit.representation_id),
                        text(&hit.subject_id),
                        V::Integer(hit.subject_revision),
                    ],
                )?,
            )?;
            Ok(VectorObservationV1 {
                concept_id: hit.subject_id,
                subject_revision: hit.subject_revision,
                distance_bits: hit.distance.to_bits(),
                valid,
            })
        })
        .collect()
}
pub(super) fn identities(
    db: &Connection,
    resources: &RetrievalResources,
    reads: &mut ReadSet,
    candidates: Vec<research_workbench_native_sdk::concept_retrieval::CandidateRankV1>,
) -> Result<
    Vec<(
        research_workbench_native_sdk::concept_retrieval::CandidateRankV1,
        Option<ConceptIdentityV1>,
    )>,
> {
    candidates
        .into_iter()
        .map(|candidate| {
            let rows = resources.capture(
                db,
                reads,
                "concept-identity",
                vec![text(&candidate.concept_id)],
            )?;
            let identity = rows
                .rows
                .first()
                .map(|row| {
                    let [V::Integer(revision), V::Text(label), V::Text(term)] = row.as_slice()
                    else {
                        return Err(resources.shape_error("concept-identity"));
                    };
                    Ok(ConceptIdentityV1 {
                        revision: *revision,
                        label: label.clone(),
                        preferred_term: term.clone(),
                    })
                })
                .transpose()?;
            Ok((candidate, identity))
        })
        .collect()
}
pub(super) fn graph(
    db: &Connection,
    resources: &RetrievalResources,
    reads: &mut ReadSet,
    seed_ids: &[String],
) -> Result<Vec<GraphEdgeObservationV1>> {
    let ids = serde_json::to_string(seed_ids).map_err(|_| resources.shape_error("graph"))?;
    resources
        .capture(db, reads, "graph", vec![text(&ids), V::Integer(16)])?
        .rows
        .into_iter()
        .map(|row| {
            let [
                V::Text(id),
                V::Integer(revision),
                V::Text(subject),
                V::Text(subject_label),
                V::Text(predicate),
                V::Text(object),
                V::Text(object_label),
            ] = row.as_slice()
            else {
                return Err(resources.shape_error("graph"));
            };
            Ok(GraphEdgeObservationV1 {
                relation_assertion_id: id.clone(),
                relation_revision: *revision,
                subject_concept_id: subject.clone(),
                subject_label: subject_label.clone(),
                predicate_kind: predicate.clone(),
                object_concept_id: object.clone(),
                object_label: object_label.clone(),
            })
        })
        .collect()
}
pub(super) fn sources(
    db: &Connection,
    resources: &RetrievalResources,
    reads: &mut ReadSet,
    concept_ids: &[String],
) -> Result<(Vec<SourceObservationV1>, Vec<SourceObservationV1>)> {
    if concept_ids.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }
    let ids = serde_json::to_string(concept_ids)
        .map_err(|_| resources.shape_error("definition-sources"))?;
    let definitions = source_rows(
        resources,
        "definition-sources",
        resources.capture(
            db,
            reads,
            "definition-sources",
            vec![text(&ids), V::Integer(48)],
        )?,
    )?;
    let usages = source_rows(
        resources,
        "usage-sources",
        resources.capture(db, reads, "usage-sources", vec![text(&ids), V::Integer(48)])?,
    )?;
    Ok((definitions, usages))
}
fn source_rows(
    resources: &RetrievalResources,
    key: &'static str,
    rows: DataRows,
) -> Result<Vec<SourceObservationV1>> {
    rows.rows
        .into_iter()
        .map(|row| {
            let [
                V::Text(anchor),
                V::Text(concept),
                V::Text(label),
                V::Text(paper),
                V::Text(title),
                V::Text(subject),
                V::Integer(revision),
                V::Integer(page),
                page_label,
                V::Text(quote),
                V::Text(body),
                V::Text(detail),
            ] = row.as_slice()
            else {
                return Err(resources.shape_error(key));
            };
            let page_label = match page_label {
                V::Null => None,
                V::Text(value) => Some(value.clone()),
                _ => return Err(resources.shape_error(key)),
            };
            Ok(SourceObservationV1 {
                anchor_id: anchor.clone(),
                concept_id: concept.clone(),
                concept_label: label.clone(),
                paper_id: paper.clone(),
                paper_title: title.clone(),
                source_subject_id: subject.clone(),
                source_subject_revision: *revision,
                page_index: *page,
                page_label,
                quote_exact: quote.clone(),
                text: body.clone(),
                detail: detail.clone(),
            })
        })
        .collect()
}
fn boolean(resources: &RetrievalResources, key: &'static str, rows: DataRows) -> Result<bool> {
    match rows.rows.as_slice() {
        [row] => match row.as_slice() {
            [V::Integer(value)] => Ok(*value != 0),
            _ => Err(resources.shape_error(key)),
        },
        _ => Err(resources.shape_error(key)),
    }
}
