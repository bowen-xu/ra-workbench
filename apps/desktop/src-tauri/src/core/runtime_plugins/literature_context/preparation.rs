//! Original legacy-session reads with fixed column conversion order and witnesses.
use super::*;
use crate::data_interface::plugin_resources::read_on_connection_in_order;
use research_workbench_native_sdk::{
    data::{DataAccess, DataField, DataResourceGrant, DataType},
    paper_context_prepare::{PaperPreparationInputV1 as Input, ReadingSessionRowV1},
};
#[derive(Clone)]
pub(crate) struct PreparationResources {
    pub context: PaperContextResources,
    resources: Vec<AcceptedDataResource>,
}
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SessionCapture {
    pub input: Option<Input>,
    latest: Vec<Vec<V>>,
    summary: Vec<Vec<V>>,
}
const SUMMARY_CODECS: &[C] = &[
    C::String,
    C::String,
    C::OptionalString,
    C::String,
    C::String,
    C::String,
    C::String,
    C::OptionalString,
    C::String,
    C::OptionalString,
    C::OptionalI64,
    C::OptionalString,
    C::Bool,
    C::String,
    C::String,
    C::String,
    C::String,
];
const COLUMN_ORDER: &[usize] = &[3, 4, 0, 1, 2, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];
const SPECS: [(&str, &str, &[C]); 2] = [
    (
        "session-latest",
        "5151ea26e786d8dd99a815aa4f972cc1bd141fe0a354894098e022e20ca30363",
        &[C::String],
    ),
    (
        "session-summary",
        "4ae724e71c272a08a6498fbe75a5e25ff6a02c500d7008107e57f31303518025",
        SUMMARY_CODECS,
    ),
];
impl PreparationResources {
    pub(crate) fn load_context(
        catalog: &RuntimePluginCatalog,
        package: &RuntimePluginPackage,
    ) -> Result<Self, RuntimePluginError> {
        Ok(Self {
            context: PaperContextResources::load(catalog, package)?,
            resources: Vec::new(),
        })
    }
    pub(crate) fn load(
        catalog: &RuntimePluginCatalog,
        package: &RuntimePluginPackage,
    ) -> Result<Self, RuntimePluginError> {
        let resources = SPECS
            .iter()
            .map(|(name, sha, codecs)| {
                AcceptedDataResource::load(
                    catalog,
                    package,
                    "literature_library",
                    &format!("data/paper-context-{name}.v1.sql"),
                    DataResourceGrant {
                        id: format!("literature.paper-context.{name}"),
                        version: 1,
                        sha256: (*sha).into(),
                        access: DataAccess::Read,
                        parameters: vec![DataField {
                            data_type: DataType::Text,
                            nullable: false,
                        }],
                        columns: codecs.iter().map(|codec| codec.field()).collect(),
                    },
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            context: PaperContextResources::load(catalog, package)?,
            resources,
        })
    }
    fn shape(&self, index: usize) -> ContextFailure {
        failure::data(Box::new(PluginDataError {
            code: "plugin_data_result_incompatible",
            message: "旧阅读会话数据与固定契约不符。",
            package_key: self.context.package_key().into(),
            resource_id: self.resources[index].grant().id.clone(),
            resource_version: 1,
            phase: "read",
            safe_cause: "reading_session_data_shape".into(),
            sqlite_code: None,
            sqlite_extended_code: None,
            original_sql_failure: None,
        }))
    }
    fn read(&self, db: &Connection, index: usize, argument: &str) -> ContextResult<Vec<Vec<V>>> {
        let resource = &self.resources[index];
        let call = DataCall {
            id: resource.grant().id.clone(),
            version: 1,
            arguments: vec![text(argument)],
        };
        let rows = if index == 1 {
            read_on_connection_in_order(
                db,
                resource,
                &call,
                P::FirstOptional,
                SPECS[index].2,
                COLUMN_ORDER,
            )
        } else {
            read_on_connection(db, resource, &call, P::FirstOptional, SPECS[index].2)
        };
        rows.map(|rows| rows.rows).map_err(failure::data)
    }
    pub(crate) fn capture_session(
        &self,
        db: &Connection,
        paper: &str,
    ) -> ContextResult<SessionCapture> {
        let latest = self.read(db, 0, paper)?;
        let id = match latest.as_slice() {
            [] => {
                return Ok(SessionCapture {
                    input: None,
                    latest,
                    summary: vec![],
                });
            }
            [row] => match row.as_slice() {
                [V::Text(id)] => id,
                _ => return Err(self.shape(0)),
            },
            _ => return Err(self.shape(0)),
        };
        let summary = self.read(db, 1, id)?;
        let row = match summary.as_slice() {
            [] => None,
            [row] => Some(self.decode_session(row)?),
            _ => return Err(self.shape(1)),
        };
        Ok(SessionCapture {
            input: Some(Input::Session {
                row: row.map(Box::new),
            }),
            latest,
            summary,
        })
    }
    pub(crate) fn capture_session_summary(
        &self,
        db: &Connection,
        id: &str,
    ) -> ContextResult<Option<ReadingSessionRowV1>> {
        match self.read(db, 1, id)?.as_slice() {
            [] => Ok(None),
            [row] => self.decode_session(row).map(Some),
            _ => Err(self.shape(1)),
        }
    }
    fn decode_session(&self, row: &[V]) -> ContextResult<ReadingSessionRowV1> {
        let [
            V::Text(id),
            V::Text(paper),
            snapshot,
            V::Text(attachments),
            V::Text(parses),
            V::Text(provider),
            V::Text(model),
            reasoning,
            V::Text(bootstrap),
            workflow,
            revision,
            hash,
            V::Integer(retrieval),
            V::Text(title),
            V::Text(status),
            V::Text(created),
            V::Text(updated),
        ] = row
        else {
            return Err(self.shape(1));
        };
        let optional = |value: &V| match value {
            V::Text(value) => Some(value.clone()),
            _ => None,
        };
        Ok(ReadingSessionRowV1 {
            id: id.clone(),
            paper_id: paper.clone(),
            context_snapshot_id: optional(snapshot),
            attachment_ids_json: attachments.clone(),
            parse_run_ids_json: parses.clone(),
            provider_id: provider.clone(),
            model_id: model.clone(),
            reasoning_effort: optional(reasoning),
            bootstrap_kind: bootstrap.clone(),
            source_workflow_id: optional(workflow),
            source_workflow_revision: match revision {
                V::Integer(value) => Some(*value),
                _ => None,
            },
            bootstrap_hash: optional(hash),
            concept_retrieval_default: *retrieval != 0,
            title: title.clone(),
            status: status.clone(),
            created_at: created.clone(),
            updated_at: updated.clone(),
        })
    }
}

/// Original preparation settlement and saved screenshot progress for internal callers.
#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PreparationDiagnostic {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure: Option<crate::runtime_plugins::literature_hbs::NativeFailure>,
    pub image_object_write_started: bool,
    pub saved_image_asset_count: usize,
}
