//! Original ordered bibliography reads, with a suspendable domain decision port.
//! Callers own the guard. A missing decision returns before any Native wait.
use crate::{
    agent::writing_error::WritingError,
    ai::structured_resource_validation::{Failure, FailureCause, Flow},
};
use research_workbench_native_sdk::{
    writing_bibliography::{
        BibliographyError, EntryDecision, EntryInput, ImportedRecord, ProjectBibliographyEntry,
    },
    writing_project::CitationBinding,
};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::Value;

/// These are the existing package-owned statements, supplied by composition.
pub(crate) struct EntrySql<'a> {
    pub(crate) rows: &'a str,
    pub(crate) support: &'a str,
    pub(crate) members: &'a str,
    pub(crate) base: &'a str,
    pub(crate) bindings: &'a str,
}

pub(crate) type EntryRead<T, Need> = Result<T, Flow<WritingError, Need>>;

fn original<E: Into<WritingError>, Need>(cause: E) -> Flow<WritingError, Need> {
    Flow::original(cause.into())
}

pub(super) fn bibliography_error(reason: BibliographyError) -> WritingError {
    match reason {
        BibliographyError::InvalidPath => WritingError::InvalidPath,
        BibliographyError::InvalidBibtexEntry => WritingError::InvalidBibtexEntry,
        BibliographyError::BibliographyImport(cause) => WritingError::BibliographyImport(cause),
    }
}

fn decision<Need>(
    input: EntryInput,
    decide: &mut impl FnMut(EntryInput) -> EntryRead<EntryDecision, Need>,
) -> EntryRead<EntryDecision, Need> {
    match decide(input)? {
        EntryDecision::Rejected { reason } => Err(Flow::original(bibliography_error(reason))),
        value => Ok(value),
    }
}

fn mismatch<Need>() -> Flow<WritingError, Need> {
    Flow::Fail(Failure {
        error: WritingError::InvalidBibtexEntry,
        cause: FailureCause::SourceReadUnavailable,
    })
}

/// Repeat only this read-only prefix after resolving a decision outside the guard.
/// The final pass re-reads every consumed row under one guard, in original order.
pub(crate) fn read_in<Need>(
    db: &Connection,
    sql: &EntrySql<'_>,
    manuscript_id: &str,
    decide: &mut impl FnMut(EntryInput) -> EntryRead<EntryDecision, Need>,
    library: &mut impl FnMut(&Connection, &str, Option<&str>) -> EntryRead<Option<Value>, Need>,
) -> EntryRead<Vec<ProjectBibliographyEntry>, Need> {
    let rows = db
        .prepare(sql.rows)
        .map_err(original)?
        .query_map([manuscript_id], |row| {
            Ok(ImportedRecord {
                id: row.get(0)?,
                citation_key: row.get(1)?,
                original_key: row.get(2)?,
                original_bibtex: row.get(3)?,
                bibtex_entry: row.get(4)?,
                imported_json: row.get(5)?,
                library_item_id: row.get(6)?,
            })
        })
        .map_err(original)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(original)?;
    let support = db
        .prepare(sql.support)
        .map_err(original)?
        .query_map([manuscript_id], |row| row.get::<_, String>(0))
        .map_err(original)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(original)?
        .join("\n");
    let EntryDecision::ImportedPrepared { entries } = decision(
        EntryInput::PrepareImported {
            manuscript_id: manuscript_id.into(),
            rows,
            support,
        },
        decide,
    )?
    else {
        return Err(mismatch());
    };
    let mut result = Vec::with_capacity(entries.len());
    for entry in entries {
        let record = &entry.record;
        let paths = db
            .prepare(sql.members)
            .map_err(original)?
            .query_map(params![manuscript_id, record.id], |row| row.get(0))
            .map_err(original)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(original)?;
        let base_key = db
            .query_row(sql.base, params![manuscript_id, record.id], |row| {
                row.get(0)
            })
            .optional()
            .map_err(original)?;
        let library_item = record
            .library_item_id
            .as_deref()
            .map(|source| library(db, source, Some(&record.id)))
            .transpose()?
            .flatten();
        let imported = serde_json::from_str(&record.imported_json).map_err(original)?;
        let EntryDecision::Entry { entry } = decision(
            EntryInput::FinishImported {
                entry: Box::new(entry),
                imported,
                paths,
                base_key,
                library_item,
            },
            decide,
        )?
        else {
            return Err(mismatch());
        };
        result.push(*entry);
    }
    let bindings = db
        .prepare(sql.bindings)
        .map_err(original)?
        .query_map([manuscript_id], |row| {
            Ok(CitationBinding {
                manuscript_id: row.get(0)?,
                literature_item_id: row.get(1)?,
                citation_key: row.get(2)?,
                bibtex_entry: row.get(3)?,
                state: row.get(4)?,
                pinned_at: row.get(5)?,
                updated_at: row.get(6)?,
            })
        })
        .map_err(original)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(original)?;
    for binding in bindings {
        let occupied_keys = result
            .iter()
            .map(|entry| entry.citation_key.clone())
            .collect();
        let EntryDecision::AutomaticPrepared { entry } = decision(
            EntryInput::PrepareAutomatic {
                manuscript_id: manuscript_id.into(),
                binding: Box::new(binding),
                occupied_keys,
            },
            decide,
        )?
        else {
            return Err(mismatch());
        };
        let Some(entry) = entry else { continue };
        let base_key = db
            .query_row(
                sql.base,
                params![manuscript_id, entry.reference_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(original)?;
        let library_item = if entry.read_library {
            library(db, &entry.binding.literature_item_id, None)?
        } else {
            None
        };
        let EntryDecision::Entry { entry } = decision(
            EntryInput::FinishAutomatic {
                entry,
                base_key,
                library_item,
            },
            decide,
        )?
        else {
            return Err(mismatch());
        };
        result.push(*entry);
    }
    Ok(result)
}
