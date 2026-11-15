use std::{
    path::Path,
    sync::{Arc, Mutex},
};

use research_data_runtime::RaftLog;
use rusqlite::{Connection, OptionalExtension, params};
use uuid::Uuid;

use super::DataError;

pub(super) fn log_has_files(path: &Path) -> Result<bool, DataError> {
    if !path.exists() {
        return Ok(false);
    }
    Ok(std::fs::read_dir(path)
        .map_err(|_| DataError::Pair("cannot inspect log directory"))?
        .next()
        .is_some())
}

/// Bootstrap is a recoverable two-phase pair operation. A ready SQL prefix may
/// never silently acquire a replacement empty log, even when its watermark is 0.
pub(super) fn lock_existing_pair(
    db: &Connection,
    path: &Path,
) -> Result<Option<RaftLog>, DataError> {
    let paired: bool = db.query_row(
        "select exists(select 1 from sqlite_master where name='data_runtime_state')",
        [],
        |r| r.get(0),
    )?;
    if !paired {
        return if log_has_files(path)? {
            Err(DataError::Pair("orphan log before SQL migration"))
        } else {
            Ok(None)
        };
    }
    let state: Option<(u32,String,String,u64)> = db.query_row("select schema_version,epoch,bootstrap_state,materialized_position from data_runtime_state where singleton=1",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?;
    let Some((schema, epoch, state, position)) = state else {
        return if log_has_files(path)? {
            Err(DataError::Pair("log exists without SQL pairing"))
        } else {
            Ok(None)
        };
    };
    let epoch = Uuid::parse_str(&epoch).map_err(|_| DataError::Pair("invalid SQL epoch"))?;
    if schema != 1 || epoch.is_nil() {
        return Err(DataError::Pair("unsupported SQL pairing schema or epoch"));
    }
    match state.as_str() {
        "preparing" if position == 0 => {
            // SQL has durably reserved this epoch, but no business admission
            // could precede ready. Preserve that state during paired backup.
            Ok(Some(RaftLog::open_preparing(path, epoch)?))
        }
        "ready" if log_has_files(path)? => Ok(Some(RaftLog::open(path, Some(epoch))?)),
        "ready" => Err(DataError::Pair(
            "paired database log is missing before migration",
        )),
        _ => Err(DataError::Pair("invalid incomplete pair bootstrap")),
    }
}

pub(super) fn open_pair(
    raw: &Arc<Mutex<Connection>>,
    path: &Path,
    locked: Option<RaftLog>,
) -> Result<RaftLog, DataError> {
    open_pair_impl(
        raw,
        path,
        locked,
        #[cfg(test)]
        None,
    )
}
#[cfg(test)]
pub(super) fn open_pair_observed(
    raw: &Arc<Mutex<Connection>>,
    path: &Path,
    counters: research_data_runtime::LogIoCounters,
) -> Result<RaftLog, DataError> {
    open_pair_impl(raw, path, None, Some(counters))
}
fn open_pair_impl(
    raw: &Arc<Mutex<Connection>>,
    path: &Path,
    locked: Option<RaftLog>,
    #[cfg(test)] counters: Option<research_data_runtime::LogIoCounters>,
) -> Result<RaftLog, DataError> {
    let db = raw.lock().map_err(|_| DataError::Unavailable)?;
    let state: Option<(String, u64, String)> = db.query_row(
        "select epoch,materialized_position,bootstrap_state from data_runtime_state where singleton=1",
        [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)),
    ).optional()?;
    let has_log = log_has_files(path)?;
    let (epoch, preparing) = match state {
        None => {
            if has_log {
                return Err(DataError::Pair("log exists without its SQL epoch"));
            }
            let epoch = Uuid::new_v4();
            db.execute(
                "insert into data_runtime_state values(1,1,?1,0,'preparing')",
                [epoch.to_string()],
            )?;
            (epoch, true)
        }
        Some((epoch, position, state)) => {
            let epoch =
                Uuid::parse_str(&epoch).map_err(|_| DataError::Pair("invalid SQL epoch"))?;
            match state.as_str() {
                "ready" if has_log => (epoch, false),
                "preparing" if position == 0 => (epoch, true),
                _ => return Err(DataError::Pair("missing log or incomplete pair bootstrap")),
            }
        }
    };
    let log = match locked {
        Some(log) if log.head()?.epoch == epoch => log,
        Some(_) => return Err(DataError::Pair("startup log epoch changed")),
        None => open_log(
            path,
            epoch,
            #[cfg(test)]
            counters,
        )?,
    };
    if preparing {
        if log.head()?.accepted_position != 0 {
            return Err(DataError::Pair("bootstrap log contains business records"));
        }
        db.execute(
            "update data_runtime_state set bootstrap_state='ready' where singleton=1 and epoch=?1",
            params![epoch.to_string()],
        )?;
    }
    let coalescing: bool = db.query_row(
        "select coalesce_text from data_projection_control where singleton=1",
        [],
        |r| r.get(0),
    )?;
    if coalescing {
        return Err(DataError::Pair("unfinished projection control"));
    }
    Ok(log)
}
fn open_log(
    path: &Path,
    epoch: Uuid,
    #[cfg(test)] counters: Option<research_data_runtime::LogIoCounters>,
) -> Result<RaftLog, DataError> {
    #[cfg(test)]
    if let Some(counters) = counters {
        return Ok(RaftLog::open_config_with_io(
            research_data_runtime::LogConfig::desktop(path),
            Some(epoch),
            counters,
        )?);
    }
    Ok(RaftLog::open(path, Some(epoch))?)
}
