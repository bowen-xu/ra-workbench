//! A selected child grant is a ceiling for tools, never a filesystem or shell grant.
use research_agent_core::submission::SubmissionRecord;
use serde_json::Value;
use std::collections::HashSet;

pub(crate) fn writing_scope(record: &SubmissionRecord) -> Option<super::sources::Resource> {
    record.input.context.iter().find_map(|source| {
        let anchor: Value = serde_json::from_str(source.anchor.as_deref()?).ok()?;
        if anchor["supplementalContext"] == true || anchor["contextOwner"]["kind"] != "manuscript" {
            return None;
        }
        let scope: super::sources::Resource =
            serde_json::from_value(anchor["contextScope"].clone()).ok()?;
        match &scope {
            super::sources::Resource::Manuscript { resource_id, .. }
                if anchor["manuscriptId"] == *resource_id
                    && anchor["contextOwner"]["resourceId"] == *resource_id =>
            {
                Some(scope)
            }
            _ => None,
        }
    })
}

pub(crate) fn writing_paths(record: &SubmissionRecord) -> Option<HashSet<String>> {
    let primary = record.input.context.iter().find_map(|source| {
        let anchor = source
            .anchor
            .as_deref()
            .and_then(|value| serde_json::from_str::<Value>(value).ok())?;
        (anchor["supplementalContext"] != true)
            .then(|| anchor["manuscriptId"].as_str().map(str::to_owned))
            .flatten()
    })?;
    let mut configured = false;
    let mut writable = HashSet::new();
    for source in &record.input.context {
        let Some(anchor) = source
            .anchor
            .as_deref()
            .and_then(|value| serde_json::from_str::<Value>(value).ok())
        else {
            continue;
        };
        if anchor["contextOwner"]["kind"] == "manuscript"
            && anchor["contextOwner"]["resourceId"] == primary
        {
            configured = true;
            if anchor["contextAccess"] == "readWrite" && anchor["resource"]["kind"] != "manuscript"
            {
                writable.insert(source.resource_id.clone());
            }
        }
    }
    configured.then_some(writable)
}
pub(crate) fn selected_writing_paths(record: &SubmissionRecord) -> Option<HashSet<String>> {
    writing_paths(record)?;
    Some(
        record
            .input
            .context
            .iter()
            .filter(|source| {
                source
                    .anchor
                    .as_deref()
                    .and_then(|text| serde_json::from_str::<Value>(text).ok())
                    .is_some_and(|anchor| {
                        anchor["contextOwner"]["kind"] == "manuscript"
                            && anchor["supplementalContext"] != true
                            && anchor["resource"]["kind"] != "manuscript"
                    })
            })
            .map(|source| source.resource_id.clone())
            .collect(),
    )
}
