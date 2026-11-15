//! Original fourteen-column row followed by the optional template query.
use super::{ReadingUnitFailure as F, ReadingUnitProblem as E, ReadingUnitResources};
use crate::data_interface::plugin_resources::{ColumnCodec as C, RowPolicy as P};
use research_workbench_native_sdk::{
    data::DataValue as V,
    reading_units::{ReadingUnitTemplateField, StructuredReadingUnit},
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
    pub(super) fn optional_text(&mut self) -> Result<Option<String>, F> {
        match self.0.next() {
            Some(V::Text(v)) => Ok(Some(v)),
            Some(V::Null) => Ok(None),
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
    r: &ReadingUnitResources,
    key: &'static str,
    args: Vec<V>,
    codecs: &[C],
) -> Result<Row, F> {
    r.read(db, key, args, P::FirstOptional, codecs)
        .map_err(|e| F::data(key, e))?
        .rows
        .into_iter()
        .next()
        .map(Row::new)
        .ok_or_else(|| F::new(key, E::Missing))
}
pub(super) fn unit(
    db: &Connection,
    r: &ReadingUnitResources,
    id: &str,
) -> Result<StructuredReadingUnit, F> {
    read_unit(db, r, id).map_err(|mut failure| {
        if failure.reading_unit_id.is_none() {
            failure.reading_unit_id = Some(id.to_owned());
        }
        failure
    })
}
fn read_unit(
    db: &Connection,
    r: &ReadingUnitResources,
    id: &str,
) -> Result<StructuredReadingUnit, F> {
    let mut row = first(
        db,
        r,
        "read",
        vec![text(id)],
        &[
            C::String,
            C::OptionalString,
            C::OptionalString,
            C::OptionalString,
            C::OptionalString,
            C::String,
            C::String,
            C::String,
            C::String,
            C::String,
            C::I64,
            C::I64,
            C::String,
            C::String,
        ],
    )?;
    let mut unit = StructuredReadingUnit {
        template_field: None,
        id: row.text()?,
        project_id: row.optional_text()?,
        project_title: row.optional_text()?,
        paper_id: row.optional_text()?,
        paper_title: row.optional_text()?,
        unit_kind: row.text()?,
        title: row.text()?,
        body: row.text()?,
        provenance_kind: row.text()?,
        epistemic_role: row.text()?,
        revision: row.integer()?,
        evidence_count: row.integer()?,
        created_at: row.text()?,
        updated_at: row.text()?,
    };
    let rows = r
        .read(
            db,
            "template-read",
            vec![text(id)],
            P::FirstOptional,
            &[C::String, C::String],
        )
        .map_err(|e| F::data("template-read", e))?;
    unit.template_field = rows
        .rows
        .into_iter()
        .next()
        .map(|r| {
            let mut row = Row::new(r);
            Ok(ReadingUnitTemplateField {
                portrait_candidate_id: row.text()?,
                field_id: row.text()?,
            })
        })
        .transpose()?;
    Ok(unit)
}
