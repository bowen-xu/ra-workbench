//! Durable intent for a later writing execution copy. This grants no execution.
use super::*;
use research_agent_core::protocol::{AgentEvent, EventPayload};

/// Identify the sole fully confirmed delivery of the previous exact lease.
/// Selection itself is read-only and never creates or approves a workspace.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn confirmed_predecessor(
    db: &Connection,
    previous: &Binding,
    next_generation: u32,
    next_lease: Uuid,
) -> Result<Uuid, Error> {
    if next_lease.is_nil() || next_lease == previous.lease_id {
        return Err(Error::Conflict);
    }
    let mut query = db.prepare("select p.delivery_id from writing_agent_delivery_proofs p join writing_agent_delivery_tool_calls c on c.delivery_id=p.delivery_id join writing_agent_delivery_results r on r.delivery_id=p.delivery_id where p.conversation_id=?1 and p.request_id=?2 and p.turn_id=?3 and p.context_snapshot_id=?4 and p.manuscript_id=?5 and p.lease_id=?6 and p.manifest_hash=?7 limit 2")?;
    let mut rows = query.query(params![
        previous.conversation_id.to_string(),
        previous.request_id.to_string(),
        previous.turn_id.to_string(),
        previous.context_snapshot_id.to_string(),
        previous.manuscript_id.to_string(),
        previous.lease_id.to_string(),
        previous.manifest_hash
    ])?;
    let Some(row) = rows.next()? else {
        return Err(Error::Denied);
    };
    let id: String = row.get(0)?;
    if rows.next()?.is_some() {
        return Err(Error::Conflict);
    }
    let predecessor = Uuid::parse_str(&id).map_err(|_| Error::Conflict)?;
    let mut next = previous.clone();
    next.lease_id = next_lease;
    validate_predecessor(db, &next, next_generation, predecessor)?;
    Ok(predecessor)
}

/// The manifest is an already captured current snapshot, not the old submission
/// snapshot. The predecessor must be fully delivered and receipt-confirmed.
pub fn prepare_in(
    tx: &Transaction<'_>,
    request_id: Uuid,
    manuscript_id: Uuid,
    generation: u32,
    predecessor: Uuid,
    manifest: &WorkspaceManifest,
) -> Result<Binding, Error> {
    if generation == 0 || generation > i32::MAX as u32 || predecessor.is_nil() {
        return Err(Error::Conflict);
    }
    discard::ensure_preparation_allowed(tx, manifest.identity.conversation_id, request_id)?;
    let record = super::super::submissions::read(tx, manifest.identity.conversation_id, request_id)
        .map_err(|_| Error::Storage)?
        .ok_or(Error::Conflict)?;
    let (hash, json) = encoded(manifest)?;
    let binding = Binding {
        conversation_id: record.conversation_id,
        request_id,
        turn_id: record.turn_id,
        context_snapshot_id: record.context_snapshot_id,
        manuscript_id,
        lease_id: manifest.identity.lease_id,
        manifest_hash: hash,
    };
    if manifest.identity.turn_id != record.turn_id
        || manifest.identity.environment_id != format!("manuscript:{manuscript_id}")
        || manifest
            .files
            .iter()
            .any(|file| file.source.resource_id != format!("{manuscript_id}/{}", file.file.path))
    {
        return Err(Error::Conflict);
    }
    validate_predecessor(tx, &binding, generation, predecessor)?;
    let inserted = tx.execute("insert into writing_agent_workspace_generations(conversation_id,request_id,generation,turn_id,context_snapshot_id,manuscript_id,lease_id,manifest_hash,manifest_json,predecessor_delivery_id) values(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10) on conflict do nothing", params![binding.conversation_id.to_string(),request_id.to_string(),generation,binding.turn_id.to_string(),binding.context_snapshot_id.to_string(),manuscript_id.to_string(),binding.lease_id.to_string(),binding.manifest_hash,json,predecessor.to_string()])?;
    if inserted == 0 {
        let same: bool = tx.query_row("select exists(select 1 from writing_agent_workspace_generations where conversation_id=?1 and request_id=?2 and generation=?3 and turn_id=?4 and context_snapshot_id=?5 and manuscript_id=?6 and lease_id=?7 and manifest_hash=?8 and manifest_json=?9 and predecessor_delivery_id=?10 and state='preparing')", params![binding.conversation_id.to_string(),request_id.to_string(),generation,binding.turn_id.to_string(),binding.context_snapshot_id.to_string(),manuscript_id.to_string(),binding.lease_id.to_string(),binding.manifest_hash,json,predecessor.to_string()], |r|r.get(0))?;
        if !same {
            return Err(Error::Conflict);
        }
    }
    Ok(binding)
}

