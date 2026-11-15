//! Verified original candidate and interrupted-workflow statements from one package.
use super::super::{
    RuntimePluginCatalog, RuntimePluginError, RuntimePluginPackage, data::load_text,
};
use crate::ai::candidate_recovery::RecoveryStatements;
pub(crate) struct RecoveryResources {
    candidates: String,
    targets: String,
    run_stage: String,
    run: String,
    attempt: String,
    turn: String,
    concept: String,
    workflow: String,
    audit: String,
    interrupt: String,
}
impl RecoveryResources {
    pub(crate) fn load(
        catalog: &RuntimePluginCatalog,
        package: &RuntimePluginPackage,
    ) -> Result<Self, RuntimePluginError> {
        let load = |name: &str, hash: &str| {
            load_text(
                catalog,
                package,
                "literature_library",
                &format!("data/{name}"),
                hash,
            )
        };
        Ok(Self {
            candidates: load(
                "analysis-recovery-candidates.v1.sql",
                "3fd2374c30e70aa7992b9ac6e4dbc5dc0e5f8874b783ed1eb4cdd5866f12463b",
            )?,
            targets: load(
                "analysis-recovery-targets.v1.sql",
                "4cc1bc45c270510d09de8e0000884f190a85bd77a1a0c259243c8f2b3498e072",
            )?,
            run_stage: load(
                "analysis-recovery-run-stage.v1.sql",
                "c495116ed4419e4e3a2c2a184ff53cfc6c95271895d9ce3848d24ceb9c95d18d",
            )?,
            run: load(
                "analysis-recovery-run.v1.sql",
                "ea3c8f1049ef73a3938f83115ad64efcc20f17241ac2447845b4a0630644ca5d",
            )?,
            attempt: load(
                "analysis-recovery-attempt.v1.sql",
                "17c661c83a4766fc98d312804263a90298ea1f3aacfe96bb9640f2f292bdf83a",
            )?,
            turn: load(
                "analysis-recovery-turn.v1.sql",
                "5e722f48868df5f1f1c78c04593d3b02d37475ca71b49a48e7efe6957ebd66d9",
            )?,
            concept: load(
                "analysis-recovery-concept.v1.sql",
                "cf5de35d0faeb710873ef0dc1cb5e67bc21bf58aa9647b1ffb305c72f99ea5e0",
            )?,
            workflow: load(
                "analysis-recovery-workflow.v1.sql",
                "aa0a7f7724cad693d62d605278963860de12d5df568aed950d663e0a1573b659",
            )?,
            audit: load(
                "analysis-recovery-audit.v1.sql",
                "65944c6b13103ab21574cc53168af959dd117b47636cc5eea2790b825287f337",
            )?,
            interrupt: load(
                "analysis-recovery-interrupt.v1.sql",
                "d9943119a214edd3648dc0c879a004eca35f61ae5fd8aff1fed87faab06cd94e",
            )?,
        })
    }
    pub(crate) fn statements(&self) -> RecoveryStatements<'_> {
        RecoveryStatements {
            candidates: &self.candidates,
            targets: &self.targets,
            run_stage: &self.run_stage,
            run: &self.run,
            attempt: &self.attempt,
            turn: &self.turn,
            concept: &self.concept,
            workflow: &self.workflow,
            audit: &self.audit,
        }
    }
    pub(crate) fn interrupt(
        &self,
        connection: &rusqlite::Connection,
    ) -> Result<(), rusqlite::Error> {
        connection.execute_batch(&self.interrupt)?;
        crate::ai::candidate_recovery::interrupt_submissions(connection)
    }
}
