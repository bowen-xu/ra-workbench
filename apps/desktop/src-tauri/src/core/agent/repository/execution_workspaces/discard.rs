//! Durable discard intent. The native coordinator owns quiescence, exact reviewed
//! inventory verification and filesystem completion; this module never deletes files.
use super::*;
use research_agent_core::workspace::WorkspaceIdentity;

#[path = "discard/recovery.rs"]
pub mod recovery;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub operation_id: Uuid,
    pub binding: Binding,
    pub environment_id: String,
    pub inventory_hash: String,
}
impl Request {
    pub fn identity(&self) -> WorkspaceIdentity {
        WorkspaceIdentity {
            conversation_id: self.binding.conversation_id,
            turn_id: self.binding.turn_id,
            environment_id: self.environment_id.clone(),
            lease_id: self.binding.lease_id,
        }
    }
    pub fn validate(&self) -> Result<(), Error> {
        let b = &self.binding;
        if [
            self.operation_id,
            b.conversation_id,
            b.request_id,
            b.turn_id,
            b.context_snapshot_id,
            b.manuscript_id,
            b.lease_id,
        ]
        .iter()
        .any(Uuid::is_nil)
            || [&b.manifest_hash, &self.inventory_hash].iter().any(|s| {
                s.len() != 64
                    || !s
                        .bytes()
                        .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
            })
        {
            return Err(Error::Conflict);
        }
        WorkspaceManifest {
            schema_version: 1,
            identity: self.identity(),
            files: vec![],
        }
        .validate()
        .map_err(|_| Error::Conflict)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Pending,
    Complete,
}

/// Exact native identity lookup also works after archival or unbinding. It grants
/// neither execution authority nor access to original manuscript files.
pub fn baseline(db: &Connection, request: &Request) -> Result<WorkspaceManifest, Error> {
    request.validate()?;
    let b = &request.binding;
    let (saved, manifest) =
        read_metadata_unscoped(db, b.conversation_id, b.request_id)?.ok_or(Error::Conflict)?;
    if saved != *b || manifest.identity != request.identity() {
        return Err(Error::Conflict);
    }
    Ok(manifest)
}

/// Check before creating any files, as well as before binding publication.
pub fn ensure_preparation_allowed(
    db: &Connection,
    conversation: Uuid,
    request: Uuid,
) -> Result<(), Error> {
    let exists: bool = db.query_row(
        "select exists(select 1 from agent_workspace_discards where conversation_id=?1 and request_id=?2)",
        params![conversation.to_string(), request.to_string()], |r| r.get(0))?;
    if exists { Err(Error::Denied) } else { Ok(()) }
}

/// Exact retries remain readable even after the conversation has been removed.
pub fn read(db: &Connection, request: &Request) -> Result<Option<State>, Error> {
    request.validate()?;
    let b = &request.binding;
    let saved: Option<(Request, String)> = db.query_row(
        "select operation_id, conversation_id, request_id, turn_id, context_snapshot_id, manuscript_id, lease_id, environment_id, manifest_hash, inventory_hash, state from agent_workspace_discards where operation_id=?1 or (conversation_id=?2 and request_id=?3) or lease_id=?4",
        params![request.operation_id.to_string(), b.conversation_id.to_string(), b.request_id.to_string(), b.lease_id.to_string()],
        recovery::decode,
    ).optional()?;
    let Some((saved, state)) = saved else {
        return Ok(None);
    };
    if saved != *request {
        return Err(Error::Conflict);
    }
    match state.as_str() {
        "pending" => Ok(Some(State::Pending)),
        "complete" => Ok(Some(State::Complete)),
        _ => Err(Error::Conflict),
    }
}

/// Caller holds the native lease with no process marker and has verified the
/// exact inventory shown in the explicit discard confirmation. Savepoint protects
/// withdrawal and intent even if the caller commits after an insertion error.
pub fn begin_in(tx: &mut Transaction<'_>, request: &Request) -> Result<State, Error> {
    if let Some(state) = read(tx, request)? {
        return Ok(state);
    }
    let b = &request.binding;
    baseline(tx, request)?;
    // A prepared, never-started input may be discarded; a started turn must have
    // a durable terminal event. In-memory admission/process checks remain required.
    let active: bool = tx.query_row(
        "select exists(select 1 from agent_items s where s.conversation_id=?1 and json_extract(s.payload_json,'$.payload.type')='turnStarted' and json_extract(s.payload_json,'$.payload.turnId')=?2 and not exists(select 1 from agent_items f where f.conversation_id=s.conversation_id and json_extract(f.payload_json,'$.payload.type')='turnFinished' and json_extract(f.payload_json,'$.payload.turnId')=?2))",
        params![b.conversation_id.to_string(), b.turn_id.to_string()], |r| r.get(0))?;
    if active {
        return Err(Error::Denied);
    }
    let sp = tx.savepoint()?;
    sp.execute("update agent_execution_workspaces set state='revoked' where conversation_id=?1 and request_id=?2",
        params![b.conversation_id.to_string(), b.request_id.to_string()])?;
    sp.execute(
        "insert into agent_workspace_discards(operation_id,conversation_id,request_id,turn_id,context_snapshot_id,manuscript_id,lease_id,environment_id,manifest_hash,inventory_hash) values(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
        params![request.operation_id.to_string(), b.conversation_id.to_string(), b.request_id.to_string(), b.turn_id.to_string(), b.context_snapshot_id.to_string(), b.manuscript_id.to_string(), b.lease_id.to_string(), request.environment_id, b.manifest_hash, request.inventory_hash])?;
    sp.commit()?;
    Ok(State::Pending)
}

/// Invoke only after WorkspaceStore confirms its durable completion receipt.
/// Releasing the binding never removes saved proposals, application history,
/// source entities or their content-object references.
pub fn complete_in(tx: &mut Transaction<'_>, request: &Request) -> Result<(), Error> {
    let state = read(tx, request)?.ok_or(Error::Conflict)?;
    let b = &request.binding;
    if state == State::Complete {
        let exists: bool = tx.query_row("select exists(select 1 from agent_execution_workspaces where conversation_id=?1 and request_id=?2)", params![b.conversation_id.to_string(), b.request_id.to_string()], |r| r.get(0))?;
        return if exists { Err(Error::Conflict) } else { Ok(()) };
    }
    baseline(tx, request)?;
    let sp = tx.savepoint()?;
    sp.execute("update agent_workspace_discards set state='complete',completed_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') where operation_id=?1 and state='pending'", [request.operation_id.to_string()])?;
    if sp.execute("delete from agent_execution_workspaces where conversation_id=?1 and request_id=?2 and lease_id=?3 and manifest_hash=?4",
        params![b.conversation_id.to_string(), b.request_id.to_string(), b.lease_id.to_string(), b.manifest_hash])? != 1 { return Err(Error::Conflict); }
    sp.commit()?;
    Ok(())
}
