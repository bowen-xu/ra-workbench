//! Frozen ordinary/manual writing starts; no credentials or current file reads.
use super::{SqlRepositoryError, submissions, submissions::start_observation};
use crate::agent::writing_submission;
use research_agent_core::{
    protocol::{ModelIdentity, TaskKind, TurnOutcome},
    submission::{ContextSourceKind, SubmissionRecord},
};
use rusqlite::{Connection, OptionalExtension, params};
use uuid::Uuid;

#[derive(Clone)]
pub(crate) struct Intent {
    pub conversation_id: Uuid,
    pub manuscript_id: Uuid,
    pub request_id: Uuid,
    pub context_snapshot_id: Uuid,
    pub model: ModelIdentity,
    pub compact_revision: Option<u64>,
}
pub(crate) enum Fact {
    Prepared(Box<SubmissionRecord>),
    Started {
        turn_id: Uuid,
        outcome: Option<TurnOutcome>,
    },
    Missing,
}
pub(crate) struct Error {
    pub safe_cause: &'static str,
    pub sql: Option<rusqlite::Error>,
}
impl Error {
    fn sql(safe_cause: &'static str, error: rusqlite::Error) -> Self {
        Self {
            safe_cause,
            sql: Some(error),
        }
    }
}
impl From<&'static str> for Error {
    fn from(safe_cause: &'static str) -> Self {
        Self {
            safe_cause,
            sql: None,
        }
    }
}

pub(crate) fn inspect(db: &Connection, intent: &Intent) -> Result<Fact, Error> {
    let c = intent.conversation_id.to_string();
    let r = intent.request_id.to_string();
    let record =
        submissions::read_sql(db, intent.conversation_id, intent.request_id).map_err(|error| {
            match error {
                SqlRepositoryError::Sql(error) => {
                    Error::sql("writing_start_submission_unavailable", error)
                }
                SqlRepositoryError::Domain(_) => "writing_start_submission_invalid".into(),
            }
        })?;
    let Some(record) = record else {
        // A retained proof or tombstone is not permission to recreate a deleted
        // result. Missing history behind it remains uncertain and read-only.
        let evidence: bool = db.query_row(
            "select exists(select 1 from data_sql_operation_receipts where operation_id=?1) or exists(select 1 from agent_items where conversation_id=?2 and json_extract(payload_json,'$.requestId')=?1) or exists(select 1 from agent_queue_submission_origins where conversation_id=?2 and request_id=?1) or exists(select 1 from agent_conversation_deletions where conversation_id=?2)",
            params![r,c], |row| row.get(0),
        ).map_err(|error| Error::sql("writing_start_origin_unavailable", error))?;
        return if evidence {
            Err("writing_start_submission_missing".into())
        } else {
            Ok(Fact::Missing)
        };
    };
    if record.context_snapshot_id != intent.context_snapshot_id
        || record.input.model != intent.model
    {
        return Err("writing_start_intent_mismatch".into());
    }
    writing_submission::validate_scope(&record, intent.manuscript_id)
        .map_err(|_| "writing_start_scope_mismatch")?;
    // Supplemental grants describe their selected resource, not the original
    // writing host. A current binding cannot supply missing historical proof.
    let manuscript = intent.manuscript_id.to_string();
    let frozen_manuscript = record.input.context.iter().any(|source| {
        if !matches!(
            source.kind,
            ContextSourceKind::Manuscript | ContextSourceKind::Retrieval
        ) {
            return false;
        }
        let Some(anchor) = source
            .anchor
            .as_deref()
            .and_then(|value| serde_json::from_str::<serde_json::Value>(value).ok())
        else {
            return false;
        };
        anchor
            .get("supplementalContext")
            .and_then(serde_json::Value::as_bool)
            != Some(true)
            && anchor
                .get("manuscriptId")
                .and_then(serde_json::Value::as_str)
                == Some(manuscript.as_str())
    });
    if !frozen_manuscript {
        return Err("writing_start_scope_unconfirmed".into());
    }
    let queued: bool = db.query_row(
        "select exists(select 1 from agent_queue_submission_origins where conversation_id=?1 and request_id=?2)",
        params![c,r], |row| row.get(0),
    ).map_err(|error| Error::sql("writing_start_origin_unavailable", error))?;
    if queued {
        return Err("writing_start_queue_origin_mismatch".into());
    }
    let revision: Option<u64> = db.query_row(
        "select expected_revision from agent_compaction_preparations where conversation_id=?1 and request_id=?2",
        params![c,r], |row| row.get(0),
    ).optional().map_err(|error| Error::sql("writing_start_purpose_unavailable", error))?;
    if revision != intent.compact_revision
        || intent.compact_revision.is_some() && record.input.text.trim() != "/compact"
    {
        return Err("writing_start_purpose_mismatch".into());
    }
    let kind = if intent.compact_revision.is_some() {
        TaskKind::Compact
    } else {
        TaskKind::Regular
    };
    let observed = start_observation::observe(db, &record, kind).map_err(|error| match error {
        start_observation::Error::Unavailable(error) => {
            Error::sql("writing_start_history_unavailable", error)
        }
        start_observation::Error::InvalidRow(error) => {
            Error::sql("writing_start_history_invalid", error)
        }
        start_observation::Error::Invalid => "writing_start_history_invalid".into(),
        start_observation::Error::IdentityMismatch => {
            "writing_start_history_identity_mismatch".into()
        }
        start_observation::Error::PrefixInvalid => "writing_start_history_prefix_invalid".into(),
    })?;
    // This observation proves the original frozen task, not its execution mode
    // or current authorization. Prepared does not promise it can execute now.
    Ok(match observed {
        Some(outcome) => Fact::Started {
            turn_id: record.turn_id,
            outcome,
        },
        None => Fact::Prepared(Box::new(record)),
    })
}
