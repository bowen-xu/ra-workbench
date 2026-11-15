//! Original adoption transaction, with explicit settlement before a Native wait.
use super::{
    bibliography_entries::bibliography_error,
    bibliography_transaction::{self, Attempt, Phases, sql_failure},
};
use crate::{
    agent::writing_error::WritingError,
    ai::structured_resource_validation::{Failure, FailureCause, Flow},
};
use research_workbench_native_sdk::{
    writing_bibliography::adoption::{AdoptionDecision, AdoptionInput, ImportedFile},
    writing_project::ManuscriptFile,
};
use rusqlite::{Connection, Transaction, params};
use serde_json::Value;

pub(crate) struct AdoptionSql<'a> {
    pub(crate) registered: &'a str,
    pub(crate) insert_file: &'a str,
    pub(crate) insert_entry: &'a str,
    pub(crate) insert_member: &'a str,
}

pub(crate) struct AdoptionBatch<'a> {
    pub(crate) manuscript_id: &'a str,
    pub(crate) parsed: &'a [ImportedFile],
    pub(crate) originals: Option<&'a [(String, String)]>,
    pub(crate) files: &'a [ManuscriptFile],
}

pub(crate) type AdoptionRead<T, Need> = Result<T, Flow<WritingError, Need>>;

fn decision<Need>(
    input: AdoptionInput,
    decide: &mut impl FnMut(AdoptionInput) -> AdoptionRead<AdoptionDecision, Need>,
) -> AdoptionRead<AdoptionDecision, Need> {
    match decide(input)? {
        AdoptionDecision::Rejected { reason } => Err(Flow::original(bibliography_error(reason))),
        value => Ok(value),
    }
}

fn mismatch<Need>() -> Flow<WritingError, Need> {
    Flow::Fail(Failure {
        error: WritingError::InvalidBibtexEntry,
        cause: FailureCause::SourceReadUnavailable,
    })
}

/// Need is returned only after the original transaction has explicitly rolled
/// back. An uncertain commit/rollback instead fences the original database owner.
pub(crate) fn adopt_in<Need>(
    db: &Connection,
    sql: &AdoptionSql<'_>,
    batch: &AdoptionBatch<'_>,
    read_hash: &mut impl FnMut(&str) -> Result<String, WritingError>,
    decide: &mut impl FnMut(AdoptionInput) -> AdoptionRead<AdoptionDecision, Need>,
    library: &mut impl FnMut(&Connection, &Value) -> AdoptionRead<Option<Value>, Need>,
) -> Attempt<(), Need> {
    bibliography_transaction::run(
        db,
        Phases {
            begin: "bibliography_adoption_begin",
            commit: "bibliography_adoption_commit",
            rollback: "bibliography_adoption_rollback",
            unsettled: "writing_bibliography_adoption_unsettled",
        },
        |tx| insert_in(tx, sql, batch, read_hash, decide, library),
    )
}

fn insert_in<Need>(
    tx: &Transaction<'_>,
    sql: &AdoptionSql<'_>,
    batch: &AdoptionBatch<'_>,
    read_hash: &mut impl FnMut(&str) -> Result<String, WritingError>,
    decide: &mut impl FnMut(AdoptionInput) -> AdoptionRead<AdoptionDecision, Need>,
    library: &mut impl FnMut(&Connection, &Value) -> AdoptionRead<Option<Value>, Need>,
) -> AdoptionRead<(), Need> {
    for file in batch.parsed {
        let registered = tx
            .query_row(
                sql.registered,
                params![batch.manuscript_id, file.path],
                |row| row.get::<_, bool>(0),
            )
            .map_err(|cause| sql_failure(cause, "bibliography_adoption_registered"))?;
        if registered {
            continue;
        }
        let hash = read_hash(&file.path).map_err(Flow::original)?;
        tx.execute(
            sql.insert_file,
            params![
                batch.manuscript_id,
                file.path,
                file.original,
                file.support,
                hash
            ],
        )
        .map_err(|cause| sql_failure(cause, "bibliography_adoption_file"))?;
        for entry in &file.entries {
            let accepted = decision(
                AdoptionInput::ValidateEntry {
                    entry: Box::new(entry.clone()),
                    path: file.path.clone(),
                    originals: batch.originals.map(<[_]>::to_vec),
                    files: batch.files.to_vec(),
                },
                decide,
            )?;
            if !matches!(accepted, AdoptionDecision::EntryAccepted) {
                return Err(mismatch());
            }
            let library = library(tx, &entry.metadata)?;
            let AdoptionDecision::CanonicalEntry { text } = decision(
                AdoptionInput::CanonicalEntry {
                    entry: Box::new(entry.clone()),
                    library: library.clone(),
                },
                decide,
            )?
            else {
                return Err(mismatch());
            };
            let id = uuid::Uuid::new_v4().to_string();
            tx.execute(
                sql.insert_entry,
                params![
                    id,
                    batch.manuscript_id,
                    entry.key,
                    entry.original_key,
                    entry.original,
                    serde_json::to_string(&entry.metadata)
                        .map_err(|cause| Flow::original(WritingError::Json(cause)))?,
                    text,
                    library.as_ref().and_then(|item| item["id"].as_str()),
                    library
                        .as_ref()
                        .map(serde_json::to_string)
                        .transpose()
                        .map_err(|cause| Flow::original(WritingError::Json(cause)))?
                ],
            )
            .map_err(|cause| sql_failure(cause, "bibliography_adoption_entry"))?;
            tx.execute(
                sql.insert_member,
                params![batch.manuscript_id, file.path, id],
            )
            .map_err(|cause| sql_failure(cause, "bibliography_adoption_member"))?;
        }
    }
    Ok(())
}
