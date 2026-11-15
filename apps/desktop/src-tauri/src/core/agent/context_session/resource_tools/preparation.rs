//! Resolve authorized PDF bodies before reading; directory discovery stays cheap.
use super::*;
impl ResourceTools {
    pub(super) fn selected_entries(
        &self,
        db: &Connection,
        call: &ToolCall,
    ) -> Result<Vec<Entry>, Error> {
        let active: bool = db.query_row("select exists(select 1 from agent_conversations where id=?1 and archived=0) and not exists(select 1 from agent_items where conversation_id=?1 and json_extract(payload_json,'$.payload.turnId')=?2 and json_extract(payload_json,'$.payload.type')='turnFinished')", params![self.record.conversation_id.to_string(),self.record.turn_id.to_string()], |row| row.get(0)).map_err(|_| Error::Unavailable)?;
        if !active {
            return Err(Error::ConversationUnavailable);
        }
        let mut entries = self.entries(db)?;
        match call.name.as_str() {
            "research_read_source" => {
                let args: Read = serde_json::from_value(call.arguments.clone())
                    .map_err(|_| Error::InvalidInput)?;
                let entry = file_handle::find(
                    &entries,
                    args.source_id.as_deref(),
                    args.file_id.as_deref(),
                )?
                .clone();
                if args.view.as_deref() == Some("structured") {
                    let Some(Resource::Paper {
                        resource_id: paper, ..
                    }) = &entry.resource
                    else {
                        return Err(Error::InvalidInput);
                    };
                    entries.retain(|candidate|matches!(&candidate.resource,Some(Resource::Paper{resource_id,..}) if resource_id==paper));
                } else {
                    entries = vec![entry];
                }
            }
            "research_search_sources" => {
                let args: Search = serde_json::from_value(call.arguments.clone())
                    .map_err(|_| Error::InvalidInput)?;
                discovery::filter(
                    self,
                    &mut entries,
                    &discovery::Filters {
                        resource_id: args.resource_id,
                        name_query: args.name_query,
                        path_glob: args.path_glob,
                        resource_type: args.resource_type,
                    },
                )?;
                if args.source_id.is_some() || args.file_id.is_some() {
                    let entry = file_handle::find(
                        &entries,
                        args.source_id.as_deref(),
                        args.file_id.as_deref(),
                    )?
                    .clone();
                    entries = vec![entry];
                }
            }
            _ => {}
        }
        Ok(entries)
    }
    pub(super) async fn prepare_content(&self, call: &ToolCall) -> Result<(), Error> {
        if !matches!(
            call.name.as_str(),
            "research_read_source" | "research_search_sources"
        ) {
            return Ok(());
        }
        let tools = self.clone();
        let request = call.clone();
        let pdfs = tokio::task::spawn_blocking(move || {
            let db = tools.store.connection().map_err(|_| Error::Unavailable)?;
            let entries = tools.selected_entries(&db, &request)?;
            let mut pdfs = Vec::new();
            for entry in entries {
                if let Some(Resource::Paper {resource_id, attachment_id, ..}) = entry.resource {
                    let attachment: String = db.query_row("select a.id from attachments a join bibliographic_items i on i.id=a.item_id where i.id=?1 and i.trashed_at is null and a.attachment_kind='pdf' and (?2 is null or a.id=?2) order by a.created_at,a.id limit 1", params![resource_id,attachment_id], |row| row.get(0)).map_err(|_| Error::SourceUnavailable)?;
                    if !pdfs.contains(&(resource_id.clone(), attachment.clone())) { pdfs.push((resource_id,attachment)); }
                }
            }
            Ok::<_,Error>(pdfs)
        }).await.map_err(|_| Error::Unavailable)??;
        for (item, attachment) in pdfs {
            // Recheck the live scope after every await and before starting work.
            {
                let db = self.store.connection().map_err(|_| Error::Unavailable)?;
                let allowed = self.selected_entries(&db, call)?.iter().any(|entry| matches!(&entry.resource, Some(Resource::Paper {resource_id, attachment_id,..}) if resource_id == &item && attachment_id.as_ref().is_none_or(|id| id == &attachment)));
                if !allowed {
                    return Err(Error::SourceUnavailable);
                }
            }
            let Some(preparation) = self.store.docling.get() else {
                // Unit fixtures without a desktop runtime can still read saved parses.
                continue;
            };
            let run = preparation
                .ensure(&self.store, &item, &attachment)
                .await
                .map_err(|_| Error::PaperParseFailed)?;
            if !matches!(
                run.summary.parse_status.as_str(),
                "success" | "partial_success"
            ) || run.json_content.as_ref().is_none_or(Value::is_null)
            {
                return Err(Error::PaperParseFailed);
            }
        }
        // evaluate() re-resolves authorization and exact attachment hash after waiting.
        Ok(())
    }
}
