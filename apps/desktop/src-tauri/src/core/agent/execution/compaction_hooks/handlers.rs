//! Invocation-local bounded status collector. No database I/O in callbacks.
use research_agent_core::context::compaction_hooks::*;
use std::{collections::HashSet, sync::Mutex};
use uuid::Uuid;

pub(super) struct HandlerCollector {
    request: CompactionHookRequest,
    ids: Vec<Uuid>,
    states: Mutex<Option<Vec<HandlerState>>>,
}
#[derive(Clone, Copy, Default)]
struct HandlerState {
    started: bool,
    outcome: Option<CompactionHookOutcome>,
}
impl HandlerCollector {
    pub(super) fn new(request: CompactionHookRequest, ids: Vec<Uuid>) -> Option<Self> {
        let mut unique = HashSet::new();
        if ids.len() > 32 || ids.iter().any(|id| id.is_nil() || !unique.insert(*id)) {
            return None;
        }
        Some(Self {
            request,
            states: Mutex::new(Some(vec![HandlerState::default(); ids.len()])),
            ids,
        })
    }
    pub(super) fn completed(
        &self,
        aggregate: CompactionHookOutcome,
    ) -> Option<Vec<(Uuid, Option<CompactionHookOutcome>)>> {
        let states = self.states.lock().ok()?;
        self.ids
            .iter()
            .zip(states.as_ref()?)
            .map(|(id, state)| {
                let outcome = match state.outcome {
                    Some(outcome) => outcome,
                    None if aggregate == CompactionHookOutcome::Canceled => {
                        CompactionHookOutcome::Canceled
                    }
                    None => return None,
                };
                Some((*id, Some(outcome)))
            })
            .collect()
    }
}
impl HookHandlerObserver for HandlerCollector {
    fn notify(&self, id: Uuid, request: &CompactionHookRequest, event: CompactionHookNotification) {
        let Ok(mut states) = self.states.lock() else {
            return;
        };
        let Some(index) = self.ids.iter().position(|expected| *expected == id) else {
            *states = None;
            return;
        };
        if request != &self.request {
            *states = None;
            return;
        }
        let Some(entries) = states.as_mut() else {
            return;
        };
        let HandlerState { started, outcome } = &mut entries[index];
        match event {
            CompactionHookNotification::Started if !*started && outcome.is_none() => {
                *started = true
            }
            CompactionHookNotification::Completed(value) if *started && outcome.is_none() => {
                *outcome = Some(value)
            }
            _ => *states = None,
        }
    }
}

#[cfg(test)]
#[path = "handlers/tests.rs"]
mod tests;
