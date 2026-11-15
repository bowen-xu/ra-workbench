//! Fixed original Docling evidence statements for the selected literature package.
use crate::{
    ai::docling_generation_support::DoclingStatements,
    runtime_plugins::{
        RuntimePluginCatalog, RuntimePluginError, RuntimePluginPackage, data::load_text,
    },
};
pub(crate) struct DoclingResources {
    existing: String,
    title: String,
    bundle: String,
    anchor: String,
    item: String,
    context: String,
}
impl DoclingResources {
    pub(crate) fn load(
        catalog: &RuntimePluginCatalog,
        package: &RuntimePluginPackage,
    ) -> Result<Self, RuntimePluginError> {
        Ok(Self {
            existing: load_text(
                catalog,
                package,
                "literature_library",
                "data/analysis-docling-evidence-existing.v1.sql",
                "89b2e7782fe9caee1a14971bf9f20ca5a49faa8af9454c6448d9b4e1d0f84e8c",
            )?,
            title: load_text(
                catalog,
                package,
                "literature_library",
                "data/analysis-docling-evidence-title.v1.sql",
                "d67dc8b4782b047a3e0edaab255eb33351840322c785ca20efdc79fc77fd925f",
            )?,
            bundle: load_text(
                catalog,
                package,
                "literature_library",
                "data/analysis-docling-evidence-bundle.v1.sql",
                "0dd99c310fea07d5a3cb5bde567159051f0c6ffec582adca509378dea6a333ae",
            )?,
            anchor: load_text(
                catalog,
                package,
                "literature_library",
                "data/analysis-docling-evidence-anchor.v1.sql",
                "c1fdcd02c79868105f2dabe27cac207f858f692eb0f3224c826f519e7234f1a3",
            )?,
            item: load_text(
                catalog,
                package,
                "literature_library",
                "data/analysis-docling-evidence-item.v1.sql",
                "aba16982d920e8cabc67273f251f51c2b59608b3b3728af6de9fc1ce2c9533b5",
            )?,
            context: load_text(
                catalog,
                package,
                "literature_library",
                "data/analysis-docling-evidence-context.v1.sql",
                "a5440d7d2d1bacd3b6a84421713b46475bb2b2747df9fe801912c0e29e2bc34a",
            )?,
        })
    }
    pub(crate) fn statements(&self) -> DoclingStatements<'_> {
        DoclingStatements {
            existing: &self.existing,
            title: &self.title,
            bundle: &self.bundle,
            anchor: &self.anchor,
            item: &self.item,
            context: &self.context,
        }
    }
}
