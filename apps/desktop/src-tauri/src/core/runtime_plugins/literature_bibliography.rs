//! Reuse the existing catalog capture and three original candidate statements.
use crate::{
    data_interface::plugin_resources::{
        ColumnCodec, PluginDataError, RowPolicy, read_on_connection,
    },
    runtime_plugins::{
        RuntimePluginCatalog, RuntimePluginError, RuntimePluginPackage, data::AcceptedDataResource,
        literature_catalog::data::CatalogResources,
    },
};
use research_workbench_native_sdk::{
    catalog::bibliography::CandidatesQuery,
    data::{DataAccess, DataCall, DataResourceGrant, DataValue},
};
use rusqlite::Connection;
pub(crate) struct Resources {
    pub(crate) catalog: CatalogResources,
    pub(crate) key: String,
    candidates: [AcceptedDataResource; 3],
    source_exists: AcceptedDataResource,
    imported_key: AcceptedDataResource,
    search: AcceptedDataResource,
}
impl Resources {
    pub(crate) fn load(
        catalog: &RuntimePluginCatalog,
        package: &RuntimePluginPackage,
    ) -> Result<Self, RuntimePluginError> {
        let mut candidates = Vec::new();
        for (name, hash) in [
            (
                "doi",
                "875bae30b295c5ca58c21553978d811a9e4852294aa0b07c13b641a21c876e22",
            ),
            (
                "title",
                "179ea2689d6257ce348cd00e2a3334bc7c6934778b94969843ddbce90d0757b0",
            ),
            (
                "title-fallback",
                "256f3e4f37dd7958a1e558177d6b45a03cb7fe1f874e8547470c8e6c3369e906",
            ),
        ] {
            candidates.push(AcceptedDataResource::load(
                catalog,
                package,
                "literature_library",
                &format!("data/catalog-bibliography-match-{name}.v1.sql"),
                DataResourceGrant {
                    id: format!("literature.bibliography.match-{name}"),
                    version: 1,
                    sha256: hash.into(),
                    access: DataAccess::Read,
                    parameters: vec![ColumnCodec::String.field()],
                    columns: vec![ColumnCodec::String.field()],
                },
            )?);
        }
        Ok(Self {
            search: AcceptedDataResource::load(
                catalog,
                package,
                "literature_library",
                "data/catalog-bibliography-citation-search.v1.sql",
                DataResourceGrant {
                    id: "literature.bibliography.citation-search".into(),
                    version: 1,
                    sha256: "a3baee37d9328f75cd5acf3b70c340ac2c0688c035d74ca820a55ed61a93f91f"
                        .into(),
                    access: DataAccess::Read,
                    parameters: vec![
                        ColumnCodec::String.field(),
                        ColumnCodec::Bool.field(),
                        ColumnCodec::String.field(),
                        ColumnCodec::String.field(),
                    ],
                    columns: search_columns().iter().map(|codec| codec.field()).collect(),
                },
            )?,
            imported_key: AcceptedDataResource::load(
                catalog,
                package,
                "literature_library",
                "data/catalog-bibliography-imported-key.v1.sql",
                DataResourceGrant {
                    id: "literature.bibliography.imported-key".into(),
                    version: 1,
                    sha256: "e110cd74f694c715dc420b7f5665b27aa12c5f3c449c6dd24841b1881fe1e562"
                        .into(),
                    access: DataAccess::Read,
                    parameters: vec![ColumnCodec::String.field()],
                    columns: vec![ColumnCodec::OptionalString.field()],
                },
            )?,
            source_exists: AcceptedDataResource::load(
                catalog,
                package,
                "literature_library",
                "data/catalog-bibliography-source-exists.v1.sql",
                DataResourceGrant {
                    id: "literature.bibliography.source-exists".into(),
                    version: 1,
                    sha256: "49a5e125b581b025c4dba15e2470846979bd54b3d5801f7dfecfa41c8d8f83f1"
                        .into(),
                    access: DataAccess::Read,
                    parameters: vec![ColumnCodec::String.field()],
                    columns: vec![ColumnCodec::Bool.field()],
                },
            )?,
            catalog: CatalogResources::load(catalog, package)?,
            key: package.package_key.clone(),
            candidates: candidates
                .try_into()
                .unwrap_or_else(|_| unreachable!("three fixed source queries")),
        })
    }
    pub(crate) fn candidates(
        &self,
        db: &Connection,
        query: CandidatesQuery,
        parameter: &str,
    ) -> Result<Vec<String>, Box<PluginDataError>> {
        let resource = &self.candidates[match query {
            CandidatesQuery::Doi => 0,
            CandidatesQuery::Title => 1,
            CandidatesQuery::TitleFallback => 2,
        }];
        let rows = read_on_connection(
            db,
            resource,
            &DataCall {
                id: resource.grant().id.clone(),
                version: 1,
                arguments: vec![DataValue::Text(parameter.into())],
            },
            RowPolicy::All,
            &[ColumnCodec::String],
        )?;
        Ok(rows
            .rows
            .into_iter()
            .map(|row| {
                let [DataValue::Text(id)] = row.as_slice() else {
                    unreachable!("read adapter has validated the fixed string codec");
                };
                id.clone()
            })
            .collect())
    }
    pub(crate) fn source_exists(
        &self,
        db: &Connection,
        id: &str,
    ) -> Result<bool, Box<PluginDataError>> {
        let resource = &self.source_exists;
        let rows = read_on_connection(
            db,
            resource,
            &DataCall {
                id: resource.grant().id.clone(),
                version: 1,
                arguments: vec![DataValue::Text(id.into())],
            },
            RowPolicy::FirstRequired,
            &[ColumnCodec::Bool],
        )?;
        let [row] = rows.rows.as_slice() else {
            unreachable!("required exists row");
        };
        let [DataValue::Integer(value)] = row.as_slice() else {
            unreachable!("original bool codec");
        };
        Ok(*value != 0)
    }
    pub(crate) fn imported_key(
        &self,
        db: &Connection,
        id: &str,
    ) -> Result<Option<String>, Box<PluginDataError>> {
        let resource = &self.imported_key;
        let rows = read_on_connection(
            db,
            resource,
            &DataCall {
                id: resource.grant().id.clone(),
                version: 1,
                arguments: vec![DataValue::Text(id.into())],
            },
            RowPolicy::FirstOptional,
            &[ColumnCodec::OptionalString],
        )?;
        Ok(rows
            .rows
            .into_iter()
            .next()
            .and_then(|row| match row.into_iter().next() {
                Some(DataValue::Text(value)) => Some(value),
                _ => None,
            }))
    }
    pub(crate) fn search(
        &self,
        db: &Connection,
        keys: &str,
        project_only: bool,
        project: &str,
        needle: &str,
    ) -> Result<research_workbench_native_sdk::data::DataRows, Box<PluginDataError>> {
        let resource = &self.search;
        read_on_connection(
            db,
            resource,
            &DataCall {
                id: resource.grant().id.clone(),
                version: 1,
                arguments: vec![
                    DataValue::Text(keys.into()),
                    DataValue::Integer(i64::from(project_only)),
                    DataValue::Text(project.into()),
                    DataValue::Text(needle.into()),
                ],
            },
            RowPolicy::All,
            &search_columns(),
        )
    }
}

fn search_columns() -> [ColumnCodec; 9] {
    use ColumnCodec::*;
    [
        String,
        String,
        OptionalI32,
        String,
        String,
        String,
        Bool,
        Bool,
        OptionalString,
    ]
}
