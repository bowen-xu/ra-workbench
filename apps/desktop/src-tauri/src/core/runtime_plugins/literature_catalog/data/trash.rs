//! Original trash selectors; main/related reads keep the existing catalog ABI.
use super::*;

impl CatalogResources {
    pub(crate) fn load_trash(
        catalog: &RuntimePluginCatalog,
        package: &RuntimePluginPackage,
    ) -> std::result::Result<Self, RuntimePluginError> {
        let mut resources = Self::load(catalog, package)?;
        let load = |name: &str, hash: &str, parameters| {
            Ok::<_, RuntimePluginError>(ReadResource {
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
                        columns: vec![field(&ColumnCodec::String)],
                    },
                )?,
                codecs: TEXT,
                policy: RowPolicy::All,
            })
        };
        resources.all = load(
            "list-trash-ids",
            "8652c340e4f3e0225715ac1fe4c0e6fe56f350dff73a9ba37519b1c197ff5c03",
            vec![],
        )?;
        resources.fts = load(
            "search-trash-fts-ids",
            "3179438c63c096a9f350d8a0406beb47b0b4583908262b90f584c95457e364b4",
            vec![field(&ColumnCodec::String)],
        )?;
        resources.year = load(
            "search-trash-year-ids",
            "b6c170fd83e41d52603f785002610bb8c791b6afbc7497304452ab0233f75822",
            vec![field(&ColumnCodec::I64)],
        )?;
        Ok(resources)
    }
}
