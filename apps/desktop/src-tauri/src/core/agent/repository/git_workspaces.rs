//! Durable inventory for application-created Git worktrees.
//!
//! This follows the existing execution-workspace rule: database state records
//! ownership and recovery uncertainty, while the native Git adapter performs
//! filesystem/process verification before any removal.

use rusqlite::{Connection, OptionalExtension, Transaction, params};
use uuid::Uuid;

#[path = "git_workspaces/startup.rs"]
pub mod startup;

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("git worktree metadata conflict")]
    Conflict,
    #[error("git worktree metadata unavailable")]
    Storage,
}

impl From<rusqlite::Error> for Error {
    fn from(_: rusqlite::Error) -> Self {
        Self::Storage
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    pub id: Uuid,
    pub manuscript_id: Uuid,
    pub repository_root: String,
    pub worktree_path: String,
    pub revision: String,
    pub state: State,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Active,
    Released,
    RecoveryRequired,
}

impl State {
    fn parse(value: &str) -> Result<Self, Error> {
        match value {
            "active" => Ok(Self::Active),
            "released" => Ok(Self::Released),
            "recovery_required" => Ok(Self::RecoveryRequired),
            _ => Err(Error::Conflict),
        }
    }
}

pub fn insert_in(tx: &Transaction<'_>, record: &Record) -> Result<(), Error> {
    if record.id.is_nil()
        || record.manuscript_id.is_nil()
        || record.repository_root.is_empty()
        || record.worktree_path.is_empty()
        || record.revision.is_empty()
        || record.state != State::Active
    {
        return Err(Error::Conflict);
    }
    let state = "active";
    tx.execute(
        "insert into agent_git_workspaces(id,manuscript_id,repository_root,worktree_path,revision,state) values(?1,?2,?3,?4,?5,?6)",
        params![
            record.id.to_string(),
            record.manuscript_id.to_string(),
            record.repository_root,
            record.worktree_path,
            record.revision,
            state
        ],
    )?;
    Ok(())
}

pub fn read(db: &Connection, manuscript_id: Uuid, id: Uuid) -> Result<Option<Record>, Error> {
    if manuscript_id.is_nil() || id.is_nil() {
        return Err(Error::Conflict);
    }
    db.query_row(
        "select manuscript_id,repository_root,worktree_path,revision,state from agent_git_workspaces where id=?1 and manuscript_id=?2",
        params![id.to_string(), manuscript_id.to_string()],
        |row| {
            let manuscript = Uuid::parse_str(&row.get::<_, String>(0)?).map_err(|_| rusqlite::Error::InvalidQuery)?;
            let state = State::parse(&row.get::<_, String>(4)?).map_err(|_| rusqlite::Error::InvalidQuery)?;
            Ok(Record {
                id,
                manuscript_id: manuscript,
                repository_root: row.get(1)?,
                worktree_path: row.get(2)?,
                revision: row.get(3)?,
                state,
            })
        },
    )
    .optional()
    .map_err(Into::into)
}

pub fn list(db: &Connection, manuscript_id: Uuid) -> Result<Vec<Record>, Error> {
    if manuscript_id.is_nil() {
        return Err(Error::Conflict);
    }
    let mut statement = db.prepare(
        "select id,repository_root,worktree_path,revision,state from agent_git_workspaces where manuscript_id=?1 order by created_at desc,id desc limit 200",
    )?;
    let rows = statement.query_map([manuscript_id.to_string()], |row| {
        let id = Uuid::parse_str(&row.get::<_, String>(0)?)
            .map_err(|_| rusqlite::Error::InvalidQuery)?;
        let state =
            State::parse(&row.get::<_, String>(4)?).map_err(|_| rusqlite::Error::InvalidQuery)?;
        Ok(Record {
            id,
            manuscript_id,
            repository_root: row.get(1)?,
            worktree_path: row.get(2)?,
            revision: row.get(3)?,
            state,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

pub fn mark_recovery_required_in(tx: &Transaction<'_>, id: Uuid) -> Result<(), Error> {
    if id.is_nil()
        || tx.execute(
            "update agent_git_workspaces set state='recovery_required' where id=?1 and state='active'",
            [id.to_string()],
        )? != 1
    {
        return Err(Error::Conflict);
    }
    Ok(())
}

pub fn release_in(tx: &Transaction<'_>, id: Uuid) -> Result<(), Error> {
    if id.is_nil() {
        return Err(Error::Conflict);
    }
    let changed = tx.execute(
        "update agent_git_workspaces set state='released',released_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') where id=?1 and state in ('active','recovery_required')",
        [id.to_string()],
    )?;
    if changed == 0 {
        let exists: bool = tx.query_row(
            "select exists(select 1 from agent_git_workspaces where id=?1 and state='released')",
            [id.to_string()],
            |row| row.get(0),
        )?;
        if !exists {
            return Err(Error::Conflict);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle_is_idempotent_but_does_not_hide_unknown_ids() {
        let mut db = Connection::open_in_memory().unwrap();
        db.execute_batch(
            "create table agent_git_workspaces(id text primary key, manuscript_id text not null, repository_root text not null, worktree_path text not null unique, revision text not null, state text not null, created_at text not null default 'now', released_at text);",
        )
        .unwrap();
        let manuscript = Uuid::new_v4();
        let id = Uuid::new_v4();
        let record = Record {
            id,
            manuscript_id: manuscript,
            repository_root: "/repo".into(),
            worktree_path: "/worktree".into(),
            revision: "HEAD".into(),
            state: State::Active,
        };
        let tx = db.transaction().unwrap();
        insert_in(&tx, &record).unwrap();
        tx.commit().unwrap();
        assert_eq!(read(&db, manuscript, id).unwrap().unwrap(), record);
        let tx = db.transaction().unwrap();
        release_in(&tx, id).unwrap();
        release_in(&tx, id).unwrap();
        tx.commit().unwrap();
        assert_eq!(
            read(&db, manuscript, id).unwrap().unwrap().state,
            State::Released
        );
        let tx = db.transaction().unwrap();
        assert_eq!(release_in(&tx, Uuid::new_v4()), Err(Error::Conflict));
    }
}
