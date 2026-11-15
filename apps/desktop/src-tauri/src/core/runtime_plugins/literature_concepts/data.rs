//! Original concept_from codec/read order, including a second preferred-term read.
use super::{ConceptFailure as F, ConceptProblem as E, ConceptResources};
use crate::data_interface::plugin_resources::{ColumnCodec as C, RowPolicy as P};
use research_workbench_native_sdk::{
    concepts::{Concept, ConceptRejectionV1, Term},
    data::DataValue as V,
};
use rusqlite::Connection;
pub(super) fn text(s: &str) -> V {
    V::Text(s.into())
}
pub(super) fn optional(s: Option<&str>) -> V {
    s.map_or(V::Null, text)
}
pub(super) struct Row(std::vec::IntoIter<V>);
impl Row {
    pub(super) fn new(row: Vec<V>) -> Self {
        Self(row.into_iter())
    }
    pub(super) fn text(&mut self) -> Result<String, F> {
        match self.0.next() {
            Some(V::Text(v)) => Ok(v),
            _ => Err(F::new("codec", E::Codec)),
        }
    }
    pub(super) fn integer(&mut self) -> Result<i64, F> {
        match self.0.next() {
            Some(V::Integer(v)) => Ok(v),
            _ => Err(F::new("codec", E::Codec)),
        }
    }
}
pub(super) fn first(
    db: &Connection,
    r: &ConceptResources,
    key: &'static str,
    args: Vec<V>,
    codecs: &[C],
) -> Result<Row, F> {
    let rows = r
        .read(db, key, args, P::FirstOptional, codecs)
        .map_err(|e| F::data(key, e))?;
    rows.rows
        .into_iter()
        .next()
        .map(Row::new)
        .ok_or_else(|| F::new(key, E::Missing))
}
pub(super) fn term(db: &Connection, r: &ConceptResources, id: &str) -> Result<Term, F> {
    let mut row = first(
        db,
        r,
        "term-read",
        vec![text(id)],
        &[
            C::String,
            C::String,
            C::String,
            C::String,
            C::String,
            C::I64,
            C::String,
            C::String,
        ],
    )?;
    Ok(Term {
        id: row.text()?,
        text: row.text()?,
        normalized_text: row.text()?,
        language: row.text()?,
        provenance_kind: row.text()?,
        revision: row.integer()?,
        created_at: row.text()?,
        updated_at: row.text()?,
    })
}
pub(super) fn concept(db: &Connection, r: &ConceptResources, id: &str) -> Result<Concept, F> {
    let mut main = first(
        db,
        r,
        "concept-main",
        vec![text(id)],
        &[
            C::String,
            C::String,
            C::String,
            C::String,
            C::String,
            C::String,
            C::String,
            C::I64,
            C::I64,
            C::I64,
            C::String,
            C::String,
        ],
    )?;
    let rows = r
        .read(
            db,
            "concept-terms",
            vec![text(id)],
            P::All,
            &[C::String, C::String],
        )
        .map_err(|e| F::data("concept-terms", e))?;
    let mappings = rows
        .rows
        .into_iter()
        .map(|row| {
            let mut row = Row::new(row);
            Ok((row.text()?, row.text()?))
        })
        .collect::<Result<Vec<_>, F>>()?;
    let json = serde_json::to_string(&mappings).map_err(|_| F::new("preferred-term", E::Codec))?;
    let rows = r
        .read(
            db,
            "preferred-term-id",
            vec![V::Text(json)],
            P::FirstOptional,
            &[C::String],
        )
        .map_err(|e| F::data("preferred-term", e))?;
    let preferred = rows
        .rows
        .into_iter()
        .next()
        .map(|row| Row::new(row).text())
        .transpose()?
        .ok_or_else(|| {
            F::new(
                "preferred-term",
                E::Domain(ConceptRejectionV1::MissingPreferredTerm),
            )
        })?;
    let terms = mappings
        .iter()
        .map(|(id, _)| term(db, r, id))
        .collect::<Result<Vec<_>, _>>()?;
    let preferred_term = term(db, r, &preferred)?;
    Ok(Concept {
        id: id.into(),
        preferred_term,
        terms,
        label: main.text()?,
        explanation: main.text()?,
        disambiguation: main.text()?,
        domain: main.text()?,
        tradition: main.text()?,
        period: main.text()?,
        provenance_kind: main.text()?,
        revision: main.integer()?,
        definition_count: main.integer()?,
        source_count: main.integer()?,
        created_at: main.text()?,
        updated_at: main.text()?,
    })
}
