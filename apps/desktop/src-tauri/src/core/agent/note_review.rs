//! Review a completed native tool result, never arbitrary renderer/model JSON.
use super::{
    note_submission, note_tools,
    repository::{self, SqliteConversationCatalog},
};
use crate::{
    notes::agent_context::{NoteContextError, NoteContextRequest, materialize_in},
    storage::AppStore,
};
use research_agent_core::{
    conversation::ConversationId,
    model::ToolCall,
    protocol::{ConversationProjection, ItemKind},
    submission::{ContextSourceKind, ContextText, SubmissionRecord},
};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Scope {
    pub conversation_id: Uuid,
    pub page_id: Uuid,
    pub turn_id: Uuid,
    pub item_id: Uuid,
}
#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Error {
    InvalidInput,
    Unbound,
    Unavailable,
    Corrupt,
    Incomplete,
    Budget,
    Conflict,
    Decided,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Review {
    pub schema_version: u8,
    pub scope: Scope,
    pub proposal: Value,
    pub current_revision: i64,
    pub baseline_matches: bool,
}

#[tauri::command]
pub(crate) async fn agent_review_note_rewrite(
    store: tauri::State<'_, AppStore>,
    request: Scope,
) -> Result<Review, Error> {
    review(&store, request).await
}

/// Verify immutable provenance without requiring the selected block to still
/// exist. Callers recheck the returned record inside their live transaction.
pub(super) async fn verified_record(
    store: &AppStore,
    scope: Scope,
) -> Result<(SubmissionRecord, Value), Error> {
    if [
        scope.conversation_id,
        scope.page_id,
        scope.turn_id,
        scope.item_id,
    ]
    .iter()
    .any(Uuid::is_nil)
    {
        return Err(Error::InvalidInput);
    }
    let owned = store.clone();
    let record = tokio::task::spawn_blocking(move || {
        let mut db = owned.connection().map_err(|_| Error::Unavailable)?;
        let tx = db.transaction().map_err(|_| Error::Unavailable)?;
        bound_record(&tx, scope)
    })
    .await
    .map_err(|_| Error::Unavailable)??;
    let (_, projection) = SqliteConversationCatalog::from_app_store(store)
        .projection_snapshot(ConversationId::from_uuid(scope.conversation_id))
        .await
        .map_err(|error| match error {
            repository::SnapshotExportError::TooLarge => Error::Budget,
            repository::SnapshotExportError::Corrupt => Error::Corrupt,
            _ => Error::Unavailable,
        })?;
    tokio::task::spawn_blocking(move || {
        let proposal = verified_proposal(&record, &projection, scope)?;
        Ok((record, proposal))
    })
    .await
    .map_err(|_| Error::Unavailable)?
}

pub(crate) async fn review(store: &AppStore, scope: Scope) -> Result<Review, Error> {
    let (record, proposal) = verified_record(store, scope).await?;
    // The immutable journal snapshot needs no runtime lease. Recheck the live
    // binding and original submission after projection work before disclosure.
    let owned = store.clone();
    tokio::task::spawn_blocking(move || {
        let (current_revision, baseline_matches, full_page) = {
            let mut db = owned.connection().map_err(|_| Error::Unavailable)?;
            let tx = db.transaction().map_err(|_| Error::Unavailable)?;
            if bound_record(&tx, scope)? != record {
                return Err(Error::Corrupt);
            }
            baseline(&tx, scope, &record)?
        };
        if let Some(full_page) = full_page {
            // Whole-page assembly is CPU work over immutable snapshots. Release
            // the shared database lock before validating the replacement.
            candidate_document(&record, &proposal, &full_page)?;
        }
        Ok(Review {
            schema_version: 1,
            scope,
            proposal,
            current_revision,
            baseline_matches,
        })
    })
    .await
    .map_err(|_| Error::Unavailable)?
}

pub(super) fn bound_record(db: &Connection, scope: Scope) -> Result<SubmissionRecord, Error> {
    note_submission::check_binding(db, scope.conversation_id, scope.page_id).map_err(|error| {
        match error {
            note_submission::Error::Unbound => Error::Unbound,
            _ => Error::Unavailable,
        }
    })?;
    let request: String = db
        .query_row(
            "select request_id from agent_submissions where conversation_id=?1 and turn_id=?2",
            params![scope.conversation_id.to_string(), scope.turn_id.to_string()],
            |r| r.get(0),
        )
        .optional()
        .map_err(|_| Error::Unavailable)?
        .ok_or(Error::Unavailable)?;
    let record = repository::submissions::read(
        db,
        scope.conversation_id,
        Uuid::parse_str(&request).map_err(|_| Error::Corrupt)?,
    )
    .map_err(|_| Error::Corrupt)?
    .ok_or(Error::Unavailable)?;
    let [source] = record.input.context.as_slice() else {
        return Err(Error::Unbound);
    };
    if record.turn_id != scope.turn_id
        || source.kind != ContextSourceKind::Note
        || source.resource_id != scope.page_id.to_string()
    {
        return Err(Error::Unbound);
    }
    Ok(record)
}