fn validate_predecessor(
    tx: &Connection,
    binding: &Binding,
    generation: u32,
    predecessor: Uuid,
) -> Result<(), Error> {
    let request_id = binding.request_id;
    if generation == 0 || generation > i32::MAX as u32 || predecessor.is_nil() {
        return Err(Error::Conflict);
    }
    discard::ensure_preparation_allowed(tx, binding.conversation_id, request_id)?;
    eligible(tx, binding)?;
    validate_previous_delivery(tx, binding, generation, predecessor)
}

fn validate_previous_delivery(
    tx: &Connection,
    binding: &Binding,
    generation: u32,
    predecessor: Uuid,
) -> Result<(), Error> {
    let request_id = binding.request_id;
    let manuscript_id = binding.manuscript_id;
    if generation == 0 || generation > i32::MAX as u32 || predecessor.is_nil() {
        return Err(Error::Conflict);
    }
    let previous: Option<(String, String, String)> = if generation == 1 {
        tx.query_row("select lease_id,state,manifest_hash from agent_execution_workspaces where conversation_id=?1 and request_id=?2 and turn_id=?3 and context_snapshot_id=?4 and manuscript_id=?5", params![binding.conversation_id.to_string(), request_id.to_string(), binding.turn_id.to_string(), binding.context_snapshot_id.to_string(),manuscript_id.to_string()], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?
    } else {
        tx.query_row("select lease_id,state,manifest_hash from writing_agent_workspace_generations where conversation_id=?1 and request_id=?2 and generation=?3 and turn_id=?4 and context_snapshot_id=?5 and manuscript_id=?6", params![binding.conversation_id.to_string(),request_id.to_string(),generation-1,binding.turn_id.to_string(),binding.context_snapshot_id.to_string(),manuscript_id.to_string()], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?
    };
    let (prior_lease, state, prior_hash) = previous.ok_or(Error::Conflict)?;
    if state != "revoked" || prior_lease == binding.lease_id.to_string() {
        return Err(Error::Denied);
    }
    let call: Option<String> = tx.query_row("select c.call_id from writing_agent_delivery_proofs p join writing_agent_delivery_tool_calls c on c.delivery_id=p.delivery_id join writing_agent_delivery_results r on r.delivery_id=p.delivery_id where p.delivery_id=?1 and p.conversation_id=?2 and p.request_id=?3 and p.turn_id=?4 and p.context_snapshot_id=?5 and p.manuscript_id=?6 and p.lease_id=?7 and p.manifest_hash=?8", params![predecessor.to_string(),binding.conversation_id.to_string(),request_id.to_string(),binding.turn_id.to_string(),binding.context_snapshot_id.to_string(),manuscript_id.to_string(),prior_lease,prior_hash], |r| r.get(0)).optional()?;
    let call = call.ok_or(Error::Denied)?;
    let raw: Option<String> = tx.query_row("select payload_json from agent_items where conversation_id=?1 and json_extract(payload_json,'$.requestId')=?2 and json_extract(payload_json,'$.payload.type')='toolReceiptConfirmed' and json_extract(payload_json,'$.payload.turnId')=?3 and json_extract(payload_json,'$.payload.callId')=?4", params![binding.conversation_id.to_string(),request_id.to_string(),binding.turn_id.to_string(),call], |r| r.get(0)).optional()?;
    let event: AgentEvent =
        serde_json::from_str(&raw.ok_or(Error::Denied)?).map_err(|_| Error::Conflict)?;
    if *event.conversation_id().as_uuid() != binding.conversation_id
        || event.request_id() != request_id
        || !matches!(event.payload(), EventPayload::ToolReceiptConfirmed { turn_id, call_id } if *turn_id == binding.turn_id && call_id.to_string() == call)
    {
        return Err(Error::Conflict);
    }
    Ok(())
}

/// Review a specific retained successor without reactivating its grant.
pub fn read_for_review(db: &Connection, binding: &Binding) -> Result<WorkspaceManifest, Error> {
    discard::ensure_preparation_allowed(db, binding.conversation_id, binding.request_id)?;
    visible(db, binding)?;
    let row: Option<(u32, String, String)> = db.query_row(
        "select generation,predecessor_delivery_id,state from writing_agent_workspace_generations where conversation_id=?1 and request_id=?2 and turn_id=?3 and context_snapshot_id=?4 and manuscript_id=?5 and lease_id=?6 and manifest_hash=?7",
        params![binding.conversation_id.to_string(),binding.request_id.to_string(),binding.turn_id.to_string(),binding.context_snapshot_id.to_string(),binding.manuscript_id.to_string(),binding.lease_id.to_string(),binding.manifest_hash],
        |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)),
    ).optional()?;
    let (generation, predecessor, state) = row.ok_or(Error::Conflict)?;
    if state != "approved" && state != "revoked" {
        return Err(Error::Denied);
    }
    let predecessor = Uuid::parse_str(&predecessor).map_err(|_| Error::Conflict)?;
    validate_previous_delivery(db, binding, generation, predecessor)?;
    let (saved, manifest) = read_state(
        db,
        binding.conversation_id,
        binding.request_id,
        generation,
        predecessor,
        binding.lease_id,
        &state,
    )?;
    if &saved != binding
        || manifest.identity.environment_id != format!("manuscript:{}", binding.manuscript_id)
        || manifest.files.iter().any(|file| {
            file.source.resource_id != format!("{}/{}", binding.manuscript_id, file.file.path)
        })
    {
        return Err(Error::Conflict);
    }
    Ok(manifest)
}

