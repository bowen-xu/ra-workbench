//! Original merge-time storage sequence, consuming fixed package policies at T1.
//! Unlike startup preparation, this path must observe preceding trigger writes.
use super::{
    BootstrapCause, BootstrapProblem, BootstrapResources, CaptureError, CompleteNalLoadError,
    DecodedNalState, FailureContext, NalLoadResources, load_complete, rows::Row,
};
use crate::{
    data_interface::plugin_resources::{ColumnCodec as C, RowPolicy as P, write_in},
    runtime_plugins::{
        data::AcceptedDataResource,
        literature_duplicate_merge::resources::rules::MergeRuleResources,
    },
};
use research_workbench_native_sdk::{
    data::{DataRows, DataValue as V},
    nal_bootstrap::BootstrapPhaseV1 as Phase,
};
use rusqlite::Transaction;
use serde::Serialize;

mod concept;
mod legacy;
mod semantic;

type Result<T> = std::result::Result<T, BootstrapProblem>;

#[derive(Clone, Copy)]
struct Import<'a, 'connection> {
    tx: &'a Transaction<'connection>,
    storage: &'a BootstrapResources,
    rules: &'a MergeRuleResources,
    context: FailureContext,
}

pub(crate) fn import_in(
    tx: &Transaction<'_>,
    storage: &BootstrapResources,
    rules: &MergeRuleResources,
) -> Result<()> {
    let io = Import {
        tx,
        storage,
        rules,
        context: FailureContext::Network,
    };
    io.legacy()?;
    io.repair()?;
    io.cleanup()
}

impl Import<'_, '_> {
    fn problem(&self, phase: Phase, cause: CaptureError) -> BootstrapProblem {
        BootstrapProblem::capture(phase, self.context, cause)
    }

    fn invalid(&self, phase: Phase) -> BootstrapProblem {
        BootstrapProblem {
            phase,
            context: self.context,
            cause: BootstrapCause::InvalidStructure,
        }
    }

    fn read(
        &self,
        phase: Phase,
        resource: &AcceptedDataResource,
        args: Vec<V>,
        policy: P,
        codecs: &[C],
    ) -> Result<DataRows> {
        super::rows::read(self.tx, resource, args, policy, codecs)
            .map_err(|cause| self.problem(phase, cause))
    }

    fn first(
        &self,
        phase: Phase,
        resource: &AcceptedDataResource,
        args: Vec<V>,
        codecs: &[C],
    ) -> Result<Row> {
        let rows = self.read(phase, resource, args, P::FirstRequired, codecs)?;
        super::capture::first(rows).map_err(|cause| self.problem(phase, cause))
    }

    fn write(&self, phase: Phase, resource: &AcceptedDataResource, args: Vec<V>) -> Result<u64> {
        write_in(self.tx, resource, &super::rows::call(resource, args))
            .map_err(|cause| self.problem(phase, CaptureError::Data(cause)))
    }

    fn encode(&self, phase: Phase, value: &(impl Serialize + ?Sized)) -> Result<String> {
        serde_json::to_string(value).map_err(|_| self.invalid(phase))
    }

    fn text(&self, phase: Phase, resource: &AcceptedDataResource, args: Vec<V>) -> Result<String> {
        self.first(phase, resource, args, &[C::String])?
            .text()
            .map_err(|cause| self.problem(phase, cause))
    }

    fn optional(
        &self,
        phase: Phase,
        resource: &AcceptedDataResource,
        args: Vec<V>,
    ) -> Result<Option<String>> {
        self.read(phase, resource, args, P::FirstOptional, &[C::String])?
            .rows
            .into_iter()
            .next()
            .map(|row| Row::from(row).text())
            .transpose()
            .map_err(|cause| self.problem(phase, cause))
    }

    fn complete(&self, phase: Phase) -> Result<DecodedNalState> {
        load_complete(
            NalLoadResources {
                revision: &self.storage.state_revision,
                concepts: &self.storage.state_concepts,
                statements: &self.storage.state_statements,
                annotations: &self.storage.state_annotations,
                rules: &self.storage.state_rules,
            },
            |phase, resource, policy, codecs| self.read(phase, resource, vec![], policy, codecs),
        )
        .map_err(|error| match error {
            CompleteNalLoadError::Read(error) => error,
            CompleteNalLoadError::Capture { phase, cause } => self.problem(phase, cause),
            CompleteNalLoadError::Structure(_) => self.invalid(phase),
        })
    }
}

fn text(value: &str) -> V {
    V::Text(value.to_owned())
}
