//! Public application addresses. Parse IDs and local paths are never exposed.
use super::{Entry, Resource};

pub(super) fn address(entry: &Entry) -> Option<String> {
    address_for(entry.resource.as_ref(), entry.identity.anchor.as_deref())
}
pub(crate) fn address_for(resource: Option<&Resource>, anchor: Option<&str>) -> Option<String> {
    if matches!(
        resource,
        Some(Resource::StructuredObject { .. } | Resource::TemplateCatalog { .. })
    ) {
        return anchor
            .and_then(|anchor| serde_json::from_str::<serde_json::Value>(anchor).ok())
            .and_then(|anchor| anchor["sourceUrl"].as_str().map(str::to_owned));
    }
    let Some(resource) = resource else {
        let anchor: serde_json::Value = serde_json::from_str(anchor?).ok()?;
        let path =
            research_agent_core::text_patch::address::parse(anchor["sourceUrl"].as_str()?).ok()?;
        return Some(research_agent_core::text_patch::address::format(&path));
    };
    let path = match resource {
        Resource::Paper {
            resource_id,
            attachment_id,
            ..
        } => {
            let mut path = vec!["library".into(), resource_id.clone()];
            if let Some(file) = attachment_id {
                path.push(file.clone());
            }
            path
        }
        Resource::ManuscriptFile { resource_id, path } => {
            let mut parts = vec!["writings".into(), resource_id.to_string()];
            parts.extend(path.split('/').map(str::to_owned));
            parts
        }
        Resource::Note { resource_id } => vec!["notes".into(), resource_id.to_string()],
        Resource::PaperMetadata { resource_id }
        | Resource::PaperPortrait { resource_id }
        | Resource::PaperGroup { resource_id, .. }
        | Resource::PaperNotes { resource_id, .. } => vec!["library".into(), resource_id.clone()],
        Resource::Manuscript { resource_id, .. } => vec!["writings".into(), resource_id.clone()],
        Resource::NoteGroup { resource_id, .. } => vec!["notes".into(), resource_id.clone()],
        Resource::StructuredObject { .. } | Resource::TemplateCatalog { .. } => return None,
    };
    Some(research_agent_core::text_patch::address::format(&path))
}

#[cfg(test)]
mod tests {
    #[test]
    fn segments_preserve_opaque_ids_and_unicode() {
        assert_eq!(
            research_agent_core::text_patch::address::object(
                research_agent_core::text_patch::address::SourceRoot::Notes,
                &["a/b #引".into()]
            ),
            "source://notes/a%2Fb%20%23%E5%BC%95"
        );
    }
}