/// Called only after the app-owned copy was fully created and verified.
pub fn finish_in(
    tx: &Transaction<'_>,
    generation: u32,
    predecessor: Uuid,
    binding: &Binding,
) -> Result<(), Error> {
    validate_predecessor(tx, binding, generation, predecessor)?;
    let changed = tx.execute("update writing_agent_workspace_generations set state='prepared' where conversation_id=?1 and request_id=?2 and generation=?3 and turn_id=?4 and context_snapshot_id=?5 and manuscript_id=?6 and lease_id=?7 and manifest_hash=?8 and predecessor_delivery_id=?9 and state='preparing'", params![binding.conversation_id.to_string(),binding.request_id.to_string(),generation,binding.turn_id.to_string(),binding.context_snapshot_id.to_string(),binding.manuscript_id.to_string(),binding.lease_id.to_string(),binding.manifest_hash,predecessor.to_string()])?;
    if changed != 1 {
        return Err(Error::Conflict);
    }
    Ok(())
}

/// Recheck the immutable identity and predecessor whenever a grant is used.
fn checked_state(
    db: &Connection,
    binding: &Binding,
    generation: u32,
    predecessor: Uuid,
    manifest: &WorkspaceManifest,
) -> Result<String, Error> {
    validate_predecessor(db, binding, generation, predecessor)?;
    let (hash, json) = encoded(manifest)?;
    if hash != binding.manifest_hash
        || manifest.identity.conversation_id != binding.conversation_id
        || manifest.identity.turn_id != binding.turn_id
        || manifest.identity.lease_id != binding.lease_id
        || manifest.identity.environment_id != format!("manuscript:{}", binding.manuscript_id)
        || manifest.files.iter().any(|file| {
            file.source.resource_id != format!("{}/{}", binding.manuscript_id, file.file.path)
        })
    {
        return Err(Error::Conflict);
    }
    db.query_row("select state from writing_agent_workspace_generations where conversation_id=?1 and request_id=?2 and generation=?3 and turn_id=?4 and context_snapshot_id=?5 and manuscript_id=?6 and lease_id=?7 and manifest_hash=?8 and manifest_json=?9 and predecessor_delivery_id=?10", params![binding.conversation_id.to_string(),binding.request_id.to_string(),generation,binding.turn_id.to_string(),binding.context_snapshot_id.to_string(),binding.manuscript_id.to_string(),binding.lease_id.to_string(),binding.manifest_hash,json,predecessor.to_string()], |r| r.get(0)).optional()?.ok_or(Error::Conflict)
}

/// Grant only a clean, exact prepared successor; a revoked grant cannot be reused.
pub fn approve_in(
    tx: &Transaction<'_>,
    binding: &Binding,
    generation: u32,
    predecessor: Uuid,
    manifest: &WorkspaceManifest,
) -> Result<(), Error> {
    if checked_state(tx, binding, generation, predecessor, manifest)? != "prepared" {
        return Err(Error::Denied);
    }
    let changed = tx.execute("update writing_agent_workspace_generations set state='approved' where conversation_id=?1 and request_id=?2 and generation=?3 and lease_id=?4 and state='prepared'", params![binding.conversation_id.to_string(),binding.request_id.to_string(),generation,binding.lease_id.to_string()])?;
    if changed != 1 {
        return Err(Error::Conflict);
    }
    Ok(())
}

