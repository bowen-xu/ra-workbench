//! Internal runtime history capability. App lends its existing connection/guard;
//! consumers receive fixed reads, never the connection or a general SQL API.
//! Original response types remain in the existing model layer.
pub(crate) use super::{error::AiCommandError, model::AiRun};
use super::{error::AiError, run_repository};
use rusqlite::{Connection, OptionalExtension, params};

pub(crate) struct RunHistoryReader<'db> {
    connection: &'db Connection,
}

impl<'db> RunHistoryReader<'db> {
    pub(crate) fn new(connection: &'db Connection) -> Self {
        Self { connection }
    }

    pub(crate) fn latest_for_parent(
        &self,
        task_kind: &str,
        parent_id: &str,
    ) -> Result<Option<AiRun>, AiCommandError> {
        let id: Option<String> = self
            .connection
            .query_row(
                "select r.id from ai_runs r where r.task_kind=?1
         and exists(select 1 from ai_scope_items s where s.run_id=r.id and s.parent_object_id=?2)
         order by r.created_at desc,r.rowid desc limit 1",
                params![task_kind, parent_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(AiError::from)?;
        id.map(|id| run_repository::get_run(self.connection, &id))
            .transpose()
            .map_err(Into::into)
    }
}
