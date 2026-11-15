//! Only lifecycle tags leave the journal; reading activity never opens a writer.
use super::*;

#[derive(Debug, serde::Serialize)]
pub struct ConversationActivity {
    pub id: Uuid,
    pub internal: bool,
}

impl SqliteConversationCatalog {
    pub async fn activity(
        &self,
        binding: Option<ConversationBinding>,
    ) -> Result<Vec<ConversationActivity>, CatalogError> {
        if let Some(binding) = &binding {
            validate_binding(binding)?;
        }
        self.run(move |db| {
            // The journal admits turns serially. Its last committed lifecycle
            // boundary owns activity, including failure, stop and restart recovery.
            let mut query = db.prepare(
                "select c.id,
                    (exists(select 1 from agent_paper_analysis_bindings b where b.conversation_id=c.id)
                     or exists(select 1 from agent_legacy_ai_run_bindings b where b.conversation_id=c.id))
                 from agent_conversations c
                 where c.archived=0
                   and (?1 is null or exists(select 1 from agent_conversation_bindings b
                     where b.conversation_id=c.id and b.resource_kind=?1 and b.resource_id=?2))
                   and (select json_extract(i.payload_json, '$.payload.type') from agent_items i
                     where i.conversation_id=c.id
                       and json_extract(i.payload_json, '$.payload.type') in ('turnStarted', 'turnFinished')
                     order by i.ordinal desc limit 1)='turnStarted'
                 order by c.id"
            ).map_err(db_error)?;
            query.query_map(params![binding.as_ref().map(|b| binding_kind(b.kind)), binding.as_ref().map(|b| b.resource_id.as_str())], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, bool>(1)?))
            }).map_err(db_error)?.map(|row| {
                let (id, internal) = row.map_err(db_error)?;
                Ok(ConversationActivity { id: Uuid::parse_str(&id).map_err(|_| CatalogError::Corrupt)?, internal })
            }).collect()
        }).await
    }
}

#[cfg(test)]
#[path = "activity/tests.rs"]
mod tests;
