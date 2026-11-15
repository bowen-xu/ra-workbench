//! Fixed catalog reads on the caller's original connection, without a new transaction.
//! Domain query planning, ID selection and item projection remain in the native host.
use crate::{
    data_interface::plugin_resources::{
        ColumnCodec, PluginDataError, RowPolicy, read_on_connection,
    },
    runtime_plugins::{
        RuntimePluginCatalog, RuntimePluginError, RuntimePluginPackage, data::AcceptedDataResource,
    },
};
use research_workbench_native_sdk::{
    catalog::{CatalogItemRows, CatalogQueryPlan, CatalogRelatedRows, CatalogSelectionRows},
    data::{DataAccess, DataCall, DataField, DataResourceGrant, DataRows, DataType, DataValue},
};
use rusqlite::Connection;
mod trash;

type Result<T> = std::result::Result<T, Box<PluginDataError>>;
const OWNER: &str = "literature_library";
const TEXT: &[ColumnCodec] = &[ColumnCodec::String];
const MAIN: &[ColumnCodec] = &[
    ColumnCodec::String,
    ColumnCodec::String,
    ColumnCodec::String,
    ColumnCodec::String,
    ColumnCodec::OptionalI32,
    ColumnCodec::String,
    ColumnCodec::String,
    ColumnCodec::String,
    ColumnCodec::String,
    ColumnCodec::String,
    ColumnCodec::String,
    ColumnCodec::String,
    ColumnCodec::String,
    ColumnCodec::String,
    ColumnCodec::String,
    ColumnCodec::String,
    ColumnCodec::OptionalString,
    ColumnCodec::String,
    ColumnCodec::String,
    ColumnCodec::String,
    ColumnCodec::OptionalString,
];

#[derive(Clone)]
struct ReadResource {
    resource: AcceptedDataResource,
    codecs: &'static [ColumnCodec],
    policy: RowPolicy,
}

impl ReadResource {
    fn call(&self, arguments: Vec<DataValue>) -> DataCall {
        DataCall {
            id: self.resource.grant().id.clone(),
            version: self.resource.grant().version,
            arguments,
        }
    }

    fn read(&self, connection: &Connection, call: &DataCall) -> Result<DataRows> {
        read_on_connection(connection, &self.resource, call, self.policy, self.codecs)
    }
}

/// Nine reviewed, immutable grants from one explicitly accepted package.
#[derive(Clone)]
pub(crate) struct CatalogResources {
    all: ReadResource,
    fts: ReadResource,
    year: ReadResource,
    main: ReadResource,
    creators: ReadResource,
    tags: ReadResource,
    attachments: ReadResource,
    project_ids: ReadResource,
    metadata_sources: ReadResource,
}

