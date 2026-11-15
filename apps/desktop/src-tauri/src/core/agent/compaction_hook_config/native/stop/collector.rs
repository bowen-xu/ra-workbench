//! Invocation-local callback collector; persistence remains the task owner's job.
use super::*;
use std::sync::Mutex;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StopCollectedOutcome {
    Completed(StopHandlerOutput),
    Canceled,
    NotStarted,
}
impl From<StopCollectedOutcome> for crate::agent::repository::stop_hook_audit::Outcome {
    fn from(value: StopCollectedOutcome) -> Self {
        match value {
            StopCollectedOutcome::Completed(output) => output.into(),
            StopCollectedOutcome::Canceled => Self::Canceled,
            StopCollectedOutcome::NotStarted => Self::NotStarted,
        }
    }
}

#[derive(Clone)]
enum State {
    Pending,
    Started,
    Finished(StopCollectedOutcome),
}

pub struct StopHandlerCollector {
    ids: Vec<(Uuid, Uuid)>,
    states: Mutex<Option<Vec<State>>>,
}
impl StopHandlerCollector {
    pub fn new(ids: Vec<(Uuid, Uuid)>) -> Option<Self> {
        let mut handlers = HashSet::new();
        let mut runs = HashSet::new();
        if ids.len() > 32
            || ids.iter().any(|(handler, run)| {
                handler.is_nil() || run.is_nil() || !handlers.insert(*handler) || !runs.insert(*run)
            })
        {
            return None;
        }
        Some(Self {
            states: Mutex::new(Some(vec![State::Pending; ids.len()])),
            ids,
        })
    }

    /// Once the dispatcher is finished or dropped, recover results in admitted
    /// order. A never-polled handler is only acceptable on cancellation.
    pub fn outcomes(&self, canceled: bool) -> Option<Vec<(Uuid, Uuid, StopCollectedOutcome)>> {
        let states = self.states.lock().ok()?;
        self.ids
            .iter()
            .zip(states.as_ref()?)
            .map(|((handler, run), state)| {
                let result = match state {
                    State::Finished(StopCollectedOutcome::Canceled) if !canceled => return None,
                    State::Finished(outcome) => outcome.clone(),
                    State::Pending if canceled => StopCollectedOutcome::NotStarted,
                    State::Pending | State::Started => return None,
                };
                Some((*handler, *run, result))
            })
            .collect()
    }
}
impl StopHandlerObserver for StopHandlerCollector {
    fn notify(&self, handler: Uuid, run: Uuid, event: StopHandlerNotification<'_>) {
        let Ok(mut states) = self.states.lock() else {
            return;
        };
        let Some(index) = self.ids.iter().position(|id| *id == (handler, run)) else {
            *states = None;
            return;
        };
        let Some(states_ref) = states.as_mut() else {
            return;
        };
        let state = &mut states_ref[index];
        match (&*state, event) {
            (State::Pending, StopHandlerNotification::Started) => *state = State::Started,
            (State::Started, StopHandlerNotification::Completed(output)) => {
                *state = State::Finished(StopCollectedOutcome::Completed(output.clone()))
            }
            (State::Started, StopHandlerNotification::Canceled) => {
                *state = State::Finished(StopCollectedOutcome::Canceled)
            }
            _ => *states = None,
        }
    }
}

#[cfg(test)]
#[path = "collector/tests.rs"]
mod tests;
