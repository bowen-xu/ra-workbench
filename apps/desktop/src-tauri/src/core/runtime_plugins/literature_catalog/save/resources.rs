use crate::{
    data_interface::plugin_resources::{
        ColumnCodec, PluginDataError, RowPolicy, read_on_connection, write_in,
    },
    runtime_plugins::{
        RuntimePluginCatalog, RuntimePluginError, RuntimePluginPackage, data::AcceptedDataResource,
    },
};
use research_workbench_native_sdk::data::{
    DataAccess, DataCall, DataField, DataResourceGrant, DataType, DataValue,
};
use rusqlite::Transaction;

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
const OPTIONAL_INTEGER: DataField = DataField {
    data_type: DataType::Integer,
    nullable: true,
};
const REAL: DataField = DataField {
    data_type: DataType::Real,
    nullable: false,
};

type Result<T> = std::result::Result<T, Box<PluginDataError>>;

#[derive(Clone)]
pub(super) struct Resource {
    accepted: AcceptedDataResource,
}

impl Resource {
    pub(super) fn parameter_failure(&self, safe_cause: &'static str) -> Box<PluginDataError> {
        Box::new(PluginDataError {
            code: "plugin_data_arguments_incompatible",
            message: "插件数据参数不符合固定类型。",
            package_key: self.accepted.package_key().into(),
            resource_id: self.accepted.grant().id.clone(),
            resource_version: self.accepted.grant().version,
            phase: "parameters",
            safe_cause: safe_cause.into(),
            sqlite_code: None,
            sqlite_extended_code: None,
            original_sql_failure: None,
        })
    }
    fn call(&self, arguments: Vec<DataValue>) -> DataCall {
        DataCall {
            id: self.accepted.grant().id.clone(),
            version: self.accepted.grant().version,
            arguments,
        }
    }
    pub(super) fn write(&self, tx: &Transaction<'_>, arguments: Vec<DataValue>) -> Result<()> {
        write_in(tx, &self.accepted, &self.call(arguments)).map(|_| ())
    }
    pub(super) fn ids(
        &self,
        tx: &Transaction<'_>,
        arguments: Vec<DataValue>,
    ) -> Result<Vec<String>> {
        let rows = read_on_connection(
            tx,
            &self.accepted,
            &self.call(arguments),
            RowPolicy::All,
            &[ColumnCodec::String],
        )?;
        rows.rows
            .into_iter()
            .map(|row| match row.into_iter().next() {
                Some(DataValue::Text(value)) => Ok(value),
                _ => Err(Box::new(PluginDataError {
                    code: "plugin_data_result_incompatible",
                    message: "插件数据结果不符合固定列类型。",
                    package_key: self.accepted.package_key().into(),
                    resource_id: self.accepted.grant().id.clone(),
                    resource_version: self.accepted.grant().version,
                    phase: "read",
                    safe_cause: "catalog_save_candidate_shape".into(),
                    sqlite_code: None,
                    sqlite_extended_code: None,
                    original_sql_failure: None,
                })),
            })
            .collect()
    }
}

/// Fixed package hashes and data shapes, not a runtime SQL or step interface.
#[derive(Clone)]
pub(crate) struct SaveResources {
    pub(super) item_upsert: Resource,
    pub(super) creators_delete: Resource,
    pub(super) creator_insert: Resource,
    pub(super) item_creator_insert: Resource,
    pub(super) doi_delete: Resource,
    pub(super) doi_insert: Resource,
    pub(super) tags_delete: Resource,
    pub(super) tag_insert: Resource,
    pub(super) item_tag_insert: Resource,
    pub(super) provenance_upsert: Resource,
    pub(super) search_delete: Resource,
    pub(super) search_insert: Resource,
    pub(super) duplicate_doi_ids: Resource,
    pub(super) duplicate_title_ids: Resource,
    pub(super) duplicate_insert: Resource,
}

