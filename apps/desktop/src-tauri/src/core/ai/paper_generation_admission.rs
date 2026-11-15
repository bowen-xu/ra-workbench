//! Fixed Core conversation facts and writes in the caller's original transaction.
//! No connection, commit operation or arbitrary SQL escapes this capability.
use super::error::AiError;
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use uuid::Uuid;

pub(crate) type PaperGenerationSource = fn(&Transaction<'_>, &str, Uuid) -> Result<Uuid, AiError>;

pub(crate) struct PaperConversationAdmission<'db> {
    transaction: &'db Connection,
}

impl<'db> PaperConversationAdmission<'db> {
    pub(crate) fn new(transaction: &'db Connection) -> Self {
        Self { transaction }
    }

    pub(crate) fn active_run(&self, task_kind: &str, paper: &str) -> rusqlite::Result<bool> {
        self.transaction.query_row(
            "select exists(select 1 from ai_runs r join ai_scope_items s on s.run_id=r.id
         where s.parent_object_id=?1 and r.task_kind=?2
         and r.status in ('queued','resolving_scope','retrieving','extracting_evidence','generating','validating'))",
            params![paper, task_kind], |row| row.get(0),
        )
    }

    pub(crate) fn unarchived_conversation(
        &self,
        pointer: Option<&str>,
    ) -> rusqlite::Result<Option<String>> {
        self.transaction
            .query_row(
                "select id from agent_conversations where id=?1 and archived=0",
                [pointer],
                |row| row.get(0),
            )
            .optional()
    }

    pub(crate) fn insert_conversation_and_binding(
        &self,
        candidate: Uuid,
        paper: &str,
        title: Option<&str>,
    ) -> rusqlite::Result<()> {
        // No title row preserves the original INSERT SELECT's zero rows. The
        // following binding still executes and keeps its original FK result.
        if let Some(title) = title {
            self.transaction.execute(
                "insert into agent_conversations(id,title) values(?1,?2)",
                params![candidate.to_string(), title],
            )?;
        }
        self.transaction.execute("insert into agent_conversation_bindings(conversation_id,resource_kind,resource_id) values(?1,'paper',?2)", params![candidate.to_string(),paper])?;
        Ok(())
    }
}