impl CatalogResources {
    pub(crate) fn load(
        catalog: &RuntimePluginCatalog,
        package: &RuntimePluginPackage,
    ) -> std::result::Result<Self, RuntimePluginError> {
        let load = |name: &str,
                    hash: &str,
                    parameters: Vec<DataField>,
                    codecs: &'static [ColumnCodec],
                    policy|
         -> std::result::Result<ReadResource, RuntimePluginError> {
            Ok(ReadResource {
                resource: AcceptedDataResource::load(
                    catalog,
                    package,
                    OWNER,
                    &format!("data/catalog-{name}.v1.sql"),
                    DataResourceGrant {
                        id: format!("literature.catalog.{name}"),
                        version: 1,
                        sha256: hash.into(),
                        access: DataAccess::Read,
                        parameters,
                        columns: codecs.iter().map(field).collect(),
                    },
                )?,
                codecs,
                policy,
            })
        };
        let string = || field(&ColumnCodec::String);
        let integer = || DataField {
            data_type: DataType::Integer,
            nullable: false,
        };
        Ok(Self {
            all: load(
                "list-active-ids",
                "3ec08183330a92b659a197c379895122e25e3f9b738e0d530000376e4d836dfd",
                vec![],
                TEXT,
                RowPolicy::All,
            )?,
            fts: load(
                "search-fts-ids",
                "81b5a90c24034e389c758e9e32e4843d9d2d92f423f628bfadb0e91f6722b4f0",
                vec![string()],
                TEXT,
                RowPolicy::All,
            )?,
            year: load(
                "search-year-ids",
                "83b5ce091f263310a5833e0fc9fe38c326b02641eaf0f814184cc78583db7f9e",
                vec![integer()],
                TEXT,
                RowPolicy::All,
            )?,
            main: load(
                "item-main",
                "073a1199d14bcde6a13b66959ae709c47a0670fd8d4e685b764f631033179e3b",
                vec![string(), integer()],
                MAIN,
                RowPolicy::FirstOptional,
            )?,
            creators: load(
                "item-creators",
                "ae99a3aad759b4a7e7bcfde85ac6d635a5c2fb5682b0b990e442a5c5e58dc4cc",
                vec![string()],
                &[ColumnCodec::String; 4],
                RowPolicy::All,
            )?,
            tags: load(
                "item-tags",
                "15f1c68e620d9d92d5480b6215bd3971a19c75dfb24b272fc01e81ae999faae8",
                vec![string()],
                TEXT,
                RowPolicy::All,
            )?,
            attachments: load(
                "item-attachments",
                "4261557a932b7009b3afb2e2ac8f9f4fe42ba46c5647ef0e445cb08ebc8204a0",
                vec![string()],
                &[
                    ColumnCodec::String,
                    ColumnCodec::String,
                    ColumnCodec::String,
                    ColumnCodec::U64,
                    ColumnCodec::String,
                ],
                RowPolicy::All,
            )?,
            project_ids: load(
                "item-project-ids",
                "97526c71337135fec5f15de3b8fe8e7ad9c7dea69185fb1d7dcab072731093ae",
                vec![string()],
                TEXT,
                RowPolicy::All,
            )?,
            metadata_sources: load(
                "item-metadata-sources",
                "04904bb234e0264de630549145007bb9a06b72e2ef43ff473aec3b5c0fabaf21",
                vec![string()],
                &[ColumnCodec::String, ColumnCodec::String, ColumnCodec::Bool],
                RowPolicy::All,
            )?,
        })
    }

    pub(crate) fn package_key(&self) -> &str {
        self.all.resource.package_key()
    }
}

struct SelectorRead {
    resource: ReadResource,
    call: DataCall,
}

/// Owned selector observations only: no connection, transaction or guard is retained.
pub(crate) struct PreparedCatalogSelection {
    primary: SelectorRead,
    year: Option<SelectorRead>,
    rows: CatalogSelectionRows,
}

impl PreparedCatalogSelection {
    pub(crate) fn rows(&self) -> &CatalogSelectionRows {
        &self.rows
    }
}

pub(crate) fn capture_selection(
    connection: &Connection,
    resources: &CatalogResources,
    plan: &CatalogQueryPlan,
) -> Result<PreparedCatalogSelection> {
    let (primary, year) = match plan {
        CatalogQueryPlan::All => (
            SelectorRead {
                resource: resources.all.clone(),
                call: resources.all.call(vec![]),
            },
            None,
        ),
        CatalogQueryPlan::Search { fts_query, year } => (
            SelectorRead {
                resource: resources.fts.clone(),
                call: resources.fts.call(vec![DataValue::Text(fts_query.clone())]),
            },
            year.map(|value| SelectorRead {
                resource: resources.year.clone(),
                call: resources
                    .year
                    .call(vec![DataValue::Integer(i64::from(value))]),
            }),
        ),
        CatalogQueryPlan::Get { .. } => {
            return Err(failure(
                &resources.all.resource,
                "validate",
                "plugin_data_call_invalid",
                "目录选择只接受原列表查询计划。",
                "catalog_selector_plan_required",
            ));
        }
    };
    let rows = read_selection(connection, &primary, year.as_ref())?;
    Ok(PreparedCatalogSelection {
        primary,
        year,
        rows,
    })
}

