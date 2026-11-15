//! Native hook configuration; parsing is not execution authorization.
//! Trust/version separation adapted from Codex hooks/src/engine/discovery.rs
//! and config_rules.rs, e363b08c9175ac1cbe5893615dd2cb9ddf95043b, Apache-2.0.
//! Copyright 2025 OpenAI. No automatic project discovery or renderer command API.
use super::{compaction_hook_process::*, process::ProcessSpec};
use research_agent_core::{context::compaction_hooks::*, protocol::ModelIdentity};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashSet},
    ffi::OsString,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use uuid::Uuid;

#[path = "compaction_hook_config/consent.rs"]
pub mod consent;
#[path = "compaction_hook_config/factory.rs"]
pub mod factory;
#[cfg(target_os = "macos")]
#[path = "compaction_hook_config/native.rs"]
pub mod native;
#[path = "compaction_hook_config/source.rs"]
pub mod source;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("invalid or unavailable hook configuration")]
pub struct HookConfigError;

/// Live host authority, consulted immediately before launching a frozen plan.
/// Implementations must use already-loaded state: no blocking I/O or side effects.
/// A configuration revision is a binding, not an execution permission.
pub trait HookInvocationAuthority: Send + Sync {
    fn check(
        &self,
        handler_id: Uuid,
        configuration_revision: &str,
        request: &CompactionHookRequest,
    ) -> Result<(), HookConfigError>;
}

/// Frozen parsed data. No Debug/serialization that could expose command/env text.
pub struct HookConfiguration {
    revision: String,
    handlers: Vec<ConfiguredHook>,
}
pub struct ConfiguredHook {
    pub id: Uuid,
    pub stage: ConfiguredHookStage,
    pub enabled: bool,
    matcher: Option<String>,
    program: PathBuf,
    arguments: Vec<OsString>,
    directory: Option<PathBuf>,
    environment: BTreeMap<OsString, OsString>,
    timeout: Duration,
    output_bytes: usize,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfiguredHookStage {
    Compaction(CompactionHookStage),
    Stop,
    SubagentStop,
}
impl ConfiguredHookStage {
    fn compaction(self) -> Option<CompactionHookStage> {
        match self {
            Self::Compaction(stage) => Some(stage),
            Self::Stop | Self::SubagentStop => None,
        }
    }
}
impl ConfiguredHook {
    /// Candidate only. Host must authorize/sandbox it before returning a plan.
    pub fn candidate(&self) -> Result<ProcessSpec, HookConfigError> {
        let directory = self.directory.as_ref().ok_or(HookConfigError)?;
        Ok(self.spec(directory))
    }
    /// Bind an omitted cwd to a native-authorized workspace. An explicit cwd
    /// never overrides the host scope; no filesystem access or grant occurs here.
    pub fn candidate_in(&self, workspace: &Path) -> Result<ProcessSpec, HookConfigError> {
        if !workspace.is_absolute()
            || workspace.components().collect::<PathBuf>().as_os_str() != workspace.as_os_str()
            || workspace.components().any(|component| {
                matches!(
                    component,
                    std::path::Component::CurDir | std::path::Component::ParentDir
                )
            })
            || self
                .directory
                .as_deref()
                .is_some_and(|directory| directory != workspace)
        {
            return Err(HookConfigError);
        }
        Ok(self.spec(workspace))
    }
    fn spec(&self, directory: &Path) -> ProcessSpec {
        ProcessSpec {
            program: self.program.clone(),
            arguments: self.arguments.clone(),
            directory: directory.to_path_buf(),
            environment: self.environment.clone(),
        }
    }
}
impl HookConfiguration {
    pub fn revision(&self) -> &str {
        &self.revision
    }
    pub fn handlers(&self) -> &[ConfiguredHook] {
        &self.handlers
    }

