//! Durable binding of a frozen working copy to one immutable writing submission.
//! Native-only; approval must come from the application-owned admission flow.
//! Run on the storage worker. Caller owns transaction/commit acknowledgement.
use research_agent_core::{submission::ContextSourceKind, workspace::WorkspaceManifest};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use sha2::{Digest, Sha256};
use uuid::Uuid;

#[path = "execution_workspaces/discard.rs"]
pub mod discard;
#[path = "execution_workspaces/generation.rs"]
pub mod generation;
#[path = "execution_workspaces/history.rs"]
pub mod history;

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("execution workspace identity changed")]
    Conflict,
    #[error("execution workspace is not authorized")]
    Denied,
    #[error("execution workspace store unavailable")]
    Storage,
}
impl From<rusqlite::Error> for Error {
    fn from(_: rusqlite::Error) -> Self {
        Self::Storage
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Binding {
    pub conversation_id: Uuid,
    pub request_id: Uuid,
    pub turn_id: Uuid,
    pub context_snapshot_id: Uuid,
    pub manuscript_id: Uuid,
    pub lease_id: Uuid,
    pub manifest_hash: String,
}
pub(super) fn encoded(manifest: &WorkspaceManifest) -> Result<(String, String), Error> {
    manifest.validate().map_err(|_| Error::Conflict)?;
    let mut manifest = manifest.clone();
    manifest.files.sort_by(|a, b| a.file.path.cmp(&b.file.path));
    let json = serde_json::to_string(&manifest).map_err(|_| Error::Conflict)?;
    if json.len() > 32 * 1024 * 1024 {
        return Err(Error::Conflict);
    }
    Ok((format!("{:x}", Sha256::digest(json.as_bytes())), json))
}
pub(super) fn visible(db: &Connection, b: &Binding) -> Result<(), Error> {
    let allowed:bool=db.query_row("select exists(select 1 from agent_submissions s join agent_conversations c on c.id=s.conversation_id join agent_conversation_bindings b on b.conversation_id=c.id join manuscripts m on m.id=b.resource_id where s.conversation_id=?1 and s.request_id=?2 and s.turn_id=?3 and s.context_snapshot_id=?4 and b.resource_kind='manuscript' and b.resource_id=?5 and m.status='active' and c.archived=0)",params![b.conversation_id.to_string(),b.request_id.to_string(),b.turn_id.to_string(),b.context_snapshot_id.to_string(),b.manuscript_id.to_string()],|r|r.get(0))?;
    if allowed { Ok(()) } else { Err(Error::Denied) }
}
fn eligible(db: &Connection, b: &Binding) -> Result<(), Error> {
    visible(db, b)?;
    let finished: bool = db.query_row(
        "select exists(select 1 from agent_items where conversation_id=?1 and json_extract(payload_json,'$.payload.type')='turnFinished' and json_extract(payload_json,'$.payload.turnId')=?2)",
        params![b.conversation_id.to_string(), b.turn_id.to_string()], |row| row.get(0),
    )?;
    if finished { Err(Error::Denied) } else { Ok(()) }
}
/// Persist only after WorkspaceStore has durably created and verified the copy.
/// Retrying the same identity is harmless; changing a prepared copy is rejected.
pub fn prepare_in(
    tx: &Transaction<'_>,
    request_id: Uuid,
    manuscript_id: Uuid,
    manifest: &WorkspaceManifest,
) -> Result<Binding, Error> {
    discard::ensure_preparation_allowed(tx, manifest.identity.conversation_id, request_id)?;
    let record = super::submissions::read(tx, manifest.identity.conversation_id, request_id)
        .map_err(|_| Error::Storage)?
        .ok_or(Error::Conflict)?;
    if record.turn_id != manifest.identity.turn_id {
        return Err(Error::Conflict);
    }
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
    eligible(tx, &binding)?;
    let manuscript = manuscript_id.to_string();
    for source in &record.input.context {
        if source.kind != ContextSourceKind::Manuscript {
            return Err(Error::Conflict);
        }
        let anchor: serde_json::Value =
            serde_json::from_str(source.anchor.as_deref().ok_or(Error::Conflict)?)
                .map_err(|_| Error::Conflict)?;
        // A dynamic directory authority has no file version. Only the primary
        // grant frozen by writing preparation is excluded from file matching.
        if let Some(crate::agent::context_session::sources::Resource::Manuscript {
            resource_id,
            new_member_access: Some(_),
            ..
        }) = crate::agent::context_session::resource_grants::resource(source)
            && resource_id == manuscript
            && source.resource_id == manuscript
            && anchor["manuscriptId"] == manuscript
            && anchor["contextOwner"]["kind"] == "manuscript"
            && anchor["contextOwner"]["resourceId"] == manuscript
            && anchor["contextScope"] == anchor["resource"]
            && anchor["supplementalContext"] == false
        {
            continue;
        }
        let file = manifest
            .files
            .iter()
            .find(|file| file.file.path == source.resource_id)
            .ok_or(Error::Conflict)?;
        if source.revision != format!("sha256:{}", file.file.version.sha256)
            || anchor["manuscriptId"].as_str() != Some(manuscript_id.to_string().as_str())
            || anchor["diskSha256"]
                .as_str()
                .map(|hash| format!("sha256:{hash}"))
                != Some(file.source.revision.clone())
            || anchor["bufferRevision"].as_str().map(str::to_owned)
                != file
                    .source
                    .editor
                    .as_ref()
                    .map(|editor| editor.revision.to_string())
        {
            return Err(Error::Conflict);
        }
    }
    if manifest
        .files
        .iter()
        .any(|file| file.source.resource_id != format!("{manuscript_id}/{}", file.file.path))
    {
        return Err(Error::Conflict);
    }
    let previous:Option<(String,String,String)>=tx.query_row("select lease_id,manifest_hash,manuscript_id from agent_execution_workspaces where conversation_id=?1 and request_id=?2",params![binding.conversation_id.to_string(),request_id.to_string()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
    if let Some(previous) = previous {
        if previous
            != (
                binding.lease_id.to_string(),
                binding.manifest_hash.clone(),
                manuscript_id.to_string(),
            )
        {
            return Err(Error::Conflict);
        }
        return Ok(binding);
    }
    tx.execute("insert into agent_execution_workspaces(conversation_id,request_id,turn_id,context_snapshot_id,manuscript_id,lease_id,manifest_hash,manifest_json) values(?1,?2,?3,?4,?5,?6,?7,?8)",params![binding.conversation_id.to_string(),request_id.to_string(),binding.turn_id.to_string(),binding.context_snapshot_id.to_string(),manuscript_id.to_string(),binding.lease_id.to_string(),binding.manifest_hash,json])?;
    Ok(binding)
}
fn state(db: &Connection, b: &Binding) -> Result<String, Error> {
    eligible(db, b)?;
    let (state,json):(String,String)=db.query_row("select state,manifest_json from agent_execution_workspaces where conversation_id=?1 and request_id=?2 and turn_id=?3 and context_snapshot_id=?4 and manuscript_id=?5 and lease_id=?6 and manifest_hash=?7",params![b.conversation_id.to_string(),b.request_id.to_string(),b.turn_id.to_string(),b.context_snapshot_id.to_string(),b.manuscript_id.to_string(),b.lease_id.to_string(),b.manifest_hash],|r|Ok((r.get(0)?,r.get(1)?))).optional()?.ok_or(Error::Conflict)?;
    if format!("{:x}", Sha256::digest(json.as_bytes())) != b.manifest_hash {
        return Err(Error::Conflict);
    }
    Ok(state)
}
/// Decode identity-checked metadata independently of execution grant state.
fn read_metadata(
    db: &Connection,
    conversation: Uuid,
    request: Uuid,
) -> Result<Option<(Binding, WorkspaceManifest)>, Error> {
    discard::ensure_preparation_allowed(db, conversation, request)?;
    let saved = read_metadata_unscoped(db, conversation, request)?;
    if let Some((binding, _)) = &saved {
        visible(db, binding)?;
    }
    Ok(saved)
}
fn read_metadata_unscoped(
    db: &Connection,
    conversation: Uuid,
    request: Uuid,
) -> Result<Option<(Binding, WorkspaceManifest)>, Error> {
    let saved: Option<(String, String, String, String, String, String)> = db.query_row(
        "select turn_id,context_snapshot_id,manuscript_id,lease_id,manifest_hash,manifest_json from agent_execution_workspaces where conversation_id=?1 and request_id=?2",
        params![conversation.to_string(), request.to_string()],
        |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?)),
    ).optional()?;
    let Some((turn, context, manuscript, lease, hash, json)) = saved else {
        return Ok(None);
    };
    let id = |value: &str| Uuid::parse_str(value).map_err(|_| Error::Conflict);
    let binding = Binding {
        conversation_id: conversation,
        request_id: request,
        turn_id: id(&turn)?,
        context_snapshot_id: id(&context)?,
        manuscript_id: id(&manuscript)?,
        lease_id: id(&lease)?,
        manifest_hash: hash,
    };
    if json.len() > 32 * 1024 * 1024 {
        return Err(Error::Conflict);
    }
    let manifest: WorkspaceManifest = serde_json::from_str(&json).map_err(|_| Error::Conflict)?;
    if encoded(&manifest)?.0 != binding.manifest_hash
        || manifest.identity.conversation_id != conversation
        || manifest.identity.turn_id != binding.turn_id
        || manifest.identity.lease_id != binding.lease_id
    {
        return Err(Error::Conflict);
    }
    Ok(Some((binding, manifest)))
}
/// Execution restoration retains the stricter live, non-revoked requirement.
pub fn read(
    db: &Connection,
    conversation: Uuid,
    request: Uuid,
) -> Result<Option<(Binding, WorkspaceManifest)>, Error> {
    let saved = read_metadata(db, conversation, request)?;
    if let Some((binding, _)) = &saved
        && state(db, binding)? == "revoked"
    {
        return Err(Error::Denied);
    }
    Ok(saved)
}
/// Read metadata for change review after completion or withdrawal. This grants
/// neither execution nor writes. The host must still verify and lock the lease.
pub fn read_for_review(
    db: &Connection,
    conversation: Uuid,
    request: Uuid,
) -> Result<Option<(Binding, WorkspaceManifest)>, Error> {
    read_metadata(db, conversation, request)
}
/// Resolve only the exact retained binding, including a later generation.
pub fn read_binding_for_review(
    db: &Connection,
    binding: &Binding,
) -> Result<WorkspaceManifest, Error> {
    if let Some((original, manifest)) =
        read_for_review(db, binding.conversation_id, binding.request_id)?
    {
        if &original == binding {
            return Ok(manifest);
        }
        if original.lease_id == binding.lease_id {
            return Err(Error::Conflict);
        }
    }
    generation::read_for_review(db, binding)
}
/// Explicit user approval of this exact binding, not the disclosure confirmation.
pub fn approve_in(tx: &Transaction<'_>, binding: &Binding) -> Result<(), Error> {
    if state(tx, binding)? == "revoked" {
        return Err(Error::Denied);
    }
    tx.execute("update agent_execution_workspaces set state='approved' where conversation_id=?1 and request_id=?2",params![binding.conversation_id.to_string(),binding.request_id.to_string()])?;
    Ok(())
}
pub fn revoke_in(tx: &Transaction<'_>, conversation: Uuid, request: Uuid) -> Result<(), Error> {
    tx.execute("update agent_execution_workspaces set state='revoked' where conversation_id=?1 and request_id=?2",params![conversation.to_string(),request.to_string()])?;
    Ok(())
}
/// Revoke only the exact native binding, including on retry or after termination.
/// Unlike authorization this remains valid after archival/unbinding so that stale
/// resource visibility never prevents withdrawing an existing execution grant.
pub fn revoke_binding_in(tx: &Transaction<'_>, b: &Binding) -> Result<(), Error> {
    let changed = tx.execute(
        "update agent_execution_workspaces set state='revoked' where conversation_id=?1 and request_id=?2 and turn_id=?3 and context_snapshot_id=?4 and manuscript_id=?5 and lease_id=?6 and manifest_hash=?7",
        params![b.conversation_id.to_string(),b.request_id.to_string(),b.turn_id.to_string(),b.context_snapshot_id.to_string(),b.manuscript_id.to_string(),b.lease_id.to_string(),b.manifest_hash],
    )?;
    if changed != 1 {
        return Err(Error::Conflict);
    }
    Ok(())
}
/// Check on each operation; registry turn admission and sandbox still apply.
pub fn authorize(
    db: &Connection,
    binding: &Binding,
    manifest: &WorkspaceManifest,
) -> Result<(), Error> {
    if manifest.identity.conversation_id != binding.conversation_id
        || manifest.identity.turn_id != binding.turn_id
        || manifest.identity.lease_id != binding.lease_id
        || encoded(manifest)?.0 != binding.manifest_hash
    {
        return Err(Error::Conflict);
    }
    if state(db, binding)? == "approved" {
        Ok(())
    } else {
        Err(Error::Denied)
    }
}

#[cfg(test)]
#[path = "execution_workspaces/tests.rs"]
mod tests;
