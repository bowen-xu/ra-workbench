//! Durable scheduling metadata for bounded child tasks. Conversation events
//! remain the source of truth for content and outcomes.

use research_agent_core::subtasks::SubtaskSpec;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use std::collections::HashSet;
use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Running,
    Completed,
    Canceled,
    Failed,
    RecoveryRequired,
}
impl Status {
    fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Canceled => "canceled",
            Self::Failed => "failed",
            Self::RecoveryRequired => "recovery_required",
        }
    }
}
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error("subtask storage unavailable")]
    Storage,
    #[error("invalid subtask record")]
    Invalid,
}
impl From<rusqlite::Error> for Error {
    fn from(_: rusqlite::Error) -> Self {
        Self::Storage
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    pub child_conversation_id: Uuid,
    pub parent_conversation_id: Uuid,
    pub parent_context_snapshot_id: Uuid,
    pub child_context_snapshot_id: Uuid,
    pub depth: u8,
    pub model_calls: u32,
    pub tool_calls: u32,
    pub approved_servers: HashSet<String>,
    pub status: Status,
    pub created_at: String,
    pub finished_at: Option<String>,
}

pub fn find_by_child(db: &Connection, child: Uuid) -> Result<Option<Record>, Error> {
    if child.is_nil() {
        return Err(Error::Invalid);
    }
    let row = db
        .query_row(
            "select child_conversation_id,parent_conversation_id,parent_context_snapshot_id,child_context_snapshot_id,depth,model_calls,tool_calls,approved_servers_json,status,created_at,finished_at from agent_subtasks where child_conversation_id=?1",
            params![child.to_string()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, String>(8)?,
                    row.get::<_, String>(9)?,
                    row.get::<_, Option<String>>(10)?,
                ))
            },
        )
        .optional()?;
    row.map(
        |(
            child,
            parent,
            parent_context,
            child_context,
            depth,
            model_calls,
            tool_calls,
            approved_servers,
            state,
            created_at,
            finished_at,
        )| {
            if !(1..=2).contains(&depth)
                || !(0..=i64::from(u32::MAX)).contains(&model_calls)
                || !(0..=i64::from(u32::MAX)).contains(&tool_calls)
            {
                return Err(Error::Invalid);
            }
            let parse = |value: String| Uuid::parse_str(&value).map_err(|_| Error::Invalid);
            Ok(Record {
                child_conversation_id: parse(child)?,
                parent_conversation_id: parse(parent)?,
                parent_context_snapshot_id: parse(parent_context)?,
                child_context_snapshot_id: parse(child_context)?,
                depth: depth as u8,
                model_calls: model_calls as u32,
                tool_calls: tool_calls as u32,
                approved_servers: parse_servers(&approved_servers)?,
                status: status(&state)?,
                created_at,
                finished_at,
            })
        },
    )
    .transpose()
}

fn parse_servers(value: &str) -> Result<HashSet<String>, Error> {
    let servers: Vec<String> = serde_json::from_str(value).map_err(|_| Error::Invalid)?;
    if servers.iter().any(|server| {
        server.is_empty() || server.len() > 128 || server.chars().any(char::is_control)
    }) {
        return Err(Error::Invalid);
    }
    Ok(servers.into_iter().collect())
}

fn status(value: &str) -> Result<Status, Error> {
    match value {
        "running" => Ok(Status::Running),
        "completed" => Ok(Status::Completed),
        "canceled" => Ok(Status::Canceled),
        "failed" => Ok(Status::Failed),
        "recovery_required" => Ok(Status::RecoveryRequired),
        _ => Err(Error::Invalid),
    }
}

