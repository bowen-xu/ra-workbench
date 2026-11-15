//! Replay the actual read prefix inside each original preparation transaction.
use super::resources::Resources;
use crate::{
    agent::writing_context::{
        ContextNeed, ContextRead, WritingContextError as Error, WritingContextReader,
        WritingContextRequest,
    },
    ai::structured_resource_validation::{DecisionOrigin, Failure, FailureCause, Flow},
    data_interface::plugin_resources::PluginDataError,
    runtime_plugins::native::{DecidedProject, NativeHostError, NativeProfile, NativeResult},
};
use research_agent_core::submission::ContextText;
use research_workbench_native_sdk::streaming_payload::{self, PayloadDigest};
use research_workbench_native_sdk::{
    writing_context::{ContextDecision, ContextInput},
    writing_project::{Decision, Input},
};
use rusqlite::Connection;
use std::{path::Path, sync::Arc};
use uuid::Uuid;

struct Cached {
    fingerprint: PayloadDigest,
    decision: Arc<Decision>,
    origin: DecisionOrigin,
}
pub(super) struct Reader {
    resources: Resources,
    scope: Option<
        Result<
            super::scope::ScopeResources,
            crate::agent::context_catalog::ContextCatalogReadError,
        >,
    >,
    key: String,
    cursor: usize,
    decisions: Vec<Cached>,
}
impl Reader {
    #[cfg(unix)]
    pub(super) fn workspace_checkpoint(
        &mut self,
        db: &Connection,
        resources: &super::super::files::checkpoints::Resources,
        id: Uuid,
        path: &str,
    ) -> ContextRead<(Arc<Decision>, DecisionOrigin)> {
        use research_workbench_native_sdk::writing_checkpoint::CheckpointInput;
        let rows = resources.read(db, &id.to_string(), path).map_err(data)?;
        self.decide(Input::Checkpoint {
            input: Box::new(CheckpointInput::Read {
                manuscript_id: id.to_string(),
                relative_path: path.into(),
                required: false,
                rows,
            }),
        })
    }
    #[cfg(unix)]
    pub(super) fn agent_file_policy(
        &mut self,
        path: &str,
    ) -> ContextRead<(
        research_workbench_native_sdk::writing_file::AgentFilePolicy,
        DecisionOrigin,
    )> {
        let (decision, origin) = self.decide(Input::PrepareAgentFileChanges {
            relative_paths: vec![path.into()],
        })?;
        match decision.as_ref() {
            Decision::AgentFileChangesPrepared { policies }
                if policies.len() == 1 && policies[0].relative_path == path =>
            {
                if !policies[0].editable {
                    return Err(native(Error::InvalidInput, origin));
                }
                Ok((policies[0].clone(), origin))
            }
            _ => Err(native(Error::Unavailable, origin)),
        }
    }
    #[cfg(unix)]
    pub(super) fn workspace_files(
        &mut self,
        db: &Connection,
        id: Uuid,
    ) -> ContextRead<Vec<research_workbench_native_sdk::writing_project::ManuscriptFile>> {
        let manuscript_id = id.to_string();
        let manuscript = self
            .resources
            .files
            .projects
            .get(db, &manuscript_id)
            .map_err(data)?;
        self.context(ContextInput::Project {
            manuscript_id: id,
            rows: manuscript.clone(),
        })?;
        let rows = self
            .resources
            .files
            .files(db, &manuscript_id)
            .map_err(data)?;
        let (decision, origin) = self.decide(Input::ListFiles {
            manuscript_id,
            manuscript,
            files: Some(rows),
        })?;
        let Decision::Files { files } = decision.as_ref() else {
            return Err(native(Error::Unavailable, origin));
        };
        Ok(files.clone())
    }

