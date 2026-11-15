//! Fixed original running-attempt and terminal failure resources for one package.
use super::failure_store::FailureStatements;
use crate::runtime_plugins::{
    RuntimePluginCatalog, RuntimePluginError, RuntimePluginPackage, data::load_text,
};
pub(crate) struct WorkflowResources {
    running: String,
    attempt: String,
    turn: String,
    workflow: String,
}
impl WorkflowResources {
    pub(crate) fn load(
        catalog: &RuntimePluginCatalog,
        package: &RuntimePluginPackage,
    ) -> Result<Self, RuntimePluginError> {
        Ok(Self {
            running: load_text(
                catalog,
                package,
                "literature_library",
                "data/analysis-workflow-running.v1.sql",
                "4d3677bfd565ad387ef539b7255f971bc0f206db79ed99309c0d007d25492dd6",
            )?,
            attempt: load_text(
                catalog,
                package,
                "literature_library",
                "data/analysis-workflow-fail-attempt.v1.sql",
                "fb6993485367aa59582ec0bc5eb8924009b6b13e8ad4c984aa151fa56f8b4f2f",
            )?,
            turn: load_text(
                catalog,
                package,
                "literature_library",
                "data/analysis-workflow-fail-turn.v1.sql",
                "a0f8876c9879b0d8d6e4e06f1c90172c533e52084d0f1088e51d51c2a1d14302",
            )?,
            workflow: load_text(
                catalog,
                package,
                "literature_library",
                "data/analysis-workflow-fail-workflow.v1.sql",
                "1c91ed168d4d8e80d1d50d9c464cefa208b765cfad74be10bd80e9ce5744e440",
            )?,
        })
    }
    pub(crate) fn failure_statements(&self) -> FailureStatements<'_> {
        FailureStatements {
            running: &self.running,
            attempt: &self.attempt,
            turn: &self.turn,
            workflow: &self.workflow,
        }
    }
}
