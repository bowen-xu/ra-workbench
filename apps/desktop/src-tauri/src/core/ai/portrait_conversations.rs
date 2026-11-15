//! Durable paper-scoped portrait conversation binding; each generation keeps a new request.
use rusqlite::Transaction;
use uuid::Uuid;

use super::error::AiError;
use crate::agent::repository::legacy_ai_runs::{self, Binding};

pub(super) fn insert_binding(
    source: impl FnOnce(&Transaction<'_>, &str, Uuid) -> Result<Uuid, AiError>,
    tx: &Transaction<'_>,
    run_id: Uuid,
    paper_id: &str,
    conversation_candidate: Uuid,
    request_id: Uuid,
) -> Result<Binding, AiError> {
    let conversation = source(tx, paper_id, conversation_candidate)?;
    let binding = legacy_ai_runs::insert_existing_in(
        tx,
        run_id,
        "paper_profile_classification",
        conversation,
        request_id,
    )?;
    Ok(binding)
}

#[cfg(test)]
#[path = "portrait_conversations/tests.rs"]
mod tests;
