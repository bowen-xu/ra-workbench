use super::{
    error::{CaptureError, Result},
    rows::call,
};
use crate::{
    ai::embedding_settings,
    data_interface::plugin_resources::{ColumnCodec, RowPolicy, read_on_connection},
    runtime_plugins::{data::AcceptedDataResource, native::SafeDatabaseError},
};
use research_workbench_native_sdk::{
    data::{DataCall, DataRows, DataValue},
    nal_bootstrap::{BootstrapPhaseV1, ConfiguredEmbeddingV1},
};
use rusqlite::Connection;

#[derive(Clone)]
struct Observation {
    phase: BootstrapPhaseV1,
    resource: AcceptedDataResource,
    call: DataCall,
    policy: RowPolicy,
    codecs: Vec<ColumnCodec>,
    result: Result<DataRows>,
    logical_failure: Option<CaptureError>,
}

impl Observation {
    fn current(&self, db: &Connection) -> Result<DataRows> {
        read_on_connection(db, &self.resource, &self.call, self.policy, &self.codecs)
            .map_err(CaptureError::Data)
    }
    fn same_binding(&self, other: &Self) -> bool {
        self.phase == other.phase
            && same_resource(&self.resource, &other.resource)
            && self.call == other.call
    }
    fn failure(&self) -> Option<&CaptureError> {
        self.result.as_ref().err().or(self.logical_failure.as_ref())
    }
    fn replay_failure(&self, db: &Connection) -> Result<()> {
        let current = self.current(db);
        compare(&self.result, &current)?;
        if let Some(error) = current.err().or_else(|| self.logical_failure.clone()) {
            Err(error)
        } else {
            Err(CaptureError::changed(None))
        }
    }
}

fn same_resource(left: &AcceptedDataResource, right: &AcceptedDataResource) -> bool {
    left.package_key() == right.package_key()
        && left.owner_id() == right.owner_id()
        && left.grant().id == right.grant().id
        && left.grant().version == right.grant().version
        && left.grant().sha256 == right.grant().sha256
}

fn compare<T: PartialEq>(previous: &Result<T>, current: &Result<T>) -> Result<()> {
    match (previous, current) {
        (Ok(left), Ok(right)) if left == right => Ok(()),
        (Err(left), Err(right)) if left.same_failure(right) => Ok(()),
        (_, Err(error)) => Err(CaptureError::changed(Some(error.clone()))),
        _ => Err(CaptureError::changed(None)),
    }
}

#[derive(Clone, Default)]
pub(super) struct ReadSet {
    observations: Vec<Observation>,
    configured: Option<Result<Option<ConfiguredEmbeddingV1>>>,
}

impl ReadSet {
    pub(super) fn observe(
        &mut self,
        db: &Connection,
        phase: BootstrapPhaseV1,
        resource: &AcceptedDataResource,
        arguments: Vec<DataValue>,
        policy: RowPolicy,
        codecs: &[ColumnCodec],
    ) -> Result<DataRows> {
        let call = call(resource, arguments);
        let result =
            read_on_connection(db, resource, &call, policy, codecs).map_err(CaptureError::Data);
        self.observations.push(Observation {
            phase,
            resource: resource.clone(),
            call,
            policy,
            codecs: codecs.to_vec(),
            result: result.clone(),
            logical_failure: None,
        });
        result
    }
    pub(super) fn fail_last(&mut self, error: CaptureError) {
        if let Some(last) = self.observations.last_mut() {
            last.logical_failure = Some(error);
        }
    }
    pub(super) fn configured(&mut self, db: &Connection) -> Result<Option<ConfiguredEmbeddingV1>> {
        let value = configured(db);
        self.configured = Some(value.clone());
        value
    }
    pub(super) fn same_capture(&self, other: &Self) -> Result<()> {
        if self.observations.len() != other.observations.len() {
            return Err(CaptureError::changed(None));
        }
        for (left, right) in self.observations.iter().zip(&other.observations) {
            if !left.same_binding(right) {
                return Err(CaptureError::changed(None));
            }
            compare(&left.result, &right.result)?;
            match (&left.logical_failure, &right.logical_failure) {
                (None, None) => {}
                (Some(left), Some(right)) if left.same_failure(right) => {}
                (_, error) => return Err(CaptureError::changed(error.clone())),
            }
        }
        match (&self.configured, &other.configured) {
            (None, None) => Ok(()),
            (Some(left), Some(right)) => compare(left, right),
            _ => Err(CaptureError::changed(None)),
        }
    }
    pub(super) fn recheck_in(&self, db: &Connection) -> Result<()> {
        for observation in &self.observations {
            // A future failed read is not a successful snapshot proof. Its
            // failure/changed identity is checked only at the original caller.
            if observation.failure().is_some() {
                continue;
            }
            compare(&observation.result, &observation.current(db))?;
        }
        if let Some(previous @ Ok(_)) = &self.configured {
            compare(previous, &configured(db))?;
        }
        Ok(())
    }
    pub(super) fn require_phase_in(&self, db: &Connection, phase: BootstrapPhaseV1) -> Result<()> {
        if phase == BootstrapPhaseV1::EmbeddingSettings
            && let Some(Err(previous)) = &self.configured
        {
            let current = configured(db);
            compare(
                &Err::<Option<ConfiguredEmbeddingV1>, _>(previous.clone()),
                &current,
            )?;
            return current.map(|_| ());
        }
        if let Some(observation) = self
            .observations
            .iter()
            .find(|item| item.phase == phase && item.failure().is_some())
        {
            return observation.replay_failure(db);
        }
        Ok(())
    }
    pub(super) fn require_probe_in(
        &self,
        db: &Connection,
        phase: BootstrapPhaseV1,
        resource: &AcceptedDataResource,
        arguments: &[DataValue],
    ) -> Result<()> {
        if let Some(observation) = self.observations.iter().find(|item| {
            item.phase == phase
                && same_resource(&item.resource, resource)
                && item.call.arguments == arguments
        }) && observation.failure().is_some()
        {
            return observation.replay_failure(db);
        }
        // New inserts' original readbacks are not R1 observations. Absence from
        // this private list is not an invented missing-row proof or an error.
        Ok(())
    }

    /// Compare an original source read again at its reachable T1 position.
    /// This is only an observation check: the caller has already executed that
    /// fixed read, and no native-declared rows can replace the private R1 rows.
    pub(super) fn check_source_rows(
        &self,
        phase: BootstrapPhaseV1,
        resource: &AcceptedDataResource,
        arguments: &[DataValue],
        actual: &DataRows,
    ) -> Result<()> {
        let observation = self
            .observations
            .iter()
            .find(|item| {
                item.phase == phase
                    && same_resource(&item.resource, resource)
                    && item.call.arguments == arguments
            })
            .ok_or_else(|| CaptureError::changed(None))?;
        if let Some(failure) = observation.failure() {
            return Err(CaptureError::changed(Some(failure.clone())));
        }
        match &observation.result {
            Ok(expected) if expected == actual => Ok(()),
            _ => Err(CaptureError::changed(None)),
        }
    }
}

fn configured(db: &Connection) -> Result<Option<ConfiguredEmbeddingV1>> {
    embedding_settings::configured_space_in(db)
        .map(|value| {
            value.map(|value| ConfiguredEmbeddingV1 {
                region: value.region,
                model_id: value.model_id,
            })
        })
        .map_err(|cause| {
            CaptureError::Owner(Box::new(SafeDatabaseError::sql(
                "embedding_settings",
                &cause,
            )))
        })
}