pub fn authorize(
    db: &Connection,
    binding: &Binding,
    generation: u32,
    predecessor: Uuid,
    manifest: &WorkspaceManifest,
) -> Result<(), Error> {
    if checked_state(db, binding, generation, predecessor, manifest)? == "approved" {
        Ok(())
    } else {
        Err(Error::Denied)
    }
}

/// Withdrawal remains possible after the turn ends or resource is archived.
pub fn revoke_binding_in(
    tx: &Transaction<'_>,
    binding: &Binding,
    generation: u32,
    predecessor: Uuid,
) -> Result<(), Error> {
    let changed = tx.execute("update writing_agent_workspace_generations set state='revoked' where conversation_id=?1 and request_id=?2 and generation=?3 and turn_id=?4 and context_snapshot_id=?5 and manuscript_id=?6 and lease_id=?7 and manifest_hash=?8 and predecessor_delivery_id=?9", params![binding.conversation_id.to_string(),binding.request_id.to_string(),generation,binding.turn_id.to_string(),binding.context_snapshot_id.to_string(),binding.manuscript_id.to_string(),binding.lease_id.to_string(),binding.manifest_hash,predecessor.to_string()])?;
    if changed != 1 {
        return Err(Error::Conflict);
    }
    Ok(())
}

/// Read a particular unfinished intent. This does not authorize or recreate it.
pub fn read_preparing(
    db: &Connection,
    conversation: Uuid,
    request: Uuid,
    generation: u32,
    predecessor: Uuid,
    lease: Uuid,
) -> Result<(Binding, WorkspaceManifest), Error> {
    read_state(
        db,
        conversation,
        request,
        generation,
        predecessor,
        lease,
        "preparing",
    )
}

/// Read an exact live grant for lease restoration. Revalidate the predecessor
/// and turn; a saved row alone is never an execution authorization.
pub fn read_approved(
    db: &Connection,
    conversation: Uuid,
    request: Uuid,
    generation: u32,
    predecessor: Uuid,
    lease: Uuid,
) -> Result<(Binding, WorkspaceManifest), Error> {
    let (binding, manifest) = read_state(
        db,
        conversation,
        request,
        generation,
        predecessor,
        lease,
        "approved",
    )?;
    authorize(db, &binding, generation, predecessor, &manifest)?;
    Ok((binding, manifest))
}

/// An interrupted preparation may have reached prepared before approval.
pub fn read_prepared(
    db: &Connection,
    conversation: Uuid,
    request: Uuid,
    generation: u32,
    predecessor: Uuid,
    lease: Uuid,
) -> Result<(Binding, WorkspaceManifest), Error> {
    let (binding, manifest) = read_state(
        db,
        conversation,
        request,
        generation,
        predecessor,
        lease,
        "prepared",
    )?;
    if checked_state(db, &binding, generation, predecessor, &manifest)? != "prepared" {
        return Err(Error::Denied);
    }
    Ok((binding, manifest))
}

fn read_state(
    db: &Connection,
    conversation: Uuid,
    request: Uuid,
    generation: u32,
    predecessor: Uuid,
    lease: Uuid,
    expected_state: &str,
) -> Result<(Binding, WorkspaceManifest), Error> {
    let row: Option<(String,String,String,String,String,String)> = db.query_row("select turn_id,context_snapshot_id,manuscript_id,manifest_hash,manifest_json,state from writing_agent_workspace_generations where conversation_id=?1 and request_id=?2 and generation=?3 and predecessor_delivery_id=?4 and lease_id=?5", params![conversation.to_string(),request.to_string(),generation,predecessor.to_string(),lease.to_string()], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).optional()?;
    let (turn, context, manuscript, hash, json, state) = row.ok_or(Error::Conflict)?;
    if state != expected_state || json.len() > 32 * 1024 * 1024 {
        return Err(Error::Denied);
    }
    let parse = |value: &str| Uuid::parse_str(value).map_err(|_| Error::Conflict);
    let binding = Binding {
        conversation_id: conversation,
        request_id: request,
        turn_id: parse(&turn)?,
        context_snapshot_id: parse(&context)?,
        manuscript_id: parse(&manuscript)?,
        lease_id: lease,
        manifest_hash: hash,
    };
    let manifest: WorkspaceManifest = serde_json::from_str(&json).map_err(|_| Error::Conflict)?;
    if encoded(&manifest)?.0 != binding.manifest_hash
        || manifest.identity.conversation_id != conversation
        || manifest.identity.turn_id != binding.turn_id
        || manifest.identity.lease_id != lease
    {
        return Err(Error::Conflict);
    }
    Ok((binding, manifest))
}
