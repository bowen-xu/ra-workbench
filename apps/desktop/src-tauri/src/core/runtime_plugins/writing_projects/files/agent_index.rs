//! Fixed original Agent index statements and admitted per-path domain decisions.
use super::{changes::ChangeResources, resources::FileResources};
use crate::{
    agent::{
        writing_context::WritingContextError as Error, writing_workspace::index::IndexFailure,
    },
    ai::structured_resource_validation::DecisionOrigin,
    data_interface::plugin_resources::{
        ColumnCodec, PluginDataError, RowPolicy, read_on_connection,
    },
    runtime_plugins::{
        RuntimePluginCatalog, RuntimePluginError, RuntimePluginPackage, data::AcceptedDataResource,
        writing_projects::data_failure,
    },
};
use research_agent_core::workspace::{FileVersion, MAX_FILE_BYTES};
use research_workbench_native_sdk::{
    data::{DataAccess, DataCall, DataResourceGrant, DataValue},
    writing_file::AgentFilePolicy,
};
use rusqlite::Connection;
use std::sync::Arc;
use uuid::Uuid;

pub(crate) struct Resources {
    files: FileResources,
    changes: ChangeResources,
    exists: AcceptedDataResource,
    matches: AcceptedDataResource,
}
impl Resources {
    pub(crate) fn load(
        catalog: &RuntimePluginCatalog,
        package: &RuntimePluginPackage,
    ) -> Result<Self, RuntimePluginError> {
        let text = ColumnCodec::String.field();
        let read = |name: &str, hash: &str, parameters| {
            AcceptedDataResource::load(
                catalog,
                package,
                "writing_projects",
                &format!("data/project-agent-file-{name}.v1.sql"),
                DataResourceGrant {
                    id: format!("writing.projects.agent-file-{name}"),
                    version: 1,
                    sha256: hash.into(),
                    access: DataAccess::Read,
                    parameters,
                    columns: vec![ColumnCodec::Bool.field()],
                },
            )
        };
        Ok(Self {
            files: FileResources::load(catalog, package)?,
            changes: ChangeResources::load(catalog, package)?,
            exists: read(
                "exists",
                "d50681889efef92f3d9b0b6bae29853e3bc94851acb7ab6fb2b7cfcb68f35c2f",
                vec![text; 2],
            )?,
            matches: read(
                "matches",
                "f57a1ae9049b44e680c2865e4d17e4ee977314c9b30641d85b1551f83d5f260c",
                vec![text, text, text, ColumnCodec::I64.field()],
            )?,
        })
    }
    fn flag(
        db: &Connection,
        resource: &AcceptedDataResource,
        arguments: Vec<DataValue>,
    ) -> Result<bool, Box<PluginDataError>> {
        let rows = read_on_connection(
            db,
            resource,
            &DataCall {
                id: resource.grant().id.clone(),
                version: 1,
                arguments,
            },
            RowPolicy::FirstRequired,
            &[ColumnCodec::Bool],
        )?;
        Ok(rows.rows[0][0] == DataValue::Integer(1))
    }
}

#[derive(Clone)]
pub(crate) struct PreparedIndex {
    pub resources: Arc<Resources>,
    pub policy: AgentFilePolicy,
    pub origin: DecisionOrigin,
    pub operation: Uuid,
}
impl PreparedIndex {
    fn path(&self, path: &str) -> Result<(), IndexFailure> {
        if self.policy.relative_path != path || !self.policy.editable {
            return Err(IndexFailure::Original(Error::InvalidInput));
        }
        Ok(())
    }
    fn data(&self, error: Box<PluginDataError>) -> IndexFailure {
        let mut error = data_failure(error);
        error.operation_id = Some(self.operation.to_string());
        error.instance_id = Some(self.origin.instance_id.clone());
        error.request_id = Some(self.origin.native_request_id.clone());
        IndexFailure::Source(error)
    }
    pub(crate) fn check_unregistered(
        &self,
        db: &Connection,
        manuscript: Uuid,
        path: &str,
    ) -> Result<(), IndexFailure> {
        self.path(path)?;
        let exists = Resources::flag(
            db,
            &self.resources.exists,
            vec![
                DataValue::Text(manuscript.to_string()),
                DataValue::Text(path.into()),
            ],
        )
        .map_err(|cause| self.data(cause))?;
        if exists {
            return Err(IndexFailure::Original(Error::Conflict));
        }
        Ok(())
    }
    pub(crate) fn check_registered(
        &self,
        db: &Connection,
        manuscript: Uuid,
        path: &str,
        before: &FileVersion,
    ) -> Result<(), IndexFailure> {
        self.path(path)?;
        let matches = Resources::flag(
            db,
            &self.resources.matches,
            vec![
                DataValue::Text(manuscript.to_string()),
                DataValue::Text(path.into()),
                DataValue::Text(before.sha256.clone()),
                DataValue::Integer(before.byte_size as i64),
            ],
        )
        .map_err(|cause| self.data(cause))?;
        if !matches {
            return Err(IndexFailure::Original(Error::Conflict));
        }
        Ok(())
    }
    pub(crate) fn register_created(
        &self,
        db: &Connection,
        manuscript: Uuid,
        path: &str,
        actual: &FileVersion,
    ) -> Result<(), IndexFailure> {
        crate::scoped_text_files::legacy::validate_relative_path(path)
            .map_err(|_| IndexFailure::Original(Error::InvalidInput))?;
        self.check_unregistered(db, manuscript, path)?;
        if actual.byte_size > MAX_FILE_BYTES {
            return Err(IndexFailure::Original(Error::Conflict));
        }
        self.resources
            .files
            .index(
                db,
                &manuscript.to_string(),
                path,
                &self.policy.file_kind,
                actual.byte_size as usize,
                &actual.sha256,
            )
            .map_err(|cause| self.data(cause))
    }
    pub(crate) fn update_registered(
        &self,
        db: &Connection,
        manuscript: Uuid,
        path: &str,
        before: &FileVersion,
        actual: Option<&FileVersion>,
    ) -> Result<(), IndexFailure> {
        crate::scoped_text_files::legacy::validate_relative_path(path)
            .map_err(|_| IndexFailure::Original(Error::InvalidInput))?;
        self.check_registered(db, manuscript, path, before)?;
        if let Some(actual) = actual {
            if actual.byte_size > MAX_FILE_BYTES {
                return Err(IndexFailure::Original(Error::Budget));
            }
            self.resources
                .files
                .index(
                    db,
                    &manuscript.to_string(),
                    path,
                    &self.policy.file_kind,
                    actual.byte_size as usize,
                    &actual.sha256,
                )
                .map_err(|cause| self.data(cause))
        } else {
            self.resources
                .changes
                .delete(db, &manuscript.to_string(), path)
                .map_err(|cause| self.data(cause))?;
            self.resources
                .files
                .touch(db, &manuscript.to_string())
                .map_err(|cause| self.data(cause))
        }
    }
}
