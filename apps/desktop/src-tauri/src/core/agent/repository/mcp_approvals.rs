//! Durable, revision-bound MCP approval receipts. Secrets and process details
//! are intentionally absent; changing a server revision invalidates the row.

use rusqlite::{Connection, Transaction, params};
use std::collections::HashSet;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error("MCP approval storage unavailable")]
    Storage,
    #[error("invalid MCP approval")]
    Invalid,
}

impl From<rusqlite::Error> for Error {
    fn from(_: rusqlite::Error) -> Self {
        Self::Storage
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Approval {
    pub server_id: String,
    pub environment_id: String,
    pub revision: String,
}

pub fn upsert(db: &Connection, approval: &Approval) -> Result<(), Error> {
    validate(approval)?;
    db.execute(
        "insert into agent_mcp_approvals(server_id,environment_id,revision) values(?1,?2,?3) on conflict(server_id) do update set environment_id=excluded.environment_id, revision=excluded.revision, approved_at=strftime('%Y-%m-%dT%H:%M:%fZ','now')",
        params![approval.server_id, approval.environment_id, approval.revision],
    )?;
    Ok(())
}

/// Reconcile the durable approval set with one successfully built runtime.
/// The runtime is built before this call, so a discovery/credential failure
/// never revokes the previous approvals. Both removal and replacement commit
/// together after the exact configured server set has been validated.
pub fn reconcile(
    db: &mut Connection,
    configured_server_ids: &[String],
    approvals: &[Approval],
) -> Result<(), Error> {
    if configured_server_ids
        .iter()
        .any(|id| id.trim().is_empty() || id.len() > 256 || id.chars().any(char::is_control))
        || configured_server_ids.iter().collect::<HashSet<_>>().len() != configured_server_ids.len()
    {
        return Err(Error::Invalid);
    }
    let configured = configured_server_ids
        .iter()
        .cloned()
        .collect::<HashSet<_>>();
    let approved = approvals
        .iter()
        .map(|approval| approval.server_id.clone())
        .collect::<HashSet<_>>();
    if approvals.len() != approved.len() || !approved.is_subset(&configured) {
        return Err(Error::Invalid);
    }
    for approval in approvals {
        validate(approval)?;
    }
    let tx = db.transaction()?;
    if configured.is_empty() {
        tx.execute("delete from agent_mcp_approvals", [])?;
    } else {
        for server_id in configured_server_ids {
            if !approved.contains(server_id) {
                tx.execute(
                    "delete from agent_mcp_approvals where server_id=?1",
                    [server_id],
                )?;
            }
        }
        tx.execute(
            &format!(
                "delete from agent_mcp_approvals where server_id not in ({})",
                std::iter::repeat_n("?", configured_server_ids.len())
                    .collect::<Vec<_>>()
                    .join(",")
            ),
            rusqlite::params_from_iter(configured_server_ids),
        )?;
    }
    for approval in approvals {
        upsert_in(&tx, approval)?;
    }
    tx.commit()?;
    Ok(())
}

fn upsert_in(db: &Transaction<'_>, approval: &Approval) -> Result<(), Error> {
    db.execute(
        "insert into agent_mcp_approvals(server_id,environment_id,revision) values(?1,?2,?3) on conflict(server_id) do update set environment_id=excluded.environment_id, revision=excluded.revision, approved_at=strftime('%Y-%m-%dT%H:%M:%fZ','now')",
        params![approval.server_id, approval.environment_id, approval.revision],
    )?;
    Ok(())
}

pub fn read(db: &Connection, server_id: &str) -> Result<Option<Approval>, Error> {
    read_sql(db, server_id).map_err(|cause| cause.domain_or(Error::Storage))
}
pub(crate) fn read_sql(
    db: &Connection,
    server_id: &str,
) -> Result<Option<Approval>, super::SqlRepositoryError<Error>> {
    if server_id.trim().is_empty()
        || server_id.len() > 256
        || server_id.chars().any(char::is_control)
    {
        return Err(super::SqlRepositoryError::Domain(Error::Invalid));
    }
    db.query_row(
        "select server_id,environment_id,revision from agent_mcp_approvals where server_id=?1",
        [server_id],
        |row| {
            Ok(Approval {
                server_id: row.get(0)?,
                environment_id: row.get(1)?,
                revision: row.get(2)?,
            })
        },
    )
    .optional()
    .map_err(Into::into)
}

pub fn revoke(db: &Connection, server_id: &str) -> Result<(), Error> {
    if server_id.trim().is_empty()
        || server_id.len() > 256
        || server_id.chars().any(char::is_control)
    {
        return Err(Error::Invalid);
    }
    db.execute(
        "delete from agent_mcp_approvals where server_id=?1",
        [server_id],
    )?;
    Ok(())
}

fn validate(value: &Approval) -> Result<(), Error> {
    for field in [&value.server_id, &value.environment_id, &value.revision] {
        if field.trim().is_empty() || field.len() > 256 || field.chars().any(char::is_control) {
            return Err(Error::Invalid);
        }
    }
    Ok(())
}

use rusqlite::OptionalExtension;

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Connection {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("create table agent_mcp_approvals (server_id text primary key, environment_id text not null, revision text not null, approved_at text not null default 'now');").unwrap();
        db
    }

    #[test]
    fn upsert_replaces_revision_and_revoke_removes_receipt() {
        let db = db();
        let first = Approval {
            server_id: "literature".into(),
            environment_id: "project".into(),
            revision: "r1".into(),
        };
        upsert(&db, &first).unwrap();
        assert_eq!(read(&db, "literature").unwrap(), Some(first.clone()));
        let second = Approval {
            revision: "r2".into(),
            ..first
        };
        upsert(&db, &second).unwrap();
        assert_eq!(read(&db, "literature").unwrap(), Some(second));
        revoke(&db, "literature").unwrap();
        assert_eq!(read(&db, "literature").unwrap(), None);
    }

    #[test]
    fn rejects_control_characters_without_touching_storage() {
        let db = db();
        assert_eq!(read(&db, "bad\nserver"), Err(Error::Invalid));
        assert_eq!(
            upsert(
                &db,
                &Approval {
                    server_id: "bad\nserver".into(),
                    environment_id: "p".into(),
                    revision: "r".into()
                }
            ),
            Err(Error::Invalid)
        );
    }

    #[test]
    fn reconcile_removes_unselected_and_removed_servers_atomically() {
        let mut db = db();
        let keep = Approval {
            server_id: "keep".into(),
            environment_id: "p".into(),
            revision: "r1".into(),
        };
        let drop = Approval {
            server_id: "drop".into(),
            environment_id: "p".into(),
            revision: "r1".into(),
        };
        upsert(&db, &keep).unwrap();
        upsert(&db, &drop).unwrap();
        reconcile(
            &mut db,
            &["keep".into(), "new".into()],
            std::slice::from_ref(&keep),
        )
        .unwrap();
        assert_eq!(read(&db, "keep").unwrap(), Some(keep));
        assert_eq!(read(&db, "drop").unwrap(), None);
    }

    #[test]
    fn reconcile_empty_configuration_clears_all_receipts() {
        let mut db = db();
        let approval = Approval {
            server_id: "literature".into(),
            environment_id: "project".into(),
            revision: "r1".into(),
        };
        upsert(&db, &approval).unwrap();

        reconcile(&mut db, &[], &[]).unwrap();

        assert_eq!(read(&db, "literature").unwrap(), None);
    }

    #[test]
    fn reconcile_rejects_invalid_sets_before_changing_receipts() {
        let mut db = db();
        let existing = Approval {
            server_id: "existing".into(),
            environment_id: "project".into(),
            revision: "r1".into(),
        };
        upsert(&db, &existing).unwrap();
        let unknown = Approval {
            server_id: "unknown".into(),
            environment_id: "project".into(),
            revision: "r1".into(),
        };

        assert_eq!(
            reconcile(&mut db, &["configured".into()], &[unknown]),
            Err(Error::Invalid)
        );
        assert_eq!(read(&db, "existing").unwrap(), Some(existing));
    }

    #[test]
    fn reconcile_removes_receipts_for_disabled_servers_when_enabled_set_is_passed() {
        let mut db = db();
        let disabled = Approval {
            server_id: "disabled".into(),
            environment_id: "project".into(),
            revision: "r1".into(),
        };
        upsert(&db, &disabled).unwrap();

        // The native command filters the configured list to enabled servers
        // before calling reconciliation, so a disabled server is treated as
        // removed from the active approval domain.
        reconcile(&mut db, &["enabled".into()], &[]).unwrap();

        assert_eq!(read(&db, "disabled").unwrap(), None);
    }
}
