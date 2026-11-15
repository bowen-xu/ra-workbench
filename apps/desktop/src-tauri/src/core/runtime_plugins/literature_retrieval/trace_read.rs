//! Decode original stored DTOs with the same per-row failure precedence.
use super::{failure::Error, resources::RetrievalResources};
use crate::{
    ai::knowledge_contracts::ConceptRetrievalError,
    data_interface::plugin_resources::MappedReadError,
};
use research_workbench_native_sdk::{
    concept_retrieval::{
        ConceptRetrievalEdge, ConceptRetrievalPreview, ConceptRetrievalSeed,
        ConceptRetrievalSource, decode_trace_match_kinds, validate_trace_semantic_status,
    },
    data::DataValue as V,
    duplicate_review::F64BitsV1,
};
use rusqlite::Connection;
type Result<T> = std::result::Result<T, Error>;

pub(crate) fn read(
    db: &Connection,
    resources: &RetrievalResources,
    trace: &str,
) -> Result<ConceptRetrievalPreview> {
    let rows = resources.read(db, "trace-header", vec![V::Text(trace.into())])?;
    let Some(row) = rows.rows.into_iter().next() else {
        return Err(ConceptRetrievalError::NotFound.into());
    };
    let mut row = Row::new(resources, "trace-header", row);
    let semantic_status = row.text()?;
    let embedding_space_id = row.optional_text()?;
    let index_generation_id = row.optional_text()?;
    let selected_source_count = row.integer()?;
    let external_paper_count = row.integer()?;
    let context_sha256 = row.text()?;
    let expires_at = row.text()?;
    validate_trace_semantic_status(&semantic_status)
        .map_err(|message| ConceptRetrievalError::InvalidInput(message.into()))?;
    let seeds = resources
        .map_rows(db, "trace-seeds", vec![V::Text(trace.into())], |fields| {
            let mut row = Row::new(resources, "trace-seeds", fields);
            let concept_id = row.text()?;
            let concept_revision = row.integer()?;
            let label = row.text()?;
            let preferred_term = row.text()?;
            let encoded = row.text()?;
            let lexical_rank = row.optional_integer()?;
            let vector_rank = row.optional_integer()?;
            let distance = row.optional_text()?;
            let score = row.text()?;
            let matched_by = decode_trace_match_kinds(&encoded)
                .map_err(|message| ConceptRetrievalError::InvalidInput(message.into()))?;
            let vector_distance = distance
                .map(|bits| {
                    u32::from_str_radix(&bits, 16)
                        .map(f32::from_bits)
                        .map_err(|_| row.invalid())
                })
                .transpose()?;
            let rrf_score = F64BitsV1::parse(score).map_err(|_| row.invalid())?.to_f64();
            Ok::<_, Error>(ConceptRetrievalSeed {
                concept_id,
                concept_revision,
                label,
                preferred_term,
                matched_by,
                lexical_rank,
                vector_rank,
                vector_distance,
                rrf_score,
            })
        })
        .map_err(|error| match error {
            MappedReadError::Data(error) => Error::Data(error),
            MappedReadError::Mapping(error) => error,
        })?;
    let edges = resources
        .read(db, "trace-edges", vec![V::Text(trace.into())])?
        .rows
        .into_iter()
        .map(|fields| {
            let mut row = Row::new(resources, "trace-edges", fields);
            Ok(ConceptRetrievalEdge {
                relation_assertion_id: row.text()?,
                relation_revision: row.integer()?,
                subject_concept_id: row.text()?,
                subject_label: row.text()?,
                predicate_kind: row.text()?,
                object_concept_id: row.text()?,
                object_label: row.text()?,
                hop: row.integer()?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let sources = resources
        .read(db, "trace-sources", vec![V::Text(trace.into())])?
        .rows
        .into_iter()
        .map(|fields| {
            let mut row = Row::new(resources, "trace-sources", fields);
            Ok(ConceptRetrievalSource {
                anchor_id: row.text()?,
                concept_id: row.text()?,
                concept_label: row.text()?,
                paper_id: row.text()?,
                paper_title: row.text()?,
                source_kind: row.text()?,
                source_subject_id: row.text()?,
                source_subject_revision: row.integer()?,
                page_index: row.integer()?,
                page_label: row.optional_text()?,
                quote_exact: row.text()?,
                context_text: row.text()?,
                selected: row.integer()? != 0,
                omission_reason: row.optional_text()?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(ConceptRetrievalPreview {
        trace_id: trace.into(),
        semantic_status,
        embedding_space_id,
        index_generation_id,
        seeds,
        edges,
        sources,
        selected_source_count,
        external_paper_count,
        context_sha256,
        expires_at,
    })
}
struct Row<'a> {
    resources: &'a RetrievalResources,
    key: &'static str,
    fields: std::vec::IntoIter<V>,
}
impl<'a> Row<'a> {
    fn new(resources: &'a RetrievalResources, key: &'static str, fields: Vec<V>) -> Self {
        Self {
            resources,
            key,
            fields: fields.into_iter(),
        }
    }
    fn invalid(&self) -> Error {
        Error::Data(self.resources.shape_error(self.key))
    }
    fn text(&mut self) -> Result<String> {
        match self.fields.next() {
            Some(V::Text(value)) => Ok(value),
            _ => Err(self.invalid()),
        }
    }
    fn optional_text(&mut self) -> Result<Option<String>> {
        match self.fields.next() {
            Some(V::Text(value)) => Ok(Some(value)),
            Some(V::Null) => Ok(None),
            _ => Err(self.invalid()),
        }
    }
    fn integer(&mut self) -> Result<i64> {
        match self.fields.next() {
            Some(V::Integer(value)) => Ok(value),
            _ => Err(self.invalid()),
        }
    }
    fn optional_integer(&mut self) -> Result<Option<i64>> {
        match self.fields.next() {
            Some(V::Integer(value)) => Ok(Some(value)),
            Some(V::Null) => Ok(None),
            _ => Err(self.invalid()),
        }
    }
}