pub fn list_for_parent(db: &Connection, parent: Uuid, limit: u16) -> Result<Vec<Record>, Error> {
    if parent.is_nil() || limit == 0 || limit > 128 {
        return Err(Error::Invalid);
    }
    let mut statement = db.prepare("select child_conversation_id,parent_conversation_id,parent_context_snapshot_id,child_context_snapshot_id,depth,model_calls,tool_calls,approved_servers_json,status,created_at,finished_at from agent_subtasks where parent_conversation_id=?1 order by created_at desc limit ?2")?;
    let rows = statement.query_map(params![parent.to_string(), i64::from(limit)], |row| {
        let child: String = row.get(0)?;
        let parent: String = row.get(1)?;
        let parent_context: String = row.get(2)?;
        let child_context: String = row.get(3)?;
        let depth: i64 = row.get(4)?;
        let model_calls: i64 = row.get(5)?;
        let tool_calls: i64 = row.get(6)?;
        let approved_servers: String = row.get(7)?;
        let status: String = row.get(8)?;
        let created_at: String = row.get(9)?;
        let finished_at: Option<String> = row.get(10)?;
        Ok((
            child,
            parent,
            parent_context,
            child_context,
            depth,
            model_calls,
            tool_calls,
            approved_servers,
            status,
            created_at,
            finished_at,
        ))
    })?;
    rows.map(|row| {
        let (
            child,
            parent_id,
            parent_context,
            child_context,
            depth,
            model_calls,
            tool_calls,
            approved_servers,
            state,
            created_at,
            finished_at,
        ) = row?;
        let parse = |value: String| Uuid::parse_str(&value).map_err(|_| Error::Invalid);
        if !(1..=2).contains(&depth)
            || model_calls < 0
            || tool_calls < 0
            || depth > i64::from(u8::MAX)
            || model_calls > i64::from(u32::MAX)
            || tool_calls > i64::from(u32::MAX)
        {
            return Err(Error::Invalid);
        }
        Ok(Record {
            child_conversation_id: parse(child)?,
            parent_conversation_id: parse(parent_id)?,
            parent_context_snapshot_id: parse(parent_context)?,
            child_context_snapshot_id: parse(child_context)?,
            depth: depth as u8,
            model_calls: model_calls as u32,
            tool_calls: tool_calls as u32,
            approved_servers: parse_servers(&approved_servers)?,
            status: status(&state)?,
            created_at,
            finished_at,
        })
    })
    .collect()
}

pub fn insert(db: &Connection, spec: &SubtaskSpec) -> Result<(), Error> {
    if spec.child_conversation_id.is_nil()
        || spec.parent_conversation_id.is_nil()
        || spec.depth == 0
        || spec.depth > 2
        || spec.child_context_snapshot_id.is_nil()
        || spec.parent_context_snapshot_id.is_nil()
    {
        return Err(Error::Invalid);
    }
    let approved_servers =
        serde_json::to_string(&spec.approved_servers).map_err(|_| Error::Invalid)?;
    db.execute("insert into agent_subtasks(child_conversation_id,parent_conversation_id,parent_context_snapshot_id,child_context_snapshot_id,depth,model_calls,tool_calls,approved_servers_json,status) values(?1,?2,?3,?4,?5,?6,?7,?8,'running')", params![spec.child_conversation_id.to_string(), spec.parent_conversation_id.to_string(), spec.parent_context_snapshot_id.to_string(), spec.child_context_snapshot_id.to_string(), spec.depth, spec.model_calls as i64, spec.tool_calls as i64, approved_servers])?;
    Ok(())
}

pub fn finish(db: &Connection, child: Uuid, status: Status) -> Result<(), Error> {
    if child.is_nil() {
        return Err(Error::Invalid);
    }
    let changed = db.execute("update agent_subtasks set status=?1, finished_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') where child_conversation_id=?2 and status='running'", params![status.as_str(), child.to_string()])?;
    if changed != 1 {
        return Err(Error::Invalid);
    }
    Ok(())
}

