//! New submissions disclose authoring text; stored legacy submissions stay intact.
use super::*;
use research_agent_core::submission::ContextText;
use serde_json::{Value, json};
pub(crate) fn project(
    db: &Connection,
    mut source: ContextText,
) -> Result<ContextText, agent_context::NoteContextError> {
    let enabled:bool=db.query_row("select exists(select 1 from sqlite_master where type='table' and name='agent_text_tool_contracts')",[],|row|row.get(0)).map_err(|_|agent_context::NoteContextError::Unavailable)?;
    if !enabled {
        return Ok(source);
    }
    let value: Value = serde_json::from_str(&source.text)
        .map_err(|_| agent_context::NoteContextError::Unavailable)?;
    let body = if value["content"].is_object() {
        &value["content"]
    } else {
        &value
    };
    let page = body["pageId"]
        .as_str()
        .ok_or(agent_context::NoteContextError::Unavailable)?;
    let title = body["title"]
        .as_str()
        .ok_or(agent_context::NoteContextError::Unavailable)?;
    let document: crate::notes::NoteDocument =
        serde_json::from_value(json!({"schemaVersion":1,"pageId":page,"blocks":body["blocks"]}))
            .map_err(|_| agent_context::NoteContextError::Unavailable)?;
    let mut anchor: Value = serde_json::from_str(
        source
            .anchor
            .as_deref()
            .ok_or(agent_context::NoteContextError::Unavailable)?,
    )
    .map_err(|_| agent_context::NoteContextError::Unavailable)?;
    anchor["pageId"] = json!(page);
    anchor["label"] = json!(title);
    anchor["sourceUrl"] = json!(research_agent_core::text_patch::address::object(
        research_agent_core::text_patch::address::SourceRoot::Notes,
        &[page.to_owned()]
    ));
    anchor["textViewVersion"] = json!(crate::notes::text_view::version());
    anchor["textFileScope"] = json!(if anchor["blockId"].is_null() {
        "full"
    } else {
        "selection"
    });
    if value["source"].is_object() {
        anchor["readOnlySource"] = json!({"paperId":value["source"]["paperId"],"attachmentId":value["source"]["attachmentId"],"pageIndex":value["source"]["pageIndex"],"quoteExact":value["source"]["quoteExact"]});
    }
    source.anchor = Some(anchor.to_string());
    source.text = crate::notes::text_view::render(&document, title)
        .map_err(|_| agent_context::NoteContextError::Unavailable)?;
    Ok(source)
}
