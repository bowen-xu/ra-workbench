//! Native user consent only. Config hashing follows pinned Codex hook discovery
//! e363b08c9175ac1cbe5893615dd2cb9ddf95043b (Apache-2.0 / Copyright 2025 OpenAI).
//! SQLite/CAS adapts the existing instruction-trust port, without borrowing its
//! grants: permission to read instructions is not permission to execute hooks.
//! Blocking storage operations; never call from a synchronous hook invocation.
use crate::agent::{instructions::InstructionRootIdentity, model::AgentHost};
use research_agent_core::protocol::ModelIdentity;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use std::path::Path;
use uuid::Uuid;

const MAX_REVISION: u64 = 9_007_199_254_740_991;

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum TrustError {
    #[error("hook trust unavailable")]
    Storage,
    #[error("hook trust changed")]
    Conflict,
    #[error("invalid hook trust scope")]
    Invalid,
}

/// Native-derived configuration root only; not an IPC path or model tool input.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GrantKey {
    root: String,
    handler: Uuid,
    host: &'static str,
    model: ModelIdentity,
}
impl GrantKey {
    pub fn new(
        root: &Path,
        handler: Uuid,
        host: AgentHost,
        model: ModelIdentity,
    ) -> Result<Self, TrustError> {
        let root_text = root.to_str().ok_or(TrustError::Invalid)?;
        let valid = |value: &str| {
            !value.is_empty()
                && value.len() <= 256
                && value.trim() == value
                && !value.chars().any(char::is_control)
        };
        if !root.is_absolute()
            || root_text.len() > 4096
            || root_text.contains('\0')
            || handler.is_nil()
            || !valid(&model.provider_id)
            || !valid(&model.model_id)
            || model
                .revision
                .as_ref()
                .is_some_and(|revision| !valid(revision))
        {
            return Err(TrustError::Invalid);
        }
        Ok(Self {
            root: root_text.into(),
            handler,
            host: match host {
                // Independent chat has no executable workspace or hook grant.
                AgentHost::General => return Err(TrustError::Invalid),
                AgentHost::Writing => "writing",
                AgentHost::Reading => "reading",
                AgentHost::Review => "review",
                AgentHost::Notes => "notes",
            },
            model,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GrantState {
    key: GrantKey,
    pub revision: u64,
    pub granted: bool,
    device: u64,
    inode: u64,
    configuration_revision: String,
}
impl GrantState {
    /// Checking this loaded snapshot does no I/O. The owning native authority
    /// must serialize updates/revocation and publish them to live invocations.
    pub fn allows(
        &self,
        key: &GrantKey,
        root: &InstructionRootIdentity<'_>,
        configuration_revision: &str,
    ) -> bool {
        self.granted
            && &self.key == key
            && Some(key.root.as_str()) == root.canonical_path.to_str()
            && self.device == root.device
            && self.inode == root.inode
            && self.configuration_revision == configuration_revision
    }
}

pub fn read(db: &Connection, key: &GrantKey) -> Result<Option<GrantState>, TrustError> {
    let row = db.query_row(
        "select revision,granted,source_device,source_inode,configuration_revision from agent_compaction_hook_trust where source_root=?1 and handler_id=?2 and host=?3 and provider_id=?4 and model_id=?5 and model_revision=?6",
        params![key.root,key.handler.to_string(),key.host,key.model.provider_id,key.model.model_id,key.model.revision.as_deref().unwrap_or("")],
        |row| Ok((row.get::<_,u64>(0)?,row.get::<_,bool>(1)?,row.get::<_,String>(2)?,row.get::<_,String>(3)?,row.get::<_,String>(4)?)))
        .optional().map_err(|_| TrustError::Storage)?;
    row.map(
        |(revision, granted, device, inode, configuration_revision)| {
            Ok(GrantState {
                key: key.clone(),
                revision,
                granted,
                device: device.parse().map_err(|_| TrustError::Storage)?,
                inode: inode.parse().map_err(|_| TrustError::Storage)?,
                configuration_revision,
            })
        },
    )
    .transpose()
}

/// Explicit native user action only. Inputs contain identities/hashes, never
/// command text, environment secrets, transcript contents or credential handles.
pub fn update(
    db: &mut Connection,
    key: &GrantKey,
    expected_revision: u64,
    root: &InstructionRootIdentity<'_>,
    configuration_revision: &str,
    granted: bool,
) -> Result<GrantState, TrustError> {
    if Some(key.root.as_str()) != root.canonical_path.to_str()
        || configuration_revision.len() != 64
        || !configuration_revision
            .bytes()
            .all(|value| value.is_ascii_digit() || (b'a'..=b'f').contains(&value))
        || expected_revision >= MAX_REVISION
    {
        return Err(TrustError::Invalid);
    }
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| TrustError::Storage)?;
    if read(&tx, key)?.as_ref().map_or(0, |state| state.revision) != expected_revision {
        return Err(TrustError::Conflict);
    }
    let revision = expected_revision + 1;
    tx.execute("insert into agent_compaction_hook_trust(source_root,handler_id,host,provider_id,model_id,model_revision,source_device,source_inode,configuration_revision,revision,granted) values (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11) on conflict(source_root,handler_id,host,provider_id,model_id,model_revision) do update set source_device=excluded.source_device,source_inode=excluded.source_inode,configuration_revision=excluded.configuration_revision,revision=excluded.revision,granted=excluded.granted,updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now')",
        params![key.root,key.handler.to_string(),key.host,key.model.provider_id,key.model.model_id,key.model.revision.as_deref().unwrap_or(""),root.device.to_string(),root.inode.to_string(),configuration_revision,revision,granted])
        .map_err(|_| TrustError::Storage)?;
    tx.commit().map_err(|_| TrustError::Storage)?;
    Ok(GrantState {
        key: key.clone(),
        revision,
        granted,
        device: root.device,
        inode: root.inode,
        configuration_revision: configuration_revision.into(),
    })
}

/// Revocation works after the source directory/configuration has disappeared.
/// Keep the row and advance its version, so an old approval cannot recreate it.
pub fn revoke(
    db: &mut Connection,
    key: &GrantKey,
    expected_revision: u64,
) -> Result<GrantState, TrustError> {
    if expected_revision == 0 || expected_revision >= MAX_REVISION {
        return Err(TrustError::Invalid);
    }
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| TrustError::Storage)?;
    let mut state = read(&tx, key)?.ok_or(TrustError::Conflict)?;
    if state.revision != expected_revision {
        return Err(TrustError::Conflict);
    }
    state.revision += 1;
    state.granted = false;
    tx.execute("update agent_compaction_hook_trust set granted=0,revision=?7,updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') where source_root=?1 and handler_id=?2 and host=?3 and provider_id=?4 and model_id=?5 and model_revision=?6",
        params![key.root,key.handler.to_string(),key.host,key.model.provider_id,key.model.model_id,key.model.revision.as_deref().unwrap_or(""),state.revision]).map_err(|_| TrustError::Storage)?;
    tx.commit().map_err(|_| TrustError::Storage)?;
    Ok(state)
}

#[cfg(test)]
#[path = "compaction_hook_trust/tests.rs"]
mod tests;
