use super::*;
use research_workbench_native_sdk::writing_project::{Decision, Input};
use research_workbench_native_sdk::{data::DataValue, writing_checkpoint::DocumentCheckpoint};
impl FileOperation {
    fn checkpoint_decision(
        &self,
        input: CheckpointInput,
    ) -> NativeResult<Ready<CheckpointDecision>> {
        let decided = self.decide(Input::Checkpoint {
            input: Box::new(input),
        })?;
        let origin = Origin {
            instance_id: decided.instance_id,
            request_id: decided.request_id,
        };
        if let Decision::Checkpoint { decision } = &decided.output.decision
            && let CheckpointDecision::Read { record } = decision.as_ref()
        {
            let Input::Checkpoint { input } = &decided.output.input else {
                return Err(origin.attach(self.checkpoint_mismatch()));
            };
            let CheckpointInput::Read { rows, .. } = input.as_ref() else {
                return Err(origin.attach(self.checkpoint_mismatch()));
            };
            if !record_matches(rows, record.as_deref()) {
                return Err(origin.attach(self.checkpoint_mismatch()));
            }
        }
        match decided.output.decision {
            Decision::Checkpoint { decision } => match *decision {
                CheckpointDecision::Rejected { reason } => {
                    Err(origin.attach(failure(reason.code(), reason.to_string(), &self.key)))
                }
                value => Ok(Ready { value, origin }),
            },
            _ => Err(origin.attach(self.checkpoint_mismatch())),
        }
    }
    pub(super) fn checkpoint_project(&self, rows: DataRows) -> NativeResult<Ready<()>> {
        let decided = self.decide(Input::GetProject {
            manuscript_id: self.manuscript_id.clone(),
            rows,
        })?;
        let origin = Origin {
            instance_id: decided.instance_id.clone(),
            request_id: decided.request_id.clone(),
        };
        let manuscript = super::super::super::project_reply(&self.key, decided)?;
        if manuscript.id != self.manuscript_id {
            return Err(origin.attach(self.checkpoint_mismatch()));
        }
        Ok(Ready { value: (), origin })
    }
    pub(super) fn checkpoint_validate(&self, write: &CheckpointWrite) -> NativeResult<Origin> {
        let Ready { value, origin } = self.checkpoint_decision(CheckpointInput::Validate {
            write: Box::new(write.clone()),
        })?;
        match value {
            CheckpointDecision::Validated { write: actual } if *actual == *write => Ok(origin),
            _ => Err(origin.attach(self.checkpoint_mismatch())),
        }
    }
    pub(super) fn checkpoint_read(
        &self,
        path: &str,
        required: bool,
        rows: DataRows,
    ) -> NativeResult<Ready<Option<CheckpointRecord>>> {
        let Ready { value, origin } = self.checkpoint_decision(CheckpointInput::Read {
            manuscript_id: self.manuscript_id.clone(),
            relative_path: path.into(),
            required,
            rows,
        })?;
        match value {
            CheckpointDecision::Read { record }
                if record.as_ref().is_none_or(|record| {
                    record.checkpoint.manuscript_id == self.manuscript_id
                        && record.checkpoint.relative_path == path
                }) && (!required || record.is_some()) =>
            {
                Ok(Ready {
                    value: record.map(|record| *record),
                    origin,
                })
            }
            _ => Err(origin.attach(self.checkpoint_mismatch())),
        }
    }
    pub(super) fn checkpoint_plan(
        &self,
        write: &CheckpointWrite,
        current: Option<CheckpointRecord>,
    ) -> NativeResult<Ready<CheckpointPlan>> {
        let Ready { value, origin } = self.checkpoint_decision(CheckpointInput::Save {
            write: Box::new(write.clone()),
            current: current.clone().map(Box::new),
        })?;
        match value {
            CheckpointDecision::Save { plan } if plan_matches(write, current.as_ref(), &plan) => {
                Ok(Ready {
                    value: *plan,
                    origin,
                })
            }
            _ => Err(origin.attach(self.checkpoint_mismatch())),
        }
    }
}

/// A decoded record must be the actual Core row, including its stored revision.
pub(crate) fn record_matches(rows: &DataRows, record: Option<&CheckpointRecord>) -> bool {
    match (rows.rows.as_slice(), record) {
        ([], None) => true,
        ([row], Some(record)) => match row.as_slice() {
            [DataValue::Integer(revision), DataValue::Text(payload)]
                if u64::try_from(*revision).ok() == Some(record.revision) =>
            {
                serde_json::from_str::<DocumentCheckpoint>(payload)
                    .is_ok_and(|checkpoint| checkpoint == record.checkpoint)
            }
            _ => false,
        },
        _ => false,
    }
}
/// Core retains the original storage CAS and counter; Native decides domain rules.
fn plan_matches(
    write: &CheckpointWrite,
    current: Option<&CheckpointRecord>,
    plan: &CheckpointPlan,
) -> bool {
    plan.write == *write
        && plan.record.checkpoint == write.checkpoint
        && if plan.write_required {
            write.expected_revision == current.map(|record| record.revision)
                && current.map_or(Some(1), |record| record.revision.checked_add(1))
                    == Some(plan.record.revision)
                && plan.record.revision <= 9_007_199_254_740_991
        } else {
            current == Some(&plan.record)
        }
}
#[cfg(test)]
mod tests;
