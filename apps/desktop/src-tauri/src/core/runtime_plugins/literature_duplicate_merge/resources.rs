use crate::runtime_plugins::{
    RuntimePluginCatalog, RuntimePluginError, RuntimePluginPackage,
    literature_nal_bootstrap::BootstrapResources,
};

pub(crate) mod inner;
pub(crate) mod outer;
pub(crate) mod rules;

#[derive(Clone)]
pub(crate) struct MergeResources {
    pub(super) outer: outer::OuterMergeResources,
    pub(super) inner: inner::InnerMergeResources,
    pub(crate) rules: rules::MergeRuleResources,
    pub(super) bootstrap: BootstrapResources,
}

impl MergeResources {
    pub(crate) fn package_key(&self) -> &str {
        self.outer.header.package_key()
    }

    pub(crate) fn load(
        catalog: &RuntimePluginCatalog,
        package: &RuntimePluginPackage,
    ) -> Result<Self, RuntimePluginError> {
        Ok(Self {
            outer: outer::OuterMergeResources::load(catalog, package)?,
            inner: inner::InnerMergeResources::load(catalog, package)?,
            rules: rules::MergeRuleResources::load(catalog, package)?,
            bootstrap: BootstrapResources::load(catalog, package)?,
        })
    }
}