/// A process restart invalidates in-memory task owners. Preserve the child
/// conversation and its durable history, but move any row that was still
/// running into an explicit recovery state instead of presenting it as live.
pub fn recover_on_startup(db: &mut Connection) -> Result<usize, Error> {
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| Error::Storage)?;
    let changed = tx
        .execute(
            "update agent_subtasks set status='recovery_required', finished_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') where status='running'",
            [],
        )
        .map_err(|_| Error::Storage)?;
    // A process restart invalidates the in-memory owner, but it does not make
    // a terminal event unknowable. Reconcile only an explicit TurnFinished
    // already committed to the child conversation; never replay a request.
    let mut statement = tx
        .prepare(
            "select child_conversation_id,
                    (select json_extract(item.payload_json, '$.payload.outcome')
                       from agent_items item
                      where item.conversation_id = task.child_conversation_id
                        and json_extract(item.payload_json, '$.payload.type') = 'turnFinished'
                      order by item.ordinal desc limit 1)
               from agent_subtasks task
              where task.status = 'recovery_required'",
        )
        .map_err(|_| Error::Storage)?;
    let reconciled = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })
        .map_err(|_| Error::Storage)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| Error::Storage)?;
    drop(statement);
    for (child, outcome) in reconciled {
        let Some(status) = outcome.and_then(|value| match value.as_str() {
            "completed" => Some("completed"),
            "canceled" => Some("canceled"),
            "failed" => Some("failed"),
            _ => None,
        }) else {
            continue;
        };
        tx.execute(
            "update agent_subtasks set status=?1 where child_conversation_id=?2 and status='recovery_required'",
            params![status, child],
        )
        .map_err(|_| Error::Storage)?;
    }
    tx.commit().map_err(|_| Error::Storage)?;
    Ok(changed)
}

/// Re-check one recovery-marked child against its already committed journal.
/// This never changes a running row and never starts a model request.
pub fn reconcile_terminal(db: &mut Connection, child: Uuid) -> Result<bool, Error> {
    if child.is_nil() {
        return Err(Error::Invalid);
    }
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| Error::Storage)?;
    let outcome: Option<String> = tx
        .query_row(
            "select (select json_extract(item.payload_json, '$.payload.outcome')
                      from agent_items item
                     where item.conversation_id = task.child_conversation_id
                       and json_extract(item.payload_json, '$.payload.type') = 'turnFinished'
                     order by item.ordinal desc limit 1)
               from agent_subtasks task
              where task.child_conversation_id=?1 and task.status='recovery_required'",
            [child.to_string()],
            |row| row.get(0),
        )
        .optional()
        .map_err(|_| Error::Storage)?
        .flatten();
    let Some(status) = outcome.and_then(|value| match value.as_str() {
        "completed" => Some("completed"),
        "canceled" => Some("canceled"),
        "failed" => Some("failed"),
        _ => None,
    }) else {
        tx.commit().map_err(|_| Error::Storage)?;
        return Ok(false);
    };
    let changed = tx
        .execute(
            "update agent_subtasks set status=?1 where child_conversation_id=?2 and status='recovery_required'",
            params![status, child.to_string()],
        )
        .map_err(|_| Error::Storage)?;
    tx.commit().map_err(|_| Error::Storage)?;
    Ok(changed == 1)
}

/// Atomically claim a recovery-marked child for an explicit host resume.
/// Completed/canceled/failed children and already-running owners cannot be
/// restarted. The caller must roll this back if admission fails.
pub fn claim_for_resume(db: &Connection, child: Uuid) -> Result<bool, Error> {
    if child.is_nil() {
        return Err(Error::Invalid);
    }
    Ok(db.execute(
        "update agent_subtasks set status='running',finished_at=null where child_conversation_id=?1 and status='recovery_required'",
        [child.to_string()],
    )? == 1)
}

pub fn release_resume_claim(db: &Connection, child: Uuid) -> Result<bool, Error> {
    if child.is_nil() {
        return Err(Error::Invalid);
    }
    Ok(db.execute(
        "update agent_subtasks set status='recovery_required',finished_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') where child_conversation_id=?1 and status='running'",
        [child.to_string()],
    )? == 1)
}