/// Repeat every original selector in its original order before reading any items.
pub(crate) fn recheck_selection(
    connection: &Connection,
    prepared: &PreparedCatalogSelection,
) -> Result<()> {
    let current = read_selection(connection, &prepared.primary, prepared.year.as_ref())?;
    if current != prepared.rows {
        let resource = if current.primary != prepared.rows.primary {
            &prepared.primary.resource.resource
        } else {
            prepared
                .year
                .as_ref()
                .map_or(&prepared.primary.resource.resource, |year| {
                    &year.resource.resource
                })
        };
        return Err(failure(
            resource,
            "recheck",
            "plugin_data_snapshot_changed",
            "插件选择所依据的目录数据已变化；本次未读取条目。",
            "catalog_selector_rows_changed",
        ));
    }
    Ok(())
}

fn read_selection(
    connection: &Connection,
    primary: &SelectorRead,
    year: Option<&SelectorRead>,
) -> Result<CatalogSelectionRows> {
    let primary = primary.resource.read(connection, &primary.call)?;
    let year = year
        .map(|read| read.resource.read(connection, &read.call))
        .transpose()?;
    Ok(CatalogSelectionRows { primary, year })
}

/// Capture the original per-ID main/related sequence. A missing main row stops
/// subsequent reads; the native domain, not Core, interprets that as NotFound.
pub(crate) fn capture_items(
    connection: &Connection,
    resources: &CatalogResources,
    ids: &[String],
) -> Result<Vec<CatalogItemRows>> {
    capture_items_with_visibility(connection, resources, ids, ItemVisibility::Active)
}

pub(crate) fn capture_trashed_items(
    connection: &Connection,
    resources: &CatalogResources,
    ids: &[String],
) -> Result<Vec<CatalogItemRows>> {
    capture_items_with_visibility(connection, resources, ids, ItemVisibility::IncludeHidden)
}

enum ItemVisibility {
    Active,
    IncludeHidden,
}

fn capture_items_with_visibility(
    connection: &Connection,
    resources: &CatalogResources,
    ids: &[String],
    visibility: ItemVisibility,
) -> Result<Vec<CatalogItemRows>> {
    let mut items = Vec::with_capacity(ids.len());
    for id in ids {
        let main = resources.main.read(
            connection,
            &resources.main.call(vec![
                DataValue::Text(id.clone()),
                DataValue::Integer(i64::from(matches!(
                    visibility,
                    ItemVisibility::IncludeHidden
                ))),
            ]),
        )?;
        if main.rows.is_empty() {
            items.push(CatalogItemRows {
                main,
                related: None,
            });
            break;
        }
        let read = |resource: &ReadResource| {
            resource.read(
                connection,
                &resource.call(vec![DataValue::Text(id.clone())]),
            )
        };
        let creators = read(&resources.creators)?;
        let tags = read(&resources.tags)?;
        let attachments = read(&resources.attachments)?;
        let project_ids = read(&resources.project_ids)?;
        let metadata_sources = read(&resources.metadata_sources)?;
        items.push(CatalogItemRows {
            main,
            related: Some(CatalogRelatedRows {
                creators,
                tags,
                attachments,
                project_ids,
                metadata_sources,
            }),
        });
    }
    Ok(items)
}

fn field(codec: &ColumnCodec) -> DataField {
    let (data_type, nullable) = match codec {
        ColumnCodec::String | ColumnCodec::F64Bits => (DataType::Text, false),
        ColumnCodec::OptionalString | ColumnCodec::OptionalF32Bits => (DataType::Text, true),
        ColumnCodec::I32
        | ColumnCodec::I64
        | ColumnCodec::U64
        | ColumnCodec::Usize
        | ColumnCodec::Bool => (DataType::Integer, false),
        ColumnCodec::OptionalI32 | ColumnCodec::OptionalI64 => (DataType::Integer, true),
    };
    DataField {
        data_type,
        nullable,
    }
}

fn failure(
    resource: &AcceptedDataResource,
    phase: &'static str,
    code: &'static str,
    message: &'static str,
    cause: &str,
) -> Box<PluginDataError> {
    Box::new(PluginDataError {
        code,
        message,
        package_key: resource.package_key().into(),
        resource_id: resource.grant().id.clone(),
        resource_version: resource.grant().version,
        phase,
        safe_cause: cause.into(),
        sqlite_code: None,
        sqlite_extended_code: None,
        original_sql_failure: None,
    })
}

#[cfg(test)]
mod tests;