impl SaveResources {
    pub(crate) fn load(
        catalog: &RuntimePluginCatalog,
        package: &RuntimePluginPackage,
    ) -> std::result::Result<Self, RuntimePluginError> {
        let load = |name: &str,
                    hash: &str,
                    access: DataAccess,
                    parameters: Vec<DataField>,
                    columns: Vec<DataField>| {
            AcceptedDataResource::load(
                catalog,
                package,
                "literature_library",
                &format!("data/catalog-save-{name}.v1.sql"),
                DataResourceGrant {
                    id: format!("literature.catalog-save.{name}"),
                    version: 1,
                    sha256: hash.into(),
                    access,
                    parameters,
                    columns,
                },
            )
            .map(|accepted| Resource { accepted })
        };
        Ok(Self {
            item_upsert: load(
                "item-upsert",
                "6f7ddffe61a35dbc2e66c8da2b50de4e79727a12fed70fd495c6073c163ed9f1",
                DataAccess::Write,
                vec![
                    TEXT,
                    TEXT,
                    TEXT,
                    TEXT,
                    OPTIONAL_INTEGER,
                    TEXT,
                    TEXT,
                    TEXT,
                    TEXT,
                    TEXT,
                    TEXT,
                    TEXT,
                    TEXT,
                    TEXT,
                    TEXT,
                    OPTIONAL_TEXT,
                ],
                vec![],
            )?,
            creators_delete: load(
                "creators-delete",
                "13c62ed27d970e8e06ea6aace3c68ad17f888158c607f153c16301a76b4715df",
                DataAccess::Write,
                vec![TEXT],
                vec![],
            )?,
            creator_insert: load(
                "creator-insert",
                "07bfe5f3af2af4c91c1b83ce58d4ebdc37f4997c7511727a854f17f983665c82",
                DataAccess::Write,
                vec![TEXT, TEXT, TEXT, TEXT],
                vec![],
            )?,
            item_creator_insert: load(
                "item-creator-insert",
                "57faab3c1fe50d94067873eaf2f56911276ec2ebc2f50abada657997a6d27486",
                DataAccess::Write,
                vec![TEXT, TEXT, TEXT, INTEGER],
                vec![],
            )?,
            doi_delete: load(
                "doi-delete",
                "e54a1504c616dfe6a4579f9483e8bdf565af107be70b0e565ee32e66457fb1ea",
                DataAccess::Write,
                vec![TEXT],
                vec![],
            )?,
            doi_insert: load(
                "doi-insert",
                "1d4ee8d0fceff7d624015d0cec545275e71c41e9753649e27c9621a1b6fbfd5b",
                DataAccess::Write,
                vec![TEXT, TEXT, TEXT, TEXT],
                vec![],
            )?,
            tags_delete: load(
                "tags-delete",
                "2f5a3b6dcfa53d3fb0975b05c4cfbf18a910be8e626748cf3b9815a858891b6d",
                DataAccess::Write,
                vec![TEXT],
                vec![],
            )?,
            tag_insert: load(
                "tag-insert",
                "c213b388dda8117f5447ecf50a659cc800282efb6f10484c39b0ea892666e813",
                DataAccess::Write,
                vec![TEXT, TEXT],
                vec![],
            )?,
            item_tag_insert: load(
                "item-tag-insert",
                "813c9e10e962e1dd198898f6bfeda78cdac1b182b6522d3ba43cf661044ea751",
                DataAccess::Write,
                vec![TEXT, TEXT],
                vec![],
            )?,
            provenance_upsert: load(
                "provenance-upsert",
                "5ca945be6d3f1dc220c0e5097af1d5e7883436be5ac25907015ce845d533e9bc",
                DataAccess::Write,
                vec![TEXT, TEXT, TEXT, INTEGER],
                vec![],
            )?,
            search_delete: load(
                "search-delete",
                "715c7a5add6de4717ac9e65b14e27cfabe82ede482fda066a46ae900837ea9fe",
                DataAccess::Write,
                vec![TEXT],
                vec![],
            )?,
            search_insert: load(
                "search-insert",
                "2b090db8c7abc5eaa72bfafdaf1ca5320a2347d6fc3adb1ed77e2b81824cc85f",
                DataAccess::Write,
                vec![TEXT],
                vec![],
            )?,
            duplicate_doi_ids: load(
                "duplicate-doi-ids",
                "b82b95a3375089d1fd34387874d884796bd54d150d8342972354a22a6ae36c48",
                DataAccess::Read,
                vec![TEXT],
                vec![TEXT],
            )?,
            duplicate_title_ids: load(
                "duplicate-title-ids",
                "47c4901a1491a88ae5de26fcff197a177165d824eaa3f3973d47ee9ffa7241d6",
                DataAccess::Read,
                vec![TEXT, TEXT, OPTIONAL_INTEGER],
                vec![TEXT],
            )?,
            duplicate_insert: load(
                "duplicate-insert",
                "ebd0065d6ed08984a1bd96c02a4b8e4e20ef46a7a0416f6000a0da29ae1e167b",
                DataAccess::Write,
                vec![TEXT, TEXT, TEXT, TEXT, REAL],
                vec![],
            )?,
        })
    }
    pub(crate) fn package_key(&self) -> &str {
        self.item_upsert.accepted.package_key()
    }
}