    /// Freeze a host-authorized plan per enabled handler. A denial is an error,
    /// never implicit execution or silent disabling. Run authorization off the
    /// executor if the host policy needs I/O. Nothing launches during this call.
    /// The separate live authority remains mandatory for every later invocation.
    pub fn authorize(
        &self,
        model: ModelIdentity,
        grant: impl FnMut(&ConfiguredHook, &str) -> Result<AuthorizedHookCommand, HookConfigError>,
        observer: Arc<dyn HookHandlerObserver>,
        authority: Arc<dyn HookInvocationAuthority>,
    ) -> Result<ConcurrentCompactionHooks, HookConfigError> {
        ConcurrentCompactionHooks::new(self.authorized_handlers(model, grant, authority)?, observer)
            .map_err(|_| HookConfigError)
    }

    pub(crate) fn authorized_handlers(
        &self,
        model: ModelIdentity,
        mut grant: impl FnMut(&ConfiguredHook, &str) -> Result<AuthorizedHookCommand, HookConfigError>,
        authority: Arc<dyn HookInvocationAuthority>,
    ) -> Result<Vec<CompactionHookHandler>, HookConfigError> {
        if model.provider_id.trim().is_empty() || model.model_id.trim().is_empty() {
            return Err(HookConfigError);
        }
        let mut handlers = Vec::new();
        for configured in self.handlers.iter().filter(|handler| handler.enabled) {
            let Some(stage) = configured.stage.compaction() else {
                continue;
            };
            // This legacy factory has no request-time consent preparation.
            if configured.matcher.is_some() {
                return Err(HookConfigError);
            }
            let plan = grant(configured, &self.revision)?;
            if !plan.process.program.is_absolute() || !plan.process.directory.is_absolute() {
                return Err(HookConfigError);
            }
            let source = Arc::new(FrozenCommand {
                handler_id: configured.id,
                revision: self.revision.clone(),
                authority: authority.clone(),
                model: model.clone(),
                stage,
                plan,
            });
            handlers.push(CompactionHookHandler {
                id: configured.id,
                stage,
                hooks: Arc::new(
                    ProcessCompactionHooks::new(
                        source,
                        configured.timeout,
                        configured.output_bytes,
                    )
                    .map_err(|_| HookConfigError)?,
                ),
            });
        }
        Ok(handlers)
    }
}

struct FrozenCommand {
    handler_id: Uuid,
    revision: String,
    authority: Arc<dyn HookInvocationAuthority>,
    model: ModelIdentity,
    stage: CompactionHookStage,
    plan: AuthorizedHookCommand,
}
impl CompactionHookCommandSource for FrozenCommand {
    fn prepare(
        &self,
        request: &CompactionHookRequest,
    ) -> Result<Option<AuthorizedHookCommand>, HookCommandError> {
        if request.model != self.model || request.stage != self.stage {
            return Err(HookCommandError);
        }
        self.authority
            .check(self.handler_id, &self.revision, request)
            .map_err(|_| HookCommandError)?;
        Ok(Some(AuthorizedHookCommand {
            process: ProcessSpec {
                program: self.plan.process.program.clone(),
                arguments: self.plan.process.arguments.clone(),
                directory: self.plan.process.directory.clone(),
                environment: self.plan.process.environment.clone(),
            },
            transcript_path: self.plan.transcript_path.clone(),
            launcher: self.plan.launcher.clone(),
        }))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    schema_version: u32,
    #[serde(default)]
    handlers: Vec<Handler>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Handler {
    id: String,
    stage: String,
    #[serde(default)]
    enabled: bool,
    matcher: Option<String>,
    program: String,
    directory: Option<String>,
    #[serde(default)]
    arguments: Vec<String>,
    #[serde(default)]
    environment: BTreeMap<String, String>,
    timeout_ms: u64,
    max_output_bytes: usize,
}

/// Exact bytes including comments determine the approval version. Error text
/// never forwards TOML diagnostics, which could quote private values.
pub fn parse(text: &str) -> Result<HookConfiguration, HookConfigError> {
    if text.len() > 256 * 1024 {
        return Err(HookConfigError);
    }
    let document: Document = toml::from_str(text).map_err(|_| HookConfigError)?;
    if document.schema_version != 1 || document.handlers.len() > 32 {
        return Err(HookConfigError);
    }
    let mut ids = HashSet::new();
    let mut handlers = Vec::new();
    for handler in document.handlers {
        let id = Uuid::parse_str(&handler.id).map_err(|_| HookConfigError)?;
        if id.is_nil()
            || !ids.insert(id)
            || handler.arguments.len() > 64
            || handler.environment.len() > 64
            || handler.timeout_ms == 0
            || handler.timeout_ms > 24 * 60 * 60 * 1000
            || !(1..=1024 * 1024).contains(&handler.max_output_bytes)
        {
            return Err(HookConfigError);
        }
        let stage = match handler.stage.as_str() {
            "pre_compact" => ConfiguredHookStage::Compaction(CompactionHookStage::PreCompact),
            "post_compact" => ConfiguredHookStage::Compaction(CompactionHookStage::PostCompact),
            "session_start_compact" => {
                ConfiguredHookStage::Compaction(CompactionHookStage::SessionStartCompact)
            }
            "stop" => ConfiguredHookStage::Stop,
            "subagent_stop" => ConfiguredHookStage::SubagentStop,
            _ => return Err(HookConfigError),
        };
        // Codex skips a group with an invalid applicable matcher. Stop ignores
        // its matcher.
        if stage != ConfiguredHookStage::Stop
            && handler.matcher.as_deref().is_some_and(|matcher| {
                !matcher_all(matcher)
                    && !matcher_exact(matcher)
                    && regex::Regex::new(matcher).is_err()
            })
        {
            eprintln!("Skipping hook with invalid matcher");
            continue;
        }
        let valid = |text: &str| text.len() <= 4096 && !text.contains('\0');
        if !valid(&handler.program)
            || handler
                .directory
                .as_deref()
                .is_some_and(|directory| !valid(directory))
            || handler.arguments.iter().any(|text| !valid(text))
            || handler.environment.iter().any(|(key, value)| {
                key.is_empty() || key.contains('=') || !valid(key) || !valid(value)
            })
        {
            return Err(HookConfigError);
        }
        let program = PathBuf::from(handler.program);
        let directory = handler.directory.map(PathBuf::from);
        if !program.is_absolute()
            || directory
                .as_deref()
                .is_some_and(|directory| !directory.is_absolute())
        {
            return Err(HookConfigError);
        }
        handlers.push(ConfiguredHook {
            id,
            stage,
            enabled: handler.enabled,
            matcher: handler.matcher,
            program,
            directory,
            arguments: handler.arguments.into_iter().map(OsString::from).collect(),
            environment: handler
                .environment
                .into_iter()
                .map(|(key, value)| (key.into(), value.into()))
                .collect(),
            timeout: Duration::from_millis(handler.timeout_ms),
            output_bytes: handler.max_output_bytes,
        });
    }
    Ok(HookConfiguration {
        revision: format!("{:x}", Sha256::digest(text.as_bytes())),
        handlers,
    })
}

fn matcher_all(matcher: &str) -> bool {
    matcher.is_empty() || matcher == "*"
}

fn matcher_exact(matcher: &str) -> bool {
    matcher
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '|')
}

fn matches_hook_matcher(matcher: Option<&str>, agent_type: &str) -> bool {
    match matcher {
        None => true,
        Some(matcher) if matcher_all(matcher) => true,
        Some(matcher) if matcher_exact(matcher) => {
            matcher.split('|').any(|part| part == agent_type)
        }
        Some(matcher) => {
            regex::Regex::new(matcher).is_ok_and(|pattern| pattern.is_match(agent_type))
        }
    }
}

#[cfg(test)]
#[path = "compaction_hook_config/tests.rs"]
mod tests;
