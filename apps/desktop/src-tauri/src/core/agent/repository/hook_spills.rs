//! Native receipts are evidence; only completed context grants model access.
//! Call on the existing blocking storage owner, never from renderer/model data.
use super::context_windows::{self, Error};
use research_agent_core::{
    context::{compaction_hooks::*, compaction_strategy::CompactionPhase},
    protocol::ModelIdentity,
    workspace::FileVersion,
};
use rusqlite::{Connection, params};
use sha2::{Digest, Sha256};
use uuid::Uuid;

fn phase(request: &CompactionHookRequest) -> &'static str {
    match request.phase {
        CompactionPhase::Manual => "manual",
        CompactionPhase::PreTurn => "pre_turn",
        CompactionPhase::MidTurn => "mid_turn",
    }
}
fn model(request: &CompactionHookRequest) -> Result<String, Error> {
    if request.stage != CompactionHookStage::SessionStartCompact
        || request.conversation_id.is_nil()
        || request.operation_id.is_nil()
        || request.turn_id.is_nil()
        || request.model.provider_id.trim().is_empty()
        || request.model.model_id.trim().is_empty()
    {
        return Err(Error::Invalid);
    }
    let model = serde_json::to_string(&request.model).map_err(|_| Error::Invalid)?;
    if model.len() > 16384 {
        return Err(Error::Invalid);
    }
    Ok(model)
}
fn current(db: &Connection, request: &CompactionHookRequest) -> Result<(), Error> {
    let history = context_windows::read_projection(db, request.conversation_id)?;
    let (count, archived): (u64, bool) = db.query_row(
        "select durable_count,archived from agent_conversations where id=?1",
        [request.conversation_id.to_string()],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    if archived || history.active_turn() != Some(request.turn_id) || count != history.seq() {
        return Err(Error::Inactive);
    }
    let active =
        context_windows::read_active(db, request.conversation_id)?.ok_or(Error::Conflict)?;
    if !context_windows::is_ancestor(
        db,
        request.conversation_id,
        active.window_id,
        request.operation_id,
    )? {
        return Err(Error::Conflict);
    }
    Ok(())
}
pub fn locator(id: Uuid) -> String {
    format!("hook-artifact://{id}")
}
/// Current owned invocation boundary, never supplied by the model tool caller.
pub(crate) fn invocation_cutoff(
    db: &Connection,
    conversation: Uuid,
    turn: Uuid,
) -> Result<u64, Error> {
    let history = context_windows::read_projection(db, conversation)?;
    let (count, archived): (u64, bool) = db.query_row(
        "select durable_count,archived from agent_conversations where id=?1",
        [conversation.to_string()],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    if archived || count != history.seq() || history.active_turn() != Some(turn) {
        return Err(Error::Inactive);
    }
    Ok(history.seq())
}
pub(crate) fn validate_request(
    db: &Connection,
    request: &CompactionHookRequest,
) -> Result<(), Error> {
    model(request)?;
    current(db, request)
}
fn digest(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

/// Caller has verified and registered the full immutable object under content_write.
/// This creates no grant, and deliberately survives origin deletion for fork evidence.
pub fn save_receipt(
    db: &Connection,
    request: &CompactionHookRequest,
    id: Uuid,
    version: &FileVersion,
) -> Result<(), Error> {
    let identity = model(request)?;
    current(db, request)?;
    if id.is_nil()
        || version.executable
        || version.byte_size > 32 * 1024 * 1024
        || version.sha256.len() != 64
        || !version
            .sha256
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(Error::Invalid);
    }
    let size: u64 = db.query_row(
        "select byte_size from content_objects where sha256=?1",
        [&version.sha256],
        |r| r.get(0),
    )?;
    if size != version.byte_size {
        return Err(Error::Conflict);
    }
    db.execute("insert into agent_hook_spill_artifacts(id,source_conversation_id,source_window_id,turn_id,phase,model_json,content_sha256) values(?1,?2,?3,?4,?5,?6,?7)",params![id.to_string(),request.conversation_id.to_string(),request.operation_id.to_string(),request.turn_id.to_string(),phase(request),identity,version.sha256])?;
    Ok(())
}

/// Exact native identity and preview are immutable; retries may repeat the same binding.
pub fn bind_preview(
    db: &Connection,
    request: &CompactionHookRequest,
    id: Uuid,
    preview: &str,
) -> Result<(), Error> {
    let identity = model(request)?;
    current(db, request)?;
    if preview.len() > 1024 * 1024
        || !preview.ends_with(&format!("\n\nFull hook output saved to: {}", locator(id)))
    {
        return Err(Error::Invalid);
    }
    let exists:bool=db.query_row("select exists(select 1 from agent_hook_spill_artifacts where id=?1 and source_conversation_id=?2 and source_window_id=?3 and turn_id=?4 and phase=?5 and model_json=?6)",params![id.to_string(),request.conversation_id.to_string(),request.operation_id.to_string(),request.turn_id.to_string(),phase(request),identity],|r|r.get(0))?;
    if !exists {
        return Err(Error::Conflict);
    }
    let hash = digest(preview);
    db.execute("insert into agent_hook_spill_previews(artifact_id,preview_sha256) values(?1,?2) on conflict(artifact_id) do nothing",params![id.to_string(),hash])?;
    let saved: String = db.query_row(
        "select preview_sha256 from agent_hook_spill_previews where artifact_id=?1",
        [id.to_string()],
        |r| r.get(0),
    )?;
    if saved != hash {
        return Err(Error::Conflict);
    }
    Ok(())
}

fn supported(db: &Connection) -> Result<bool, Error> {
    Ok(db.query_row("select exists(select 1 from sqlite_master where type='table' and name='agent_hook_spill_artifacts')",[],|r|r.get(0))?)
}

/// Runs inside the original context completion transaction after its context insert.
pub(crate) fn activate(
    db: &Connection,
    request: &CompactionHookRequest,
    audit: i64,
    boundary: u64,
    contexts: &[String],
) -> Result<(), Error> {
    if !supported(db)? {
        return Ok(());
    } // Older imported test/schema adapters have no artifacts.
    let identity = model(request)?;
    let active =
        context_windows::read_active(db, request.conversation_id)?.ok_or(Error::Conflict)?;
    if !context_windows::is_ancestor(
        db,
        request.conversation_id,
        active.window_id,
        request.operation_id,
    )? {
        return Err(Error::Conflict);
    }
    for text in contexts {
        db.execute("insert into agent_hook_spill_grants(conversation_id,window_id,source_audit_seq,artifact_id,turn_id,boundary_seq) select ?1,?9,?3,a.id,?4,?5 from agent_hook_spill_artifacts a join agent_hook_spill_previews p on p.artifact_id=a.id where a.source_conversation_id=?1 and a.source_window_id=?2 and a.turn_id=?4 and a.model_json=?6 and a.phase=?7 and p.preview_sha256=?8 on conflict(conversation_id,window_id,source_audit_seq,artifact_id) do nothing",params![request.conversation_id.to_string(),request.operation_id.to_string(),audit,request.turn_id.to_string(),boundary,identity,phase(request),digest(text),active.window_id.to_string()])?;
    }
    Ok(())
}

/// The caller has just inherited this exact context in the child's fork transaction.
pub(crate) fn inherit(
    db: &Connection,
    parent: Uuid,
    child: Uuid,
    windows: (Uuid, Uuid),
    context: (u64, Uuid, u64),
) -> Result<(), Error> {
    let (source, target) = windows;
    let (audit, turn, boundary) = context;
    if !supported(db)? {
        return Ok(());
    }
    db.execute("insert into agent_hook_spill_grants(conversation_id,window_id,source_audit_seq,artifact_id,turn_id,boundary_seq) select ?1,?2,source_audit_seq,artifact_id,turn_id,boundary_seq from agent_hook_spill_grants where conversation_id=?3 and window_id=?4 and source_audit_seq=?5 and turn_id=?6 and boundary_seq=?7",params![child.to_string(),target.to_string(),parent.to_string(),source.to_string(),audit,turn.to_string(),boundary])?;
    Ok(())
}

/// Native read metadata, not a filesystem permission. Caller must independently
/// authorize the current host and verify full bytes/hash under content_read.
/// Window rollover may cover context while its surviving turn still needs evidence.
pub fn resolve(
    db: &Connection,
    conversation: Uuid,
    id: Uuid,
    model: &ModelIdentity,
    through: u64,
) -> Result<(FileVersion, String), Error> {
    let history = context_windows::read_projection(db, conversation)?;
    if through > history.seq() {
        return Err(Error::Conflict);
    }
    let archived: bool = db.query_row(
        "select archived from agent_conversations where id=?1",
        [conversation.to_string()],
        |r| r.get(0),
    )?;
    if archived {
        return Err(Error::Inactive);
    }
    let identity = serde_json::to_string(model).map_err(|_| Error::Invalid)?;
    let mut query=db.prepare("select g.turn_id,c.sha256,c.byte_size,c.relative_path from agent_hook_spill_grants g join agent_hook_spill_artifacts a on a.id=g.artifact_id join content_objects c on c.sha256=a.content_sha256 where g.conversation_id=?1 and g.artifact_id=?2 and a.model_json=?3 and g.boundary_seq<=?4 order by g.boundary_seq desc")?;
    let mut rows = query.query(params![
        conversation.to_string(),
        id.to_string(),
        identity,
        through
    ])?;
    while let Some(row) = rows.next()? {
        let turn = Uuid::parse_str(&row.get::<_, String>(0)?).map_err(|_| Error::Invalid)?;
        if history.turn(turn).is_some() {
            return Ok((
                FileVersion {
                    sha256: row.get(1)?,
                    byte_size: row.get(2)?,
                    executable: false,
                },
                row.get(3)?,
            ));
        }
    }
    Err(Error::Inactive)
}
