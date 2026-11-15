//! Generation pins template versions; ordinary conversations continue to read live objects.
use crate::{
    agent::context_session::resource_grants,
    structured_resources::{self, ReadResult, TemplateRef},
};
use research_agent_core::submission::SubmissionRecord;
use rusqlite::Connection;
use serde_json::Value;

fn references(record: &SubmissionRecord) -> Result<Vec<(String, TemplateRef)>, String> {
    let mut result = vec![];
    for source in &record.input.context {
        let Some(crate::agent::context_session::sources::Resource::StructuredObject {
            resource_id,
            ..
        }) = resource_grants::resource(source)
        else {
            continue;
        };
        let Some(raw) = source.anchor.as_deref() else {
            continue;
        };
        let anchor: Value =
            serde_json::from_str(raw).map_err(|_| "invalid frozen template grant")?;
        let Some(value) = anchor.get("frozenTemplateRef") else {
            continue;
        };
        let reference: TemplateRef = serde_json::from_value(value.clone())
            .map_err(|_| "invalid frozen template reference")?;
        if reference.resource_id.is_empty() || reference.revision < 1 {
            return Err("invalid frozen template reference".into());
        }
        result.push((resource_id, reference));
    }
    Ok(result)
}

pub(super) fn check_binding(
    db: &Connection,
    record: &SubmissionRecord,
    target: &str,
) -> Result<(), String> {
    for (id, frozen) in references(record)? {
        if id != target {
            continue;
        }
        let resource = structured_resources::get(db, target).map_err(|error| error.to_string())?;
        if resource.kind != "structuredReading"
            || resource.template_ref.as_ref().is_none_or(|current| {
                current.resource_id != frozen.resource_id || current.revision != frozen.revision
            })
        {
            return Err("the adopted template changed after generation started; restart generation using the current template".into());
        }
    }
    Ok(())
}

pub(super) fn read(
    db: &Connection,
    record: &SubmissionRecord,
    id: &str,
    saved: bool,
) -> Result<ReadResult, String> {
    let references = references(record)?
        .into_iter()
        .map(|(_, reference)| reference)
        .filter(|reference| reference.resource_id == id)
        .collect::<Vec<_>>();
    if let Some(reference) = references.first() {
        if references
            .iter()
            .any(|other| other.revision != reference.revision)
        {
            return Err("ambiguous frozen template version".into());
        }
        let mut resource = structured_resources::get_version(db, id, reference.revision)
            .map_err(|error| error.to_string())?;
        if resource.kind != "readingTemplate" {
            return Err("frozen reference is not a reading template".into());
        }
        if let Some(title) = resource
            .content
            .as_ref()
            .and_then(|value| value["name"].as_str())
        {
            resource.title = title.into();
        }
        let text = resource
            .content
            .as_ref()
            .map(serde_json::to_string_pretty)
            .transpose()
            .map_err(|error| error.to_string())?
            .unwrap_or_default();
        return Ok(ReadResult {
            resource,
            text,
            draft: None,
        });
    }
    structured_resources::read_text(db, id, Some(&record.conversation_id.to_string()), saved)
        .map_err(|error| error.to_string())
}