/// Move a claimed child to the fresh context identity of an explicit retry.
/// The interrupted turn remains immutable; subsequent recovery attempts must
/// bind to the newest frozen submission instead of the terminated turn.
pub fn bind_retry_context(
    db: &Connection,
    child: Uuid,
    previous: Uuid,
    retry: Uuid,
) -> Result<bool, Error> {
    if child.is_nil() || previous.is_nil() || retry.is_nil() || previous == retry {
        return Err(Error::Invalid);
    }
    Ok(db.execute(
        "update agent_subtasks set child_context_snapshot_id=?1
          where child_conversation_id=?2 and child_context_snapshot_id=?3 and status='running'",
        params![retry.to_string(), child.to_string(), previous.to_string()],
    )? == 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn db() -> Connection {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("create table agent_subtasks (child_conversation_id text primary key,parent_conversation_id text not null,parent_context_snapshot_id text not null,child_context_snapshot_id text not null,depth integer not null,model_calls integer not null,tool_calls integer not null,approved_servers_json text not null default '[]',status text not null,created_at text not null default 'now',finished_at text); create table agent_items (conversation_id text not null, ordinal integer not null, payload_json text not null);").unwrap();
        db
    }
    #[test]
    fn records_one_terminal_transition() {
        let db = db();
        let spec = SubtaskSpec {
            parent_conversation_id: Uuid::new_v4(),
            child_conversation_id: Uuid::new_v4(),
            parent_context_snapshot_id: Uuid::new_v4(),
            child_context_snapshot_id: Uuid::new_v4(),
            depth: 1,
            model_calls: 2,
            tool_calls: 3,
            approved_servers: Default::default(),
        };
        insert(&db, &spec).unwrap();
        finish(&db, spec.child_conversation_id, Status::Completed).unwrap();
        assert_eq!(
            finish(&db, spec.child_conversation_id, Status::Failed),
            Err(Error::Invalid)
        );
    }

    #[test]
    fn claimed_retry_rebinds_only_the_expected_context() {
        let db = db();
        let original = Uuid::new_v4();
        let retry = Uuid::new_v4();
        let spec = SubtaskSpec {
            parent_conversation_id: Uuid::new_v4(),
            child_conversation_id: Uuid::new_v4(),
            parent_context_snapshot_id: Uuid::new_v4(),
            child_context_snapshot_id: original,
            depth: 1,
            model_calls: 1,
            tool_calls: 1,
            approved_servers: Default::default(),
        };
        insert(&db, &spec).unwrap();
        finish(&db, spec.child_conversation_id, Status::RecoveryRequired).unwrap();
        assert!(claim_for_resume(&db, spec.child_conversation_id).unwrap());
        assert!(
            !bind_retry_context(&db, spec.child_conversation_id, Uuid::new_v4(), retry).unwrap()
        );
        assert!(bind_retry_context(&db, spec.child_conversation_id, original, retry).unwrap());
        assert_eq!(
            find_by_child(&db, spec.child_conversation_id)
                .unwrap()
                .unwrap()
                .child_context_snapshot_id,
            retry
        );
    }

    #[test]
    fn approved_server_scope_survives_round_trip() {
        let db = db();
        let spec = SubtaskSpec {
            parent_conversation_id: Uuid::new_v4(),
            child_conversation_id: Uuid::new_v4(),
            parent_context_snapshot_id: Uuid::new_v4(),
            child_context_snapshot_id: Uuid::new_v4(),
            depth: 1,
            model_calls: 2,
            tool_calls: 3,
            approved_servers: HashSet::from(["literature".to_owned()]),
        };
        insert(&db, &spec).unwrap();
        let restored = find_by_child(&db, spec.child_conversation_id)
            .unwrap()
            .expect("stored child");
        assert_eq!(restored.approved_servers, spec.approved_servers);
        let listed = list_for_parent(&db, spec.parent_conversation_id, 10).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].approved_servers, spec.approved_servers);
        let mut db = db;
        assert_eq!(recover_on_startup(&mut db).unwrap(), 1);
        let recovered = find_by_child(&db, spec.child_conversation_id)
            .unwrap()
            .expect("recovered child");
        assert_eq!(recovered.status, Status::RecoveryRequired);
        assert_eq!(recovered.approved_servers, spec.approved_servers);
    }

    #[test]
    fn lists_only_bounded_children_for_the_requested_parent() {
        let db = db();
        let parent = Uuid::new_v4();
        let spec = SubtaskSpec {
            parent_conversation_id: parent,
            child_conversation_id: Uuid::new_v4(),
            parent_context_snapshot_id: Uuid::new_v4(),
            child_context_snapshot_id: Uuid::new_v4(),
            depth: 1,
            model_calls: 2,
            tool_calls: 3,
            approved_servers: Default::default(),
        };
        insert(&db, &spec).unwrap();
        let other = SubtaskSpec {
            parent_conversation_id: Uuid::new_v4(),
            child_conversation_id: Uuid::new_v4(),
            parent_context_snapshot_id: Uuid::new_v4(),
            child_context_snapshot_id: Uuid::new_v4(),
            depth: 1,
            model_calls: 1,
            tool_calls: 1,
            approved_servers: Default::default(),
        };
        insert(&db, &other).unwrap();
        let records = list_for_parent(&db, parent, 10).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].child_conversation_id, spec.child_conversation_id);
        assert_eq!(records[0].status, Status::Running);
    }

    #[test]
    fn startup_reconciles_only_committed_terminal_events() {
        let mut db = db();
        let parent = Uuid::new_v4();
        let completed = SubtaskSpec {
            parent_conversation_id: parent,
            child_conversation_id: Uuid::new_v4(),
            parent_context_snapshot_id: Uuid::new_v4(),
            child_context_snapshot_id: Uuid::new_v4(),
            depth: 1,
            model_calls: 1,
            tool_calls: 0,
            approved_servers: Default::default(),
        };
        let unknown = SubtaskSpec {
            child_conversation_id: Uuid::new_v4(),
            ..completed.clone()
        };
        insert(&db, &completed).unwrap();
        insert(&db, &unknown).unwrap();
        db.execute(
            "insert into agent_items(conversation_id,ordinal,payload_json) values(?1,0,?2)",
            params![
                completed.child_conversation_id.to_string(),
                serde_json::json!({"payload":{"type":"turnFinished","outcome":"completed"}})
                    .to_string()
            ],
        )
        .unwrap();

        assert_eq!(recover_on_startup(&mut db).unwrap(), 2);
        assert_eq!(
            find_by_child(&db, completed.child_conversation_id)
                .unwrap()
                .unwrap()
                .status,
            Status::Completed
        );
        assert_eq!(
            find_by_child(&db, unknown.child_conversation_id)
                .unwrap()
                .unwrap()
                .status,
            Status::RecoveryRequired
        );
    }

    #[test]
    fn startup_reconciles_each_known_terminal_outcome_without_guessing_unknowns() {
        let mut db = db();
        let parent = Uuid::new_v4();
        let outcomes = [
            ("completed", Status::Completed),
            ("canceled", Status::Canceled),
            ("failed", Status::Failed),
        ];
        let mut children = Vec::new();
        for (outcome, _) in outcomes {
            let spec = SubtaskSpec {
                parent_conversation_id: parent,
                child_conversation_id: Uuid::new_v4(),
                parent_context_snapshot_id: Uuid::new_v4(),
                child_context_snapshot_id: Uuid::new_v4(),
                depth: 1,
                model_calls: 1,
                tool_calls: 1,
                approved_servers: Default::default(),
            };
            insert(&db, &spec).unwrap();
            db.execute(
                "insert into agent_items(conversation_id,ordinal,payload_json) values(?1,0,?2)",
                params![
                    spec.child_conversation_id.to_string(),
                    serde_json::json!({"payload":{"type":"turnFinished","outcome":outcome}})
                        .to_string()
                ],
            )
            .unwrap();
            children.push(spec.child_conversation_id);
        }
        let unknown = SubtaskSpec {
            parent_conversation_id: parent,
            child_conversation_id: Uuid::new_v4(),
            parent_context_snapshot_id: Uuid::new_v4(),
            child_context_snapshot_id: Uuid::new_v4(),
            depth: 1,
            model_calls: 1,
            tool_calls: 1,
            approved_servers: Default::default(),
        };
        insert(&db, &unknown).unwrap();
        db.execute(
            "insert into agent_items(conversation_id,ordinal,payload_json) values(?1,0,?2)",
            params![
                unknown.child_conversation_id.to_string(),
                serde_json::json!({"payload":{"type":"turnFinished","outcome":"future_status"}})
                    .to_string()
            ],
        )
        .unwrap();

        assert_eq!(recover_on_startup(&mut db).unwrap(), 4);
        for (child, (_, expected)) in children.into_iter().zip(outcomes) {
            assert_eq!(find_by_child(&db, child).unwrap().unwrap().status, expected);
        }
        assert_eq!(
            find_by_child(&db, unknown.child_conversation_id)
                .unwrap()
                .unwrap()
                .status,
            Status::RecoveryRequired
        );
    }

    #[test]
    fn startup_marks_running_children_recoverable_without_touching_terminal_rows() {
        let mut db = db();
        let running = SubtaskSpec {
            parent_conversation_id: Uuid::new_v4(),
            child_conversation_id: Uuid::new_v4(),
            parent_context_snapshot_id: Uuid::new_v4(),
            child_context_snapshot_id: Uuid::new_v4(),
            depth: 1,
            model_calls: 1,
            tool_calls: 1,
            approved_servers: Default::default(),
        };
        let completed = SubtaskSpec {
            parent_conversation_id: Uuid::new_v4(),
            child_conversation_id: Uuid::new_v4(),
            parent_context_snapshot_id: Uuid::new_v4(),
            child_context_snapshot_id: Uuid::new_v4(),
            depth: 1,
            model_calls: 1,
            tool_calls: 1,
            approved_servers: Default::default(),
        };
        insert(&db, &running).unwrap();
        insert(&db, &completed).unwrap();
        finish(&db, completed.child_conversation_id, Status::Completed).unwrap();
        assert_eq!(recover_on_startup(&mut db).unwrap(), 1);
        assert_eq!(recover_on_startup(&mut db).unwrap(), 0);
        assert_eq!(
            list_for_parent(&db, running.parent_conversation_id, 1).unwrap()[0].status,
            Status::RecoveryRequired
        );
        assert_eq!(
            list_for_parent(&db, completed.parent_conversation_id, 1).unwrap()[0].status,
            Status::Completed
        );
    }

    #[test]
    fn recovery_claim_is_single_owner_and_can_be_released() {
        let mut db = db();
        let spec = SubtaskSpec {
            parent_conversation_id: Uuid::new_v4(),
            child_conversation_id: Uuid::new_v4(),
            parent_context_snapshot_id: Uuid::new_v4(),
            child_context_snapshot_id: Uuid::new_v4(),
            depth: 1,
            model_calls: 1,
            tool_calls: 1,
            approved_servers: Default::default(),
        };
        insert(&db, &spec).unwrap();
        recover_on_startup(&mut db).unwrap();

        assert!(claim_for_resume(&db, spec.child_conversation_id).unwrap());
        assert!(!claim_for_resume(&db, spec.child_conversation_id).unwrap());
        assert_eq!(
            find_by_child(&db, spec.child_conversation_id)
                .unwrap()
                .unwrap()
                .status,
            Status::Running
        );
        assert!(release_resume_claim(&db, spec.child_conversation_id).unwrap());
        assert!(!release_resume_claim(&db, spec.child_conversation_id).unwrap());
        assert!(claim_for_resume(&db, spec.child_conversation_id).unwrap());
        finish(&db, spec.child_conversation_id, Status::Completed).unwrap();
        assert!(!claim_for_resume(&db, spec.child_conversation_id).unwrap());
    }

    #[test]
    fn finds_parent_bounds_by_child_identity() {
        let db = db();
        let spec = SubtaskSpec {
            parent_conversation_id: Uuid::new_v4(),
            child_conversation_id: Uuid::new_v4(),
            parent_context_snapshot_id: Uuid::new_v4(),
            child_context_snapshot_id: Uuid::new_v4(),
            depth: 2,
            model_calls: 1,
            tool_calls: 2,
            approved_servers: Default::default(),
        };
        insert(&db, &spec).unwrap();
        let found = find_by_child(&db, spec.child_conversation_id)
            .unwrap()
            .unwrap();
        assert_eq!(found.depth, 2);
        assert_eq!(
            found.child_context_snapshot_id,
            spec.child_context_snapshot_id
        );
        assert_eq!(found.model_calls, 1);
        assert!(find_by_child(&db, Uuid::new_v4()).unwrap().is_none());
    }
}
