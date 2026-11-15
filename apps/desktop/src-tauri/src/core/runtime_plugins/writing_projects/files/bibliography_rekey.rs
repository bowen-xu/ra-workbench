//! Original checkpoint/canonical/redo transaction for suspended Native decisions.
use super::bibliography_transaction::{self, Attempt, Phases, sql_failure};
use crate::{
    agent::writing_error::WritingError,
    ai::structured_resource_validation::{Failure, FailureCause, Flow},
};
use research_workbench_native_sdk::{
    writing_bibliography::rekey::{Assignment, Decision, Input, Rejection},
    writing_checkpoint::{CheckpointRecord, DocumentCheckpoint},
    writing_project::{CitationKeyWrite, ManuscriptFile},
};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use std::collections::BTreeMap;

pub(crate) struct RekeyTable<'a> {
    pub(crate) records: &'a str,
    pub(crate) temporary: &'a str,
    pub(crate) update: &'a str,
}
pub(crate) struct RekeySql<'a> {
    pub(crate) checkpoint_read: &'a str,
    pub(crate) checkpoint_update: &'a str,
    pub(crate) tables: [RekeyTable<'a>; 2],
    pub(crate) base: &'a str,
    pub(crate) writes: &'a str,
}
pub(crate) struct RekeyBatch<'a> {
    pub(crate) manuscript_id: &'a str,
    pub(crate) assignments: &'a [Assignment],
    pub(crate) mapping: &'a BTreeMap<String, String>,
}
pub(crate) type RekeyRead<T, Need> = Result<T, Flow<WritingError, Need>>;

pub(crate) fn rejection(reason: Rejection) -> WritingError {
    reason.into()
}

fn mismatch<Need>() -> Flow<WritingError, Need> {
    Flow::Fail(Failure {
        error: WritingError::InvalidBibtexEntry,
        cause: FailureCause::SourceReadUnavailable,
    })
}
fn decision<Need>(
    input: Input,
    decide: &mut impl FnMut(Input) -> RekeyRead<Decision, Need>,
) -> RekeyRead<Decision, Need> {
    match decide(input)? {
        Decision::Rejected { reason } => Err(Flow::original(rejection(reason))),
        decision => Ok(decision),
    }
}
pub(crate) fn plan_in<Need>(
    db: &Connection,
    sql: &RekeySql<'_>,
    batch: &RekeyBatch<'_>,
    files: &mut impl FnMut(&Connection) -> RekeyRead<Vec<ManuscriptFile>, Need>,
    read: &mut impl FnMut(&str) -> Result<(String, String), WritingError>,
    decide: &mut impl FnMut(Input) -> RekeyRead<Decision, Need>,
) -> Attempt<Vec<CitationKeyWrite>, Need> {
    bibliography_transaction::run(
        db,
        Phases {
            begin: "bibliography_rekey_begin",
            commit: "bibliography_rekey_commit",
            rollback: "bibliography_rekey_rollback",
            unsettled: "writing_bibliography_rekey_unsettled",
        },
        |tx| prepare_in(tx, sql, batch, files, read, decide),
    )
}

fn prepare_in<Need>(
    tx: &Transaction<'_>,
    sql: &RekeySql<'_>,
    batch: &RekeyBatch<'_>,
    files: &mut impl FnMut(&Connection) -> RekeyRead<Vec<ManuscriptFile>, Need>,
    read: &mut impl FnMut(&str) -> Result<(String, String), WritingError>,
    decide: &mut impl FnMut(Input) -> RekeyRead<Decision, Need>,
) -> RekeyRead<Vec<CitationKeyWrite>, Need> {
    let id = batch.manuscript_id;
    let mut writes = Vec::new();
    for file in files(tx)?.iter().filter(|file| file.kind == "tex") {
        let (text, actual) = read(&file.relative_path).map_err(Flow::original)?;
        if actual != file.sha256 {
            return Err(Flow::original(WritingError::FileConflict));
        }
        let Decision::Source { text: next } = decision(
            Input::Source {
                source: text.clone(),
                mapping: batch.mapping.clone(),
            },
            decide,
        )?
        else {
            return Err(mismatch());
        };
        let row: Option<(u64, String)> = tx
            .query_row(
                sql.checkpoint_read,
                params![id, file.relative_path],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|cause| sql_failure(cause, "bibliography_rekey_checkpoint_read"))?;
        // Keep the original serde error at this read point. Native owns semantic validation.
        let record = row
            .map(|(revision, payload)| {
                serde_json::from_str::<DocumentCheckpoint>(&payload)
                    .map(|checkpoint| {
                        Box::new(CheckpointRecord {
                            revision,
                            checkpoint,
                        })
                    })
                    .map_err(|cause| Flow::original(WritingError::Json(cause)))
            })
            .transpose()?;
        let Decision::Checkpoint { checkpoint } = decision(
            Input::Checkpoint {
                manuscript_id: id.into(),
                relative_path: file.relative_path.clone(),
                text: text.clone(),
                actual_sha256: actual.clone(),
                next: next.clone(),
                mapping: batch.mapping.clone(),
                record,
            },
            decide,
        )?
        else {
            return Err(mismatch());
        };
        if let Some(checkpoint) = checkpoint {
            let json = serde_json::to_string(&checkpoint)
                .map_err(|cause| Flow::original(WritingError::Json(cause)))?;
            tx.execute(sql.checkpoint_update, params![id, file.relative_path, json])
                .map_err(|cause| sql_failure(cause, "bibliography_rekey_checkpoint_update"))?;
        }
        if text != next {
            writes.push(CitationKeyWrite {
                path: file.relative_path.clone(),
                previous: actual,
                content: next,
            });
        }
    }
    for table in &sql.tables {
        let records = tx
            .prepare(table.records)
            .and_then(|mut statement| {
                statement
                    .query_map([id], |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                        ))
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .map_err(|cause| sql_failure(cause, "bibliography_rekey_records"))?;
        let Decision::TemporaryOwners { owners } = decision(
            Input::TemporaryOwners {
                keys: records
                    .iter()
                    .map(|(owner, key, _)| (owner.clone(), key.clone()))
                    .collect(),
                mapping: batch.mapping.clone(),
            },
            decide,
        )?
        else {
            return Err(mismatch());
        };
        for owner in owners {
            tx.execute(
                table.temporary,
                params![id, owner, format!("migration:{}", uuid::Uuid::new_v4())],
            )
            .map_err(|cause| sql_failure(cause, "bibliography_rekey_temporary"))?;
        }
        // Parsing stays after temporary updates, preserving the original failure order.
        for (owner, key, source) in records {
            let Decision::Canonical { key, text } = decision(
                Input::Canonical {
                    key,
                    source,
                    mapping: batch.mapping.clone(),
                },
                decide,
            )?
            else {
                return Err(mismatch());
            };
            tx.execute(table.update, params![id, owner, key, text])
                .map_err(|cause| sql_failure(cause, "bibliography_rekey_canonical"))?;
        }
    }
    for assignment in batch.assignments {
        tx.execute(
            sql.base,
            params![id, assignment.reference_id, assignment.base_key],
        )
        .map_err(|cause| sql_failure(cause, "bibliography_rekey_base"))?;
    }
    if !writes.is_empty() {
        let json = serde_json::to_string(&writes)
            .map_err(|cause| Flow::original(WritingError::Json(cause)))?;
        tx.execute(sql.writes, params![id, json])
            .map_err(|cause| sql_failure(cause, "bibliography_rekey_journal"))?;
    }
    Ok(writes)
}