    pub fn new(resources: Resources, key: String) -> Self {
        Self {
            resources,
            scope: None,
            key,
            cursor: 0,
            decisions: Vec::new(),
        }
    }
    pub(super) fn with_scope(
        mut self,
        scope: Option<
            Result<
                super::scope::ScopeResources,
                crate::agent::context_catalog::ContextCatalogReadError,
            >,
        >,
    ) -> Self {
        self.scope = scope;
        self
    }
    pub fn begin(&mut self) {
        self.cursor = 0;
    }
    pub fn resolve(&mut self, decided: DecidedProject) -> NativeResult<()> {
        let output = decided.output;
        let matches = match (&output.input, &output.decision) {
            (Input::Checkpoint { input }, Decision::Checkpoint { decision }) => {
                use research_workbench_native_sdk::writing_checkpoint::{
                    CheckpointDecision, CheckpointInput,
                };
                match (input.as_ref(), decision.as_ref()) {
                    (CheckpointInput::Read { rows, .. }, CheckpointDecision::Read { record }) => {
                        super::super::files::checkpoints::record_matches(rows, record.as_deref())
                    }
                    (CheckpointInput::Read { .. }, CheckpointDecision::Rejected { .. }) => true,
                    _ => false,
                }
            }
            (
                Input::PrepareAgentFileChanges { relative_paths },
                Decision::AgentFileChangesPrepared { policies },
            ) => {
                relative_paths.len() == policies.len()
                    && relative_paths
                        .iter()
                        .zip(policies)
                        .all(|(path, policy)| path == &policy.relative_path)
            }
            (Input::ListFiles { .. }, Decision::Files { .. } | Decision::Rejected { .. }) => true,
            (Input::Context { input }, Decision::Context { decision }) => {
                super::response::matches_context(input, decision)
            }
            _ => false,
        };
        if !matches {
            let mut error = NativeHostError::for_profile(
                NativeProfile::WritingProjects,
                "plugin_host_response_mismatch",
                "写作上下文结果与本次阶段不匹配。上次读取事务已结算。",
            )
            .package(&self.key);
            error.instance_id = Some(decided.instance_id);
            error.request_id = Some(decided.request_id);
            return Err(error);
        }
        self.decisions.push(Cached {
            fingerprint: streaming_payload::write_json(std::io::sink(), &output.input)
                .map_err(|error| {
                    let mut failure = NativeHostError::for_profile(
                        NativeProfile::WritingProjects,
                        "plugin_host_context_fingerprint_failed",
                        error.to_string(),
                    )
                    .package(&self.key);
                    failure.instance_id = Some(decided.instance_id.clone());
                    failure.request_id = Some(decided.request_id.clone());
                    failure
                })?
                .1,
            decision: Arc::new(output.decision),
            origin: DecisionOrigin {
                package_key: self.key.clone(),
                instance_id: decided.instance_id,
                native_request_id: decided.request_id,
            },
        });
        Ok(())
    }
    pub fn rejection(&self, cause: &FailureCause) -> Option<Error> {
        let FailureCause::NativeRejection(origin) = cause else {
            return None;
        };
        self.decisions.iter().find_map(|cached| {
            if cached.origin.package_key != origin.package_key
                || cached.origin.instance_id != origin.instance_id
                || cached.origin.native_request_id != origin.native_request_id
            {
                return None;
            }
            match cached.decision.as_ref() {
                Decision::Context { decision } => match decision.as_ref() {
                    ContextDecision::Rejected { reason } => Some(reason.clone()),
                    _ => None,
                },
                _ => None,
            }
        })
    }
    fn decide(&mut self, input: Input) -> ContextRead<(Arc<Decision>, DecisionOrigin)> {
        let fingerprint = streaming_payload::write_json(std::io::sink(), &input)
            .map_err(|_| Flow::original(Error::Unavailable))?
            .1;
        let index = self.cursor;
        self.cursor += 1;
        match self.decisions.get(index) {
            Some(cached)
                if cached.fingerprint.byte_size == fingerprint.byte_size
                    && cached.fingerprint.sha256 == fingerprint.sha256 =>
            {
                Ok((cached.decision.clone(), cached.origin.clone()))
            }
            _ => {
                self.decisions.truncate(index);
                Err(Flow::Need(Box::new(input)))
            }
        }
    }
    fn context(&mut self, input: ContextInput) -> ContextRead<(ContextDecision, DecisionOrigin)> {
        let (decision, origin) = self.decide(Input::Context {
            input: Box::new(input),
        })?;
        match decision.as_ref() {
            Decision::Context { decision } => match decision.as_ref() {
                ContextDecision::Rejected { reason } => Err(native(reason.clone(), origin)),
                decision => Ok((decision.clone(), origin)),
            },
            _ => Err(native(Error::Unavailable, origin)),
        }
    }
}
fn native(error: Error, origin: DecisionOrigin) -> Flow<Error, ContextNeed> {
    Flow::Fail(Failure {
        error,
        cause: FailureCause::NativeRejection(Box::new(origin)),
    })
}
fn data(error: Box<PluginDataError>) -> Flow<Error, ContextNeed> {
    Flow::Fail(Failure {
        error: Error::Unavailable,
        cause: FailureCause::SourceData(error),
    })
}

