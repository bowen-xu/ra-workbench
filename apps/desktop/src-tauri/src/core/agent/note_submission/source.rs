//! Only host selection varies; preparation and receipt persistence are shared.
use super::Error;
use crate::notes::{agent_context, reading_agent_context};
use research_agent_core::submission::ContextText;
use rusqlite::{Connection, params};
use uuid::Uuid;
#[path = "source/text_view.rs"]
pub(crate) mod text_view;

pub(crate) enum Selection {
    Page(agent_context::NoteContextRequest),
    Reading(reading_agent_context::Request),
}
impl Selection {
    pub fn resource(&self) -> Resource {
        match self {
            Self::Page(request) => Resource::Page(request.page_id),
            Self::Reading(request) => Resource::Reading(request.reading_note_id),
        }
    }
    pub fn validate(&self) -> Result<(), agent_context::NoteContextError> {
        match self {
            Self::Page(request) => agent_context::validate_request(request),
            Self::Reading(request) => reading_agent_context::validate(request),
        }
    }
    pub fn materialize(
        &self,
        db: &Connection,
    ) -> Result<ContextText, agent_context::NoteContextError> {
        match self {
            Self::Page(request) => {
                text_view::project(db, agent_context::materialize_in(db, request)?)
            }
            Self::Reading(request) => {
                text_view::project(db, reading_agent_context::materialize_in(db, request)?)
            }
        }
    }
}

/// The same Core projection is used in the short capture and original commit.
/// A partial selection may also grant the original full read-only file context.
pub(super) fn materialize_contexts(
    db: &Connection,
    selection: &Selection,
    user_input: &[research_agent_core::user_input::UserInput],
) -> Result<Vec<ContextText>, Error> {
    let context = selection.materialize(db).map_err(super::context_error)?;
    let mut contexts = vec![context];
    if let Selection::Page(selected) = selection
        && selected.block_id.is_some()
        && user_input.iter().any(|item| matches!(item, research_agent_core::user_input::UserInput::Mention {target: research_agent_core::user_input::MentionTarget::File {resource_id, ..}, ..} if resource_id == &selected.page_id.to_string()))
    {
        let mut full = text_view::project(
            db,
            agent_context::materialize_in(
                db,
                &agent_context::NoteContextRequest {
                    page_id: selected.page_id,
                    expected_revision: selected.expected_revision,
                    block_id: None,
                },
            )
            .map_err(super::context_error)?,
        )
        .map_err(super::context_error)?;
        if let Some(anchor) = &mut full.anchor {
            let mut value: serde_json::Value =
                serde_json::from_str(anchor).map_err(|_| Error::Unavailable)?;
            value["contextAccess"] = serde_json::json!("readOnly");
            *anchor = value.to_string();
        }
        contexts.push(full);
    }
    Ok(contexts)
}
#[derive(Clone, Copy)]
pub(crate) enum Resource {
    Page(Uuid),
    Reading(Uuid),
}
impl Resource {
    pub fn id(self) -> Uuid {
        match self {
            Self::Page(id) | Self::Reading(id) => id,
        }
    }
    pub fn anchor_key(self) -> &'static str {
        match self {
            Self::Page(_) => "pageId",
            Self::Reading(_) => "readingNoteId",
        }
    }
}
#[derive(Clone, Copy)]
pub(crate) struct SourceReceipt {
    pub conversation_id: Uuid,
    pub request_id: Uuid,
    pub resource: Resource,
}
impl SourceReceipt {
    pub fn validate(self) -> Result<(), Error> {
        if self.conversation_id.is_nil() || self.request_id.is_nil() || self.resource.id().is_nil()
        {
            Err(Error::InvalidInput)
        } else {
            Ok(())
        }
    }
    pub fn check_binding(self, db: &Connection) -> Result<(), Error> {
        self.resource.check_binding(db, self.conversation_id)
    }
}
impl Resource {
    pub fn check_binding(self, db: &Connection, conversation_id: Uuid) -> Result<(), Error> {
        match self {
            Resource::Page(id) => super::check_binding(db, conversation_id, id),
            Resource::Reading(id) => {
                let allowed: bool = db.query_row(
                    "select exists(select 1 from agent_conversation_bindings b
                    join agent_conversations c on c.id=b.conversation_id join reading_note_sources s on s.block_id=b.resource_id join note_pages n on n.id=s.page_id
                    where b.conversation_id=?1 and b.resource_kind='readingNote' and b.resource_id=?2
                    and c.archived=0 and n.status='active')",
                    params![conversation_id.to_string(), id.to_string()], |r| r.get(0),
                ).map_err(|_| Error::Unavailable)?;
                if allowed { Ok(()) } else { Err(Error::Unbound) }
            }
        }
    }
}