pub(super) fn verified_proposal(
    record: &SubmissionRecord,
    projection: &ConversationProjection,
    scope: Scope,
) -> Result<Value, Error> {
    let turn = projection.turn(scope.turn_id).ok_or(Error::Incomplete)?;
    if turn.context_snapshot_id != record.context_snapshot_id || turn.model != record.input.model {
        return Err(Error::Corrupt);
    }
    let result = turn.items.get(&scope.item_id).ok_or(Error::Unavailable)?;
    if !result.completed || result.interrupted {
        return Err(Error::Incomplete);
    }
    let ItemKind::ToolResult {
        call_id,
        is_error: Some(false),
    } = result.kind
    else {
        return Err(Error::InvalidInput);
    };
    let matches: Vec<_> = turn
        .item_order
        .iter()
        .enumerate()
        .filter_map(|(index, id)| {
            let item = turn.items.get(id)?;
            match &item.kind {
                ItemKind::ToolCall {
                    call_id: candidate, ..
                } if *candidate == call_id => Some((index, item)),
                _ => None,
            }
        })
        .collect();
    let [(call_index, call)] = matches.as_slice() else {
        return Err(Error::Corrupt);
    };
    let result_index = turn
        .item_order
        .iter()
        .position(|id| *id == scope.item_id)
        .ok_or(Error::Corrupt)?;
    if *call_index >= result_index || !call.completed || call.interrupted {
        return Err(Error::Incomplete);
    }
    let ItemKind::ToolCall {
        name,
        provider_call_id: Some(provider_id),
        ..
    } = &call.kind
    else {
        return Err(Error::Corrupt);
    };
    if name != note_tools::REWRITE {
        return Err(Error::InvalidInput);
    }
    let call = ToolCall {
        namespace: None,
        raw_arguments_json: None,
        call_id: provider_id.clone(),
        name: name.clone(),
        arguments: serde_json::from_str(&call.text).map_err(|_| Error::Corrupt)?,
    };
    let expected = note_tools::evaluate(record, &call).map_err(|_| Error::Corrupt)?;
    if result.text != expected {
        return Err(Error::Corrupt);
    }
    serde_json::from_str(&expected).map_err(|_| Error::Corrupt)
}

fn baseline(
    db: &Connection,
    scope: Scope,
    record: &SubmissionRecord,
) -> Result<(i64, bool, Option<ContextText>), Error> {
    let revision: i64 = db
        .query_row(
            "select revision from note_pages where id=?1",
            [scope.page_id.to_string()],
            |r| r.get(0),
        )
        .map_err(|_| Error::Unavailable)?;
    if !(0..=9_007_199_254_740_991).contains(&revision) {
        return Err(Error::Corrupt);
    }
    let source = &record.input.context[0];
    let anchor: Value = serde_json::from_str(source.anchor.as_deref().ok_or(Error::Corrupt)?)
        .map_err(|_| Error::Corrupt)?;
    let current = materialize_in(
        db,
        &NoteContextRequest {
            page_id: scope.page_id,
            expected_revision: revision,
            block_id: anchor["blockId"].as_str().map(str::to_owned),
        },
    );
    match current {
        Ok(current) if current != *source => Ok((revision, false, None)),
        Ok(_) => {
            let full_page = materialize_in(
                db,
                &NoteContextRequest {
                    page_id: scope.page_id,
                    expected_revision: revision,
                    block_id: None,
                },
            )
            .map_err(|error| match error {
                NoteContextError::Budget => Error::Budget,
                _ => Error::Unavailable,
            })?;
            Ok((revision, true, Some(full_page)))
        }
        Err(NoteContextError::Conflict) => Ok((revision, false, None)),
        Err(NoteContextError::Budget) => Err(Error::Budget),
        Err(_) => Err(Error::Unavailable),
    }
}

pub(super) fn candidate_document(
    record: &SubmissionRecord,
    proposal: &Value,
    full_page: &ContextText,
) -> Result<crate::notes::NoteDocument, Error> {
    let edits = proposal["edits"]
        .as_array()
        .ok_or(Error::Corrupt)?
        .iter()
        .map(|edit| {
            Ok(crate::notes::agent_rewrite::InlineRewrite {
                block_id: edit["blockId"].as_str().ok_or(Error::Corrupt)?.to_owned(),
                content: edit["after"].clone(),
            })
        })
        .collect::<Result<Vec<_>, Error>>()?;
    crate::notes::agent_rewrite::candidate(&record.input.context[0], full_page, &edits)
        .map_err(|_| Error::Unavailable)
}
