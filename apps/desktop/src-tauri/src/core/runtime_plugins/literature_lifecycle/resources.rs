//! Fixed lifecycle grants; execution keeps the caller's original connection.
use crate::{
    data_interface::plugin_resources::{
        ColumnCodec, PluginDataError, RowPolicy, read_on_connection, write_on_connection,
    },
    runtime_plugins::{
        RuntimePluginCatalog, RuntimePluginError, RuntimePluginPackage, data::AcceptedDataResource,
    },
};
use research_workbench_native_sdk::data::{
    DataAccess, DataCall, DataField, DataResourceGrant, DataType, DataValue,
};
use rusqlite::Connection;

type Result<T> = std::result::Result<T, Box<PluginDataError>>;
const TEXT: DataField = DataField {
    data_type: DataType::Text,
    nullable: false,
};
const OPTIONAL_TEXT: DataField = DataField {
    data_type: DataType::Text,
    nullable: true,
};
const INTEGER: DataField = DataField {
    data_type: DataType::Integer,
    nullable: false,
};

#[derive(Clone)]
pub(crate) struct LifecycleResources {
    pub(super) archive: AcceptedDataResource,
    pub(super) restore: AcceptedDataResource,
    pub(super) trash: AcceptedDataResource,
    pub(super) restore_trash: AcceptedDataResource,
    pub(super) project_delete: AcceptedDataResource,
    pub(super) project_insert: AcceptedDataResource,
    pub(super) result_policy: AcceptedDataResource,
}

impl LifecycleResources {
    pub(crate) fn load(
        catalog: &RuntimePluginCatalog,
        package: &RuntimePluginPackage,
    ) -> std::result::Result<Self, RuntimePluginError> {
        let load = |suffix: &str, hash: &str, access, parameters, columns| {
            AcceptedDataResource::load(
                catalog,
                package,
                "literature_library",
                &format!("data/literature-lifecycle-{suffix}.v1.sql"),
                DataResourceGrant {
                    id: format!("literature.lifecycle.{suffix}"),
                    version: 1,
                    sha256: hash.into(),
                    access,
                    parameters,
                    columns,
                },
            )
        };
        Ok(Self {
            archive: load(
                "archive",
                "40f558f5b4fb07b5497f97439d75425f2a2fbbf08afd198409f4c7ac05014ec7",
                DataAccess::Write,
                vec![TEXT],
                vec![],
            )?,
            restore: load(
                "restore",
                "e7c249b4d9b7fcaa4aa5ec9c6ff43f956ccadf2f6305051f66b488af59026a58",
                DataAccess::Write,
                vec![TEXT],
                vec![],
            )?,
            trash: load(
                "trash",
                "14c71ff9028ca5b0bcf8e7566688c2a4b1b34f5ed1b20ba43413cfdaf4337329",
                DataAccess::Write,
                vec![TEXT],
                vec![],
            )?,
            restore_trash: load(
                "restore-trash",
                "683036a783f7ff8d9f8d62a865215ea08f786fa73d06ae201ee5df5fb0a61184",
                DataAccess::Write,
                vec![TEXT],
                vec![],
            )?,
            project_delete: load(
                "project-delete",
                "e37b667a1d9b05842794fceb36240d32b108309a9032684143d616a191e04e97",
                DataAccess::Write,
                vec![TEXT],
                vec![],
            )?,
            project_insert: load(
                "project-insert",
                "a634c3359f19a3e74fba9d780e89a4c6a6382e8c559cb974de0efdaa875dd882",
                DataAccess::Write,
                vec![TEXT; 2],
                vec![],
            )?,
            result_policy: load(
                "affected-policy",
                "a358b77dafd61e71bcf7b98f24b7a32f9f65952216c6eac02f70bdd42945cf52",
                DataAccess::Read,
                vec![INTEGER],
                vec![OPTIONAL_TEXT],
            )?,
        })
    }

    pub(crate) fn package_key(&self) -> &str {
        self.archive.package_key()
    }

    /// Autocommit and borrowed transactions share the original fixed statement.
    /// A SQL failure or zero count makes no claim about trigger side effects.
    pub(super) fn write(
        &self,
        db: &Connection,
        resource: &AcceptedDataResource,
        arguments: &[DataValue],
    ) -> Result<u64> {
        write_on_connection(db, resource, &call(resource, arguments.to_vec()))
    }

    pub(super) fn missing(&self, db: &Connection, affected: u64) -> Result<Option<String>> {
        let affected = i64::try_from(affected)
            .map_err(|_| self.policy_error("parameters", "lifecycle_affected_rows_out_of_range"))?;
        let rows = read_on_connection(
            db,
            &self.result_policy,
            &call(&self.result_policy, vec![DataValue::Integer(affected)]),
            RowPolicy::FirstRequired,
            &[ColumnCodec::OptionalString],
        )?;
        match rows.rows.into_iter().next().map(Vec::into_iter) {
            Some(mut row) => match (row.next(), row.next()) {
                (Some(DataValue::Null), None) => Ok(None),
                (Some(DataValue::Text(code)), None) => Ok(Some(code)),
                _ => Err(self.policy_error("decode", "lifecycle_affected_policy_shape")),
            },
            None => Err(self.policy_error("decode", "lifecycle_affected_policy_shape")),
        }
    }

    fn policy_error(&self, phase: &'static str, safe_cause: &'static str) -> Box<PluginDataError> {
        Box::new(PluginDataError {
            code: "plugin_data_result_incompatible",
            message: "文献生命周期数据与固定契约不符。",
            package_key: self.result_policy.package_key().into(),
            resource_id: self.result_policy.grant().id.clone(),
            resource_version: self.result_policy.grant().version,
            phase,
            safe_cause: safe_cause.into(),
            sqlite_code: None,
            sqlite_extended_code: None,
            original_sql_failure: None,
        })
    }
}

fn call(resource: &AcceptedDataResource, arguments: Vec<DataValue>) -> DataCall {
    DataCall {
        id: resource.grant().id.clone(),
        version: resource.grant().version,
        arguments,
    }
}
