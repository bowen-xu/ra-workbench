use super::{
    error::{CaptureError, Result},
    resources::BootstrapResources,
    rows::{Row, read},
};
use crate::data_interface::plugin_resources::{ColumnCodec as C, RowPolicy};
use research_workbench_native_sdk::{
    data::DataValue,
    knowledge_snapshot::{KnowledgeConcept, KnowledgeTerm},
};
use rusqlite::Connection;

pub(super) const TERM_CODECS: &[C] = &[
    C::String,
    C::String,
    C::String,
    C::String,
    C::String,
    C::I64,
    C::String,
    C::String,
];

pub(super) fn decode_term(
    rows: research_workbench_native_sdk::data::DataRows,
) -> Result<KnowledgeTerm> {
    let mut row = Row::from(
        rows.rows
            .into_iter()
            .next()
            .ok_or(CaptureError::Shape("missing-term"))?,
    );
    Ok(KnowledgeTerm {
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

impl BootstrapResources {
    /// Original optional query_row + KnowledgeError::NotFound, not an invented
    /// SQLite QueryReturnedNoRows. Timestamps and revision are actual readbacks.
    pub(super) fn read_term_in(&self, db: &Connection, id: &str) -> Result<KnowledgeTerm> {
        let rows = read(
            db,
            &self.term_read,
            vec![DataValue::Text(id.into())],
            RowPolicy::FirstOptional,
            TERM_CODECS,
        )?;
        decode_term(rows)
    }

    /// Keep concept_from's main -> mapping -> preferred presence -> each term
    /// -> preferred reread order. This does not choose or normalize a term.
    pub(super) fn read_concept_in(&self, db: &Connection, id: &str) -> Result<KnowledgeConcept> {
        self.read_concept_with(db, id, |mappings| {
            mappings
                .iter()
                .find(|(_, relation)| relation == "preferred")
                .map(|(id, _)| id.clone())
                .ok_or(CaptureError::Shape("missing-preferred-term"))
        })
    }

    /// Call the consuming policy only after all original mapping rows pass
    /// their codecs; term readbacks still follow in their original order.
    pub(super) fn read_concept_with(
        &self,
        db: &Connection,
        id: &str,
        preferred: impl FnOnce(&[(String, String)]) -> Result<String>,
    ) -> Result<KnowledgeConcept> {
        let main = read(
            db,
            &self.concept_main,
            vec![DataValue::Text(id.into())],
            RowPolicy::FirstOptional,
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
        let mut main = Row::from(
            main.rows
                .into_iter()
                .next()
                .ok_or(CaptureError::Shape("missing-concept"))?,
        );
        let term_rows = read(
            db,
            &self.concept_terms,
            vec![DataValue::Text(id.into())],
            RowPolicy::All,
            &[C::String, C::String],
        )?;
        let mappings = term_rows
            .rows
            .into_iter()
            .map(|row| {
                let mut row = Row::from(row);
                Ok((row.text()?, row.text()?))
            })
            .collect::<Result<Vec<_>>>()?;
        let preferred = preferred(&mappings)?;
        let terms = mappings
            .iter()
            .map(|(id, _)| self.read_term_in(db, id))
            .collect::<Result<Vec<_>>>()?;
        let preferred_term = self.read_term_in(db, &preferred)?;
        Ok(KnowledgeConcept {
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
}
