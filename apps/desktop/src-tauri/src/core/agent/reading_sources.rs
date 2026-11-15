//! Read-only restoration of paper source geometry from a turn's frozen snapshot.
use crate::{
    ai::{paper_context::agent_preview, structured_resource_validation::Failure},
    storage::AppStore,
};
use research_agent_core::submission::{
    ContextSourceKind, SubmissionImageReference, SubmissionInput,
};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use tauri::State;
use uuid::Uuid;

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Request {
    conversation_id: Uuid,
    paper_id: String,
    turn_id: Uuid,
}

#[derive(Debug, PartialEq, Eq, Serialize, thiserror::Error)]
#[serde(rename_all = "camelCase")]
pub enum Error {
    #[error("invalid paper source request")]
    InvalidInput,
    #[error("paper turn is not bound to this paper")]
    Unbound,
    #[error("paper turn source snapshot was not found")]
    NotFound,
    #[error("paper turn source snapshot changed")]
    Conflict,
    #[error("paper turn sources are unavailable")]
    Unavailable,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Response {
    schema_version: u8,
    conversation_id: Uuid,
    paper_id: String,
    turn_id: Uuid,
    sources: Vec<agent_preview::SourceLocation>,
    retrieval_trace_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
struct Scope {
    request_id: Uuid,
    snapshot_id: String,
    context_hash: String,
    input: SubmissionInput,
    input_json: String,
    images: Vec<SubmissionImageReference>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SourceCapture {
    scope: Scope,
    allowed: HashSet<String>,
    image_source_rows: Vec<String>,
    retrieval_trace_id: Option<String>,
}
impl SourceCapture {
    pub(crate) fn snapshot_request(&self, request: &Request) -> agent_preview::Request {
        agent_preview::Request {
            paper_id: request.paper_id.clone(),
            snapshot_id: self.scope.snapshot_id.clone(),
            expected_context_hash: self.scope.context_hash.clone(),
        }
    }
}

#[tauri::command]
pub(crate) async fn agent_read_paper_turn_sources(
    store: State<'_, AppStore>,
    request: Request,
) -> Result<Response, serde_json::Value> {
    read(store.inner(), request).await
}

pub(crate) async fn read(
    store: &AppStore,
    request: Request,
) -> Result<Response, serde_json::Value> {
    validate_request(&request).map_err(|error| serde_json::json!(error))?;
    if let Some(binding) = store.native_note_trace() {
        return binding
            .service
            .read_paper_turn_sources(store.clone(), binding.package_key.clone(), request.clone())
            .await
            .map_err(|mut error| {
                if error.is_object() {
                    error["conversationId"] = serde_json::json!(request.conversation_id);
                    error["turnId"] = serde_json::json!(request.turn_id);
                    error["paperId"] = serde_json::json!(request.paper_id);
                }
                error
            });
    }
    let store = store.clone();
    tokio::task::spawn_blocking(move || -> Result<Response, Error> {
        let db = store.connection().map_err(|_| Error::Unavailable)?;
        let capture = capture_sources(&db, &request).map_err(|failure| failure.error)?;
        let source = store
            .require_paper_reading_source()
            .map_err(|_| Error::Unavailable)?;
        let sources = agent_preview::materialize_source_locations(
            source,
            &db,
            &request.paper_id,
            &capture.scope.snapshot_id,
            &capture.scope.context_hash,
            &capture.allowed,
        )
        .map_err(preview_error)?;
        Ok(source_response(request, capture, sources))
    })
    .await
    .map_err(|_| serde_json::json!(Error::Unavailable))?
    .map_err(|error| serde_json::json!(error))
}

/// The original Core authorization, submission and image-ID reads, in order.
pub(crate) fn capture_sources(
    db: &Connection,
    request: &Request,
) -> Result<SourceCapture, Failure<Error>> {
    let scope = read_scope(db, request)?;
    let (mut allowed, retrieval_trace_id) =
        validate_input(&scope, request).map_err(Failure::original)?;
    let mut statement = db.prepare(
        "select source_id from agent_submission_images where conversation_id=?1 and request_id=?2",
    ).map_err(capture_sql_error)?;
    let rows = statement
        .query_map(
            params![
                request.conversation_id.to_string(),
                scope.request_id.to_string()
            ],
            |row| row.get::<_, String>(0),
        )
        .map_err(capture_sql_error)?;
    let mut image_source_rows = Vec::new();
    for row in rows {
        let id = row.map_err(capture_sql_error)?;
        allowed.insert(id.clone());
        image_source_rows.push(id);
    }
    Ok(SourceCapture {
        scope,
        allowed,
        image_source_rows,
        retrieval_trace_id,
    })
}

pub(crate) fn validate_capture_ids(capture: &SourceCapture) -> Result<(), Error> {
    agent_preview::validate_source_location_ids(&capture.allowed).map_err(preview_error)
}

pub(crate) fn complete_sources(
    request: Request,
    capture: SourceCapture,
    preview: agent_preview::Preview,
) -> Result<Response, Error> {
    let sources = agent_preview::source_locations_from_preview(preview, &capture.allowed)
        .map_err(preview_error)?;
    Ok(source_response(request, capture, sources))
}

fn source_response(
    request: Request,
    capture: SourceCapture,
    sources: Vec<agent_preview::SourceLocation>,
) -> Response {
    Response {
        schema_version: 1,
        conversation_id: request.conversation_id,
        paper_id: request.paper_id,
        turn_id: request.turn_id,
        sources,
        retrieval_trace_id: capture.retrieval_trace_id,
    }
}

pub(crate) fn validate_request(request: &Request) -> Result<(), Error> {
    if request.conversation_id.is_nil()
        || request.turn_id.is_nil()
        || request.paper_id.trim().is_empty()
        || request.paper_id.len() > 1024
        || request.paper_id.chars().any(char::is_control)
    {
        return Err(Error::InvalidInput);
    }
    Ok(())
}

fn capture_sql_error(error: rusqlite::Error) -> Failure<Error> {
    Failure::sql(Error::Unavailable, "source-read", &error)
}

fn read_scope(db: &Connection, request: &Request) -> Result<Scope, Failure<Error>> {
    let binding: bool = db
        .query_row(
            "select exists(select 1 from agent_conversation_bindings b join bibliographic_items p on p.id=b.resource_id where b.conversation_id=?1 and b.resource_kind='paper' and b.resource_id=?2 and p.trashed_at is null)",
            params![request.conversation_id.to_string(), request.paper_id],
            |row| row.get(0),
        )
        .map_err(capture_sql_error)?;
    if !binding {
        return Err(Failure::original(Error::Unbound));
    }
    let raw: Option<(String, String, String, String)> = db
        .query_row(
            "select s.request_id,p.paper_context_snapshot_id,p.context_hash,s.input_json
             from agent_submissions s join agent_paper_submission_scopes p
             on p.conversation_id=s.conversation_id and p.request_id=s.request_id and p.turn_id=s.turn_id
             where s.conversation_id=?1 and s.turn_id=?2 and p.paper_id=?3 and s.schema_version=1",
            params![request.conversation_id.to_string(), request.turn_id.to_string(), request.paper_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()
        .map_err(capture_sql_error)?;
    let (request_id, snapshot_id, context_hash, input_json) =
        raw.ok_or_else(|| Failure::original(Error::NotFound))?;
    let request_id =
        Uuid::parse_str(&request_id).map_err(|_| Failure::original(Error::Unavailable))?;
    let images = super::repository::submissions::read_image_manifest(
        db,
        request.conversation_id,
        request_id,
    )
    .map_err(|_| Failure::original(Error::Unavailable))?;
    Ok(Scope {
        request_id,
        snapshot_id,
        context_hash,
        input: serde_json::from_str(&input_json)
            .map_err(|_| Failure::original(Error::Unavailable))?,
        input_json,
        images,
    })
}

fn validate_input(
    scope: &Scope,
    request: &Request,
) -> Result<(HashSet<String>, Option<String>), Error> {
    scope
        .input
        .validate_with_image_references(&scope.images)
        .map_err(|_| Error::Unavailable)?;
    let mut sources = HashSet::new();
    let mut retrieval_trace_id = None;
    for source in &scope.input.context {
        let anchor: serde_json::Value =
            serde_json::from_str(source.anchor.as_deref().ok_or(Error::Unavailable)?)
                .map_err(|_| Error::Unavailable)?;
        let same_scope = source.resource_id == request.paper_id
            && anchor.get("paperId").and_then(serde_json::Value::as_str)
                == Some(request.paper_id.as_str())
            && anchor.get("snapshotId").and_then(serde_json::Value::as_str)
                == Some(scope.snapshot_id.as_str());
        match source.kind {
            ContextSourceKind::Paper => {
                if source.revision != scope.context_hash
                    || !same_scope
                    || !sources.insert(source.source_id.clone())
                {
                    return Err(Error::Unbound);
                }
            }
            ContextSourceKind::Retrieval => {
                let trace_id = anchor
                    .get("traceId")
                    .and_then(serde_json::Value::as_str)
                    .ok_or(Error::Unbound)?;
                if !same_scope
                    || source.revision != format!("trace:{trace_id}")
                    || source.source_id != format!("retrieval:{trace_id}")
                    || retrieval_trace_id.replace(trace_id.to_owned()).is_some()
                {
                    return Err(Error::Unbound);
                }
            }
            _ => return Err(Error::Unbound),
        }
    }
    Ok((sources, retrieval_trace_id))
}

impl From<agent_preview::Error> for Error {
    fn from(error: agent_preview::Error) -> Self {
        preview_error(error)
    }
}

pub(crate) fn preview_error(error: agent_preview::Error) -> Error {
    match error {
        agent_preview::Error::InvalidInput => Error::InvalidInput,
        agent_preview::Error::Conflict => Error::Conflict,
        agent_preview::Error::Unavailable
        | agent_preview::Error::Corrupt
        | agent_preview::Error::Budget => Error::Unavailable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use research_agent_core::{
        protocol::ModelIdentity,
        submission::{ContextSourceKind, ContextText},
    };

    fn scope(context: Vec<ContextText>) -> Scope {
        Scope {
            input_json: String::new(),
            images: vec![],
            request_id: Uuid::new_v4(),
            snapshot_id: "snapshot-1".to_owned(),
            context_hash: "hash-1".to_owned(),
            input: SubmissionInput {
                remote_compaction_v2: None,
                user_input: vec![],
                skill_instructions: vec![],
                text: "question".to_owned(),
                model: ModelIdentity {
                    provider_id: "test".to_owned(),
                    model_id: "test".to_owned(),
                    revision: None,
                },
                reasoning_effort: None,
                service_tier: None,
                context,
                instructions: Vec::new(),
                instruction_configs: Vec::new(),
                skills: Vec::new(),
            },
        }
    }

    fn request() -> Request {
        Request {
            conversation_id: Uuid::new_v4(),
            paper_id: "paper-1".to_owned(),
            turn_id: Uuid::new_v4(),
        }
    }

    #[test]
    fn accepts_one_retrieval_trace_alongside_paper_sources() {
        let request = request();
        let context = vec![ContextText {
            source_id: "retrieval:trace-1".to_owned(),
            kind: ContextSourceKind::Retrieval,
            resource_id: "paper-1".to_owned(),
            revision: "trace:trace-1".to_owned(),
            anchor: Some(serde_json::json!({ "paperId": "paper-1", "snapshotId": "snapshot-1", "traceId": "trace-1" }).to_string()),
            text: "retrieved context".to_owned(),
        }];
        let (sources, trace) = validate_input(&scope(context), &request).unwrap();
        assert!(sources.is_empty());
        assert_eq!(trace.as_deref(), Some("trace-1"));
    }

    #[test]
    fn rejects_retrieval_from_another_snapshot() {
        let request = request();
        let context = vec![ContextText {
            source_id: "retrieval:trace-1".to_owned(),
            kind: ContextSourceKind::Retrieval,
            resource_id: "paper-1".to_owned(),
            revision: "trace:trace-1".to_owned(),
            anchor: Some(serde_json::json!({ "paperId": "paper-1", "snapshotId": "other", "traceId": "trace-1" }).to_string()),
            text: "retrieved context".to_owned(),
        }];
        assert_eq!(
            validate_input(&scope(context), &request),
            Err(Error::Unbound)
        );
    }
}
