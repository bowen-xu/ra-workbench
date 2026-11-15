//! Four reviewed original read statements, bound to one accepted package.
use crate::runtime_plugins::{
    RuntimePluginCatalog, RuntimePluginError, RuntimePluginPackage, data::load_text,
};

pub(crate) struct HistoryResources {
    workflow: String,
    latest: String,
    turns: String,
    attempts: String,
}

pub(crate) struct Statements<'a> {
    pub workflow: &'a str,
    pub latest: &'a str,
    pub turns: &'a str,
    pub attempts: &'a str,
}

impl HistoryResources {
    pub(crate) fn load(
        catalog: &RuntimePluginCatalog,
        package: &RuntimePluginPackage,
    ) -> Result<Self, RuntimePluginError> {
        Ok(Self {
            workflow: load_text(
                catalog,
                package,
                "literature_library",
                "data/analysis-history-workflow.v1.sql",
                "22697a7547ff51686852584ef0c1e4a44766f6059056c4d3629c8ac99599b7da",
            )?,
            latest: load_text(
                catalog,
                package,
                "literature_library",
                "data/analysis-history-latest.v1.sql",
                "26d43786d1905051ee8818a6f1494f6c89d09857691cb9cc76970957aa9be5e4",
            )?,
            turns: load_text(
                catalog,
                package,
                "literature_library",
                "data/analysis-history-turns.v1.sql",
                "2a232f201bfde60e4fad1472a6c187e80770a88af4e7b76ac1f0a2c3ee1642aa",
            )?,
            attempts: load_text(
                catalog,
                package,
                "literature_library",
                "data/analysis-history-attempts.v1.sql",
                "feb1f18c9599db1ba6cea1d03a6dafb40fd2f692a3f122b50e449a4c007e1451",
            )?,
        })
    }

    pub(crate) fn statements(&self) -> Statements<'_> {
        Statements {
            workflow: &self.workflow,
            latest: &self.latest,
            turns: &self.turns,
            attempts: &self.attempts,
        }
    }
}
