//! Reuse full PreparedDataRead proofs; replay deferred SQL failures only at the
//! original reachable caller, as in the existing NAL startup adapter.
use super::super::super::{data_failure, failure};
use crate::{
    data_interface::plugin_resources::{PluginDataError, PreparedDataRead},
    runtime_plugins::{data::AcceptedDataResource, native::NativeResult},
};
use research_workbench_native_sdk::data::{DataCall, DataRows, DataValue};
use rusqlite::Transaction;

pub(super) struct Observation {
    resource: AcceptedDataResource,
    call: DataCall,
    captured: Result<PreparedDataRead, Box<PluginDataError>>,
}
impl Observation {
    pub(super) fn available(&self) -> bool {
        self.captured.is_ok()
    }
    pub(super) fn capture(tx: &Transaction<'_>, resource: &AcceptedDataResource, id: &str) -> Self {
        let call = DataCall {
            id: resource.grant().id.clone(),
            version: 1,
            arguments: vec![DataValue::Text(id.into())],
        };
        let captured = PreparedDataRead::capture(tx, resource.clone(), call.clone());
        Self {
            resource: resource.clone(),
            call,
            captured,
        }
    }
    pub(super) fn rows(&self) -> Option<DataRows> {
        self.captured.as_ref().ok().map(|read| read.rows().clone())
    }
    pub(super) fn require(&self, tx: &Transaction<'_>) -> NativeResult<()> {
        match &self.captured {
            Ok(read) => {
                read.recheck_in(tx).map_err(data_failure)?;
                Ok(())
            }
            Err(previous) => {
                let current =
                    PreparedDataRead::capture(tx, self.resource.clone(), self.call.clone());
                if let Err(current) = current {
                    // Both JSON views omit SQL, values and original private text.
                    if matches!((serde_json::to_value(previous),serde_json::to_value(&current)),(Ok(a),Ok(b)) if a==b)
                    {
                        return Err(data_failure(current));
                    }
                }
                Err(self.changed())
            }
        }
    }
    fn changed(&self) -> Box<crate::runtime_plugins::native::NativeHostError> {
        let mut error = failure(
            "plugin_data_snapshot_changed",
            "目录关联读取状态已变化；本次关联事务未提交。",
            self.resource.package_key(),
        );
        error.data_error = Some(Box::new(PluginDataError {
            code: "plugin_data_snapshot_changed",
            message: "目录关联读取状态已变化；本次关联事务未提交。",
            package_key: self.resource.package_key().into(),
            resource_id: self.resource.grant().id.clone(),
            resource_version: 1,
            phase: "recheck",
            safe_cause: "writing_directory_read_set_changed".into(),
            sqlite_code: None,
            sqlite_extended_code: None,
            original_sql_failure: None,
        }));
        error
    }
}

pub(super) struct Relations {
    pub(super) manuscript: Observation,
    pub(super) files: Observation,
    pub(super) drafts: Observation,
    pub(super) session: Observation,
}
impl Relations {
    pub(super) fn all_available(&self) -> bool {
        [&self.manuscript, &self.files, &self.drafts, &self.session]
            .iter()
            .all(|value| value.available())
    }
    pub(super) fn require_all(&self, tx: &Transaction<'_>) -> NativeResult<()> {
        for value in [&self.manuscript, &self.files, &self.drafts, &self.session] {
            value.require(tx)?;
        }
        Ok(())
    }
}