impl WritingContextReader for Reader {
    fn resource_scope(&self) -> Option<crate::agent::writing_resources::WritingScope<'_>> {
        self.scope
            .as_ref()
            .map(|source| crate::agent::writing_resources::WritingScope::Native(source))
    }
    fn selected(
        &mut self,
        db: &Connection,
        root: &Path,
        request: WritingContextRequest,
    ) -> ContextRead<Vec<ContextText>> {
        self.context(ContextInput::Validate {
            request: request.clone(),
        })?;
        let manuscript = request.manuscript_id.to_string();
        let rows = self
            .resources
            .files
            .projects
            .get(db, &manuscript)
            .map_err(data)?;
        self.context(ContextInput::Project {
            manuscript_id: request.manuscript_id,
            rows,
        })?;
        // list_files_in originally resolves the manuscript again before its
        // ordered file query. Keep both observations on this same transaction.
        let manuscript_rows = self
            .resources
            .files
            .projects
            .get(db, &manuscript)
            .map_err(data)?;
        let rows = self.resources.files.files(db, &manuscript).map_err(data)?;
        let (decision, origin) = self.decide(Input::ListFiles {
            manuscript_id: manuscript.clone(),
            manuscript: manuscript_rows,
            files: Some(rows),
        })?;
        let Decision::Files { files } = decision.as_ref() else {
            return Err(native(Error::Unavailable, origin));
        };
        let mut context = Vec::new();
        let mut used_bytes = 0;
        for file in request.files {
            self.context(ContextInput::Registered {
                relative_path: file.relative_path.clone(),
                files: files
                    .iter()
                    .filter(|entry| entry.relative_path == file.relative_path)
                    .cloned()
                    .collect(),
            })?;
            let (disk, disk_hash) = crate::scoped_text_files::legacy::read_text_file(
                root,
                &manuscript,
                &file.relative_path,
            )
            .map_err(|_| Flow::original(Error::Unavailable))?;
            let (decision, origin) = self.context(ContextInput::Materialize {
                manuscript_id: request.manuscript_id,
                file,
                disk,
                disk_hash,
                used_bytes,
            })?;
            let ContextDecision::Materialized {
                source,
                used_bytes: bytes,
            } = decision
            else {
                return Err(native(Error::Unavailable, origin));
            };
            used_bytes = bytes;
            context.push(source);
        }
        Ok(context)
    }
    fn current_file(
        &mut self,
        db: &Connection,
        root: &Path,
        manuscript: Uuid,
        path: &str,
    ) -> ContextRead<Vec<ContextText>> {
        let rows = self
            .resources
            .current(db, &manuscript.to_string(), path)
            .map_err(data)?;
        let (decision, origin) = self.context(ContextInput::CurrentFile {
            manuscript_id: manuscript,
            relative_path: path.into(),
            rows,
        })?;
        let ContextDecision::CurrentFile { request } = decision else {
            return Err(native(Error::Unavailable, origin));
        };
        self.selected(db, root, request)
    }
}
