//! User settings only, never model tools. Native app config is the only source;
//! IPC carries identities/version fingerprints, never paths or executable plans.
use super::{
    compaction_hook_config::{self as config, ConfiguredHookStage, consent::SqliteHookConsent},
    instructions::InstructionRootIdentity,
    model::AgentHost,
    repository::compaction_hook_trust::{GrantKey, TrustError},
};
use crate::storage::AppStore;
use research_agent_core::{
    context::compaction_hooks::CompactionHookStage, protocol::ModelIdentity,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{path::PathBuf, sync::Arc};
use tauri::State;
use uuid::Uuid;

#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Host {
    Writing,
    Reading,
    Review,
    Notes,
}
impl Host {
    fn native(self) -> AgentHost {
        match self {
            Self::Writing => AgentHost::Writing,
            Self::Reading => AgentHost::Reading,
            Self::Review => AgentHost::Review,
            Self::Notes => AgentHost::Notes,
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Scope {
    pub host: Host,
    pub model: ModelIdentity,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApproveRequest {
    pub scope: Scope,
    pub handler_id: Uuid,
    pub expected_revision: u64,
    pub fingerprint: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RevokeRequest {
    pub scope: Scope,
    pub handler_id: Uuid,
    pub expected_revision: u64,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Review {
    pub schema_version: u8,
    pub handlers: Vec<HandlerReview>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HandlerReview {
    pub handler_id: Uuid,
    pub stage: &'static str,
    pub enabled: bool,
    pub fingerprint: String,
    pub revision: u64,
    pub granted: bool,
    pub matches_current: bool,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Decision {
    pub schema_version: u8,
    pub revision: u64,
    pub granted: bool,
}
#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Error {
    InvalidInput,
    Conflict,
    Unavailable,
}
impl From<TrustError> for Error {
    fn from(value: TrustError) -> Self {
        match value {
            TrustError::Invalid => Self::InvalidInput,
            TrustError::Conflict => Self::Conflict,
            TrustError::Storage => Self::Unavailable,
        }
    }
}
pub(crate) struct Snapshot {
    pub(crate) path: PathBuf,
    pub(crate) device: u64,
    pub(crate) inode: u64,
    pub(crate) configuration: config::HookConfiguration,
}
impl Snapshot {
    pub(crate) fn identity(&self) -> InstructionRootIdentity<'_> {
        InstructionRootIdentity {
            canonical_path: &self.path,
            device: self.device,
            inode: self.inode,
        }
    }
    fn fingerprint(&self, scope: &Scope, id: Uuid) -> Result<String, Error> {
        let bytes = serde_json::to_vec(&(
            1u8,
            &self.path,
            self.device,
            self.inode,
            self.configuration.revision(),
            scope,
            id,
        ))
        .map_err(|_| Error::Unavailable)?;
        Ok(format!("{:x}", Sha256::digest(bytes)))
    }
}

pub struct HookConsentService {
    root: PathBuf,
    consent: Arc<SqliteHookConsent>,
    #[cfg(unix)]
    store: AppStore,
}
impl HookConsentService {
    pub(crate) fn from_app_store(store: &AppStore) -> Self {
        Self {
            root: store.agent_configuration_directory().into(),
            consent: Arc::new(SqliteHookConsent::new(store.shared_agent_connection())),
            #[cfg(unix)]
            store: store.clone(),
        }
    }
    /// Share this exact controller with the app's factory; do not construct a
    /// second controller from its database or publish settings through raw SQL.
    pub fn consent(&self) -> Arc<SqliteHookConsent> {
        self.consent.clone()
    }
    /// Native DI only; creating an adapter does not grant a model read access.
    #[cfg(unix)]
    pub fn output_store(&self) -> Arc<dyn super::compaction_hook_spill::HookSpillStore> {
        Arc::new(
            super::compaction_hook_spill::native::NativeHookSpillStore::new(self.store.clone()),
        )
    }
    #[cfg(unix)]
    pub fn output_reader(&self) -> Arc<dyn super::compaction_hook_spill::HookArtifactReader> {
        Arc::new(
            super::compaction_hook_spill::native::NativeHookSpillStore::new(self.store.clone()),
        )
    }
    fn key(&self, scope: &Scope, id: Uuid) -> Result<GrantKey, Error> {
        GrantKey::new(&self.root, id, scope.host.native(), scope.model.clone()).map_err(Into::into)
    }
    pub(crate) fn snapshot(&self) -> Result<Option<Snapshot>, Error> {
        match std::fs::symlink_metadata(&self.root) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(Error::Unavailable),
            Ok(_) => {}
        }
        let mut identity = None;
        let configuration = config::source::load(&self.root, |root| {
            identity = Some((root.device, root.inode));
            Ok(()) // Native-owned app settings root only, not a selected path.
        })
        .map_err(|_| Error::Unavailable)?;
        configuration
            .map(|configuration| {
                let (device, inode) = identity.ok_or(Error::Unavailable)?;
                Ok(Snapshot {
                    path: self.root.clone(),
                    device,
                    inode,
                    configuration,
                })
            })
            .transpose()
    }
    /// Blocking worker only. Metadata does not disclose config command/env data;
    /// a future settings UI must provide native inspection before approving it.
    pub fn review(&self, scope: Scope) -> Result<Review, Error> {
        self.key(&scope, Uuid::from_u128(1))?;
        let mut handlers = Vec::new();
        if let Some(snapshot) = self.snapshot()? {
            for handler in snapshot.configuration.handlers() {
                let key = self.key(&scope, handler.id)?;
                let state = self.consent.read(&key)?;
                handlers.push(HandlerReview {
                    handler_id: handler.id,
                    stage: match handler.stage {
                        ConfiguredHookStage::Compaction(CompactionHookStage::PreCompact) => {
                            "preCompact"
                        }
                        ConfiguredHookStage::Compaction(CompactionHookStage::PostCompact) => {
                            "postCompact"
                        }
                        ConfiguredHookStage::Compaction(
                            CompactionHookStage::SessionStartCompact,
                        ) => "sessionStartCompact",
                        ConfiguredHookStage::Stop => "stop",
                        ConfiguredHookStage::SubagentStop => "subagentStop",
                    },
                    enabled: handler.enabled,
                    fingerprint: snapshot.fingerprint(&scope, handler.id)?,
                    revision: state.as_ref().map_or(0, |state| state.revision),
                    granted: state.as_ref().is_some_and(|state| state.granted),
                    matches_current: handler.enabled
                        && state.as_ref().is_some_and(|state| {
                            state.allows(
                                &key,
                                &snapshot.identity(),
                                snapshot.configuration.revision(),
                            )
                        }),
                });
            }
        }
        Ok(Review {
            schema_version: 1,
            handlers,
        })
    }
    pub fn approve(&self, request: ApproveRequest) -> Result<Decision, Error> {
        let key = self.key(&request.scope, request.handler_id)?;
        if request.fingerprint.len() != 64
            || !request
                .fingerprint
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        {
            return Err(Error::InvalidInput);
        }
        let snapshot = self.snapshot()?.ok_or(Error::Conflict)?;
        if !snapshot
            .configuration
            .handlers()
            .iter()
            .any(|handler| handler.id == request.handler_id && handler.enabled)
            || snapshot.fingerprint(&request.scope, request.handler_id)? != request.fingerprint
        {
            return Err(Error::Conflict);
        }
        let state = self.consent.update(
            &key,
            request.expected_revision,
            &snapshot.identity(),
            snapshot.configuration.revision(),
            true,
        )?;
        Ok(Decision {
            schema_version: 1,
            revision: state.revision,
            granted: state.granted,
        })
    }
    pub fn revoke(&self, request: RevokeRequest) -> Result<Decision, Error> {
        let key = self.key(&request.scope, request.handler_id)?;
        let state = self.consent.revoke(&key, request.expected_revision)?;
        Ok(Decision {
            schema_version: 1,
            revision: state.revision,
            granted: state.granted,
        })
    }
}

#[tauri::command]
pub async fn agent_review_compaction_hooks(
    service: State<'_, Arc<HookConsentService>>,
    scope: Scope,
) -> Result<Review, Error> {
    let service = service.inner().clone();
    tokio::task::spawn_blocking(move || service.review(scope))
        .await
        .map_err(|_| Error::Unavailable)?
}
#[tauri::command]
pub async fn agent_approve_compaction_hook(
    service: State<'_, Arc<HookConsentService>>,
    request: ApproveRequest,
) -> Result<Decision, Error> {
    let service = service.inner().clone();
    tokio::task::spawn_blocking(move || service.approve(request))
        .await
        .map_err(|_| Error::Unavailable)?
}
#[tauri::command]
pub async fn agent_revoke_compaction_hook(
    service: State<'_, Arc<HookConsentService>>,
    request: RevokeRequest,
) -> Result<Decision, Error> {
    let service = service.inner().clone();
    tokio::task::spawn_blocking(move || service.revoke(request))
        .await
        .map_err(|_| Error::Unavailable)?
}
#[cfg(test)]
#[path = "compaction_hook_commands/tests.rs"]
mod tests;
