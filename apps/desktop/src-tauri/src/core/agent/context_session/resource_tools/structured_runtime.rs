//! Core-only authorization and the original read-proof/result projection.
use super::*;
use crate::ai::structured_resource_validation::{Failure, FailureCause};

pub(crate) struct ProofFailure {
    pub failure: Failure<Error>,
    pub confirmed: bool,
}

impl ResourceTools {
    pub(super) async fn runtime_structured_read(&self, call: &ToolCall) -> Option<ToolOutput> {
        if call.name != "research_read_source"
            || call.arguments.get("view").and_then(Value::as_str) != Some("structured")
            || self.store.paper_reading_source().is_some()
        {
            return None;
        }
        let Some(binding) = self.store.native_note_trace().cloned() else {
            return Some(crate::runtime_plugins::literature_trace::service::SourceReadError::missing_binding().into_output(&self.record,call));
        };
        let result = binding
            .service
            .read_structured_source(
                self.store.clone(),
                binding.package_key,
                self.record.clone(),
                call.clone(),
            )
            .await;
        Some(match result {
            Ok(value) => ToolOutput {
                content: value.to_string(),
                is_error: false,
            },
            Err(error) => error.into_output(&self.record, call),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AuthorizedStructuredRead {
    pub paper_id: String,
    pub attachment_id: String,
    pub page_index: Option<u32>,
    pub scopes: Vec<(String, Option<u32>)>,
    identity: ContextText,
    name: String,
    source_url: Option<String>,
    file_id: Option<String>,
}

pub(crate) fn capture(
    store: &AppStore,
    record: &SubmissionRecord,
    db: &Connection,
    call: &ToolCall,
    scope: Option<crate::agent::writing_resources::OwnedWritingScope>,
) -> Result<AuthorizedStructuredRead, Error> {
    let args: Read =
        serde_json::from_value(call.arguments.clone()).map_err(|_| Error::InvalidInput)?;
    if call.name != "research_read_source" || args.view.as_deref() != Some("structured") {
        return Err(Error::InvalidInput);
    }
    let tools = ResourceTools {
        store: store.clone(),
        record: record.clone(),
        scope,
    };
    let entries = tools.selected_entries(db, call)?;
    let entry = file_handle::find(&entries, args.source_id.as_deref(), args.file_id.as_deref())?;
    let Some(Resource::Paper {
        resource_id: paper_id,
        attachment_id: Some(attachment_id),
        page_index,
    }) = &entry.resource
    else {
        return Err(Error::InvalidInput);
    };
    let mut scopes = tools
        .entries(db)?
        .into_iter()
        .filter_map(|entry| match entry.resource {
            Some(Resource::Paper {
                resource_id,
                attachment_id: Some(attachment_id),
                page_index,
            }) if resource_id == *paper_id => Some((attachment_id, page_index)),
            _ => None,
        })
        .collect::<Vec<_>>();
    scopes.sort();
    scopes.dedup();
    Ok(AuthorizedStructuredRead {
        paper_id: paper_id.clone(),
        attachment_id: attachment_id.clone(),
        page_index: *page_index,
        scopes,
        identity: entry.identity.clone(),
        name: discovery::name(entry),
        source_url: source_link::address(entry),
        file_id: file_handle::id(entry).map(str::to_owned),
    })
}

/// Called only after the owned operation rechecks the captured authorization and
/// source inputs. This remains the original autocommit proof write, not a new
/// transaction spanning snapshot creation and the tool's acknowledgement.
pub(crate) fn publish(
    db: &Connection,
    record: &SubmissionRecord,
    call: &ToolCall,
    authorized: &AuthorizedStructuredRead,
    snapshot: &str,
    value: Value,
) -> Result<Value, ProofFailure> {
    let args: Read = serde_json::from_value(call.arguments.clone()).map_err(|_| ProofFailure {
        failure: Failure::original(Error::InvalidInput),
        confirmed: false,
    })?;
    let proof = json!({"kind":"structuredEvidenceRead","paperId":authorized.paper_id,"snapshotId":snapshot,"scopes":authorized.scopes});
    let path = research_agent_core::text_patch::address::object(
        research_agent_core::text_patch::address::SourceRoot::Library,
        std::slice::from_ref(&authorized.paper_id),
    );
    db.execute("insert into agent_text_read_scopes(conversation_id,scope_request_id,path,version,snapshot_json) values(?1,?2,?3,?4,?5) on conflict(conversation_id,scope_request_id,path) do update set version=excluded.version,snapshot_json=excluded.snapshot_json",params![record.conversation_id.to_string(),record.request_id.to_string(),path,snapshot,proof.to_string()]).map_err(|error|ProofFailure {
        failure:Failure {error:Error::Unavailable,cause:FailureCause::SourceDatabase(Box::new(crate::runtime_plugins::native::SafeDatabaseError::sql("source-proof",&error)))},confirmed:false,
    })?;
    let mut source = authorized.identity.clone();
    source.text = value.to_string();
    source.revision = snapshot.into();
    let mut record = record.clone();
    record.input.context = vec![source];
    let mut request = call.clone();
    request.arguments = json!({"sourceId":authorized.identity.source_id,"startChar":args.start_char,"maxChars":args.max_chars});
    source_tools::evaluate(&record, &request)
        .map(without_versions)
        .map(|mut result| {
            result["name"] = json!(authorized.name);
            if let Some(address) = &authorized.source_url {
                result["sourceUrl"] = json!(address);
            }
            if let Some(id) = &authorized.file_id {
                result["fileId"] = json!(id);
            }
            result
        })
        .map_err(|_| ProofFailure {
            failure: Failure::original(Error::InvalidInput),
            confirmed: true,
        })
}
