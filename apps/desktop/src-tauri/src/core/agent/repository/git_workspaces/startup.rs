//! Conservative startup reconciliation for the Git worktree registry.
//! Missing paths become recovery candidates; no Git worktree is removed here.

use super::Error;
use rusqlite::{Connection, params};
use std::path::Path;

pub fn mark_missing_on_startup(db: &mut Connection) -> Result<usize, Error> {
    let rows = {
        let mut statement =
            db.prepare("select id,worktree_path from agent_git_workspaces where state='active'")?;
        statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?
    };
    let mut changed = 0;
    let tx = db.transaction()?;
    for (id, path) in rows {
        if !Path::new(&path).is_dir()
            && tx.execute(
                "update agent_git_workspaces set state='recovery_required' where id=?1 and state='active'",
                params![id],
            )? == 1
        {
            changed += 1;
        }
    }
    tx.commit()?;
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn startup_marks_missing_without_deleting_inventory() {
        let mut db = Connection::open_in_memory().unwrap();
        db.execute_batch("create table agent_git_workspaces(id text primary key, worktree_path text, state text);
            insert into agent_git_workspaces values ('missing','/path/that/does/not/exist','active');")
            .unwrap();
        assert_eq!(mark_missing_on_startup(&mut db).unwrap(), 1);
        let state: String = db
            .query_row(
                "select state from agent_git_workspaces where id='missing'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(state, "recovery_required");
    }
}
