//! Exact original attempt resources, selected from the same literature package.
use super::attempt_store::{CompleteAttemptStatements, StartAttemptStatements};
use crate::runtime_plugins::{
    RuntimePluginCatalog, RuntimePluginError, RuntimePluginPackage, data::load_text,
};

pub(crate) struct AttemptResources {
    start_attempt: String,
    start_turn: String,
    start_workflow: String,
    complete_attempt: String,
    complete_turn: String,
    complete_workflow: String,
    pub concept: String,
}
impl AttemptResources {
    pub(crate) fn load(
        catalog: &RuntimePluginCatalog,
        package: &RuntimePluginPackage,
    ) -> Result<Self, RuntimePluginError> {
        let load = |name: &str, sha: &str| {
            load_text(
                catalog,
                package,
                "literature_library",
                &format!("data/analysis-attempt-{name}.v1.sql"),
                sha,
            )
        };
        Ok(Self {
            start_attempt: load(
                "start-attempt",
                "2cd0144101683418b3e6c5e53dd3d8c2cb4f2631e67ff78641809d69770a1c47",
            )?,
            start_turn: load(
                "start-turn",
                "b77f476573465c4723ba1d70e3b51608907c174f11aa16e0133cb28d85262c16",
            )?,
            start_workflow: load(
                "start-workflow",
                "8dd60d8d535de64834faddc502212ca245fa530c8c2391b19dd3649496b59681",
            )?,
            complete_attempt: load(
                "complete-attempt",
                "c0ed017065572a9a530683b7f28099212f126b6c0a076b334875c307f9dab282",
            )?,
            complete_turn: load(
                "complete-turn",
                "6eab360768b0523131c299124c53d7eb8b18cff58a533aeb94db60ed2962520e",
            )?,
            complete_workflow: load(
                "complete-workflow",
                "005fdf185413cb94781d9d9ea0be04da24971f0c2ea3b5d2dfce469ce1856608",
            )?,
            concept: load(
                "complete-concept",
                "cf5de35d0faeb710873ef0dc1cb5e67bc21bf58aa9647b1ffb305c72f99ea5e0",
            )?,
        })
    }
    pub(crate) fn start(&self) -> StartAttemptStatements<'_> {
        StartAttemptStatements {
            attempt: &self.start_attempt,
            turn: &self.start_turn,
            workflow: &self.start_workflow,
        }
    }
    pub(crate) fn complete(&self) -> CompleteAttemptStatements<'_> {
        CompleteAttemptStatements {
            attempt: &self.complete_attempt,
            turn: &self.complete_turn,
            workflow: &self.complete_workflow,
        }
    }
}
