//! Small lifecycle views refresh at reliable boundaries, never on text deltas.
use super::*;
use research_agent_core::protocol::EventPayload;

impl AgentSessionService {
    pub(super) fn install_app_notifications(&mut self, store: &crate::storage::AppStore) {
        let activity = store.clone();
        self.activity_changed = Some(Arc::new(move || activity.notify_agent_activity_changed()));
        let subtasks = store.clone();
        self.subtasks_changed = Some(Arc::new(move || {
            subtasks.notify_agent_subtasks_changed(None)
        }));
    }

    pub(super) fn notify_lifecycle(&self, receipt: &CommittedBatch) {
        if receipt.events.iter().any(|event| {
            matches!(
                event.payload(),
                EventPayload::TurnStarted { .. }
                    | EventPayload::TurnFinished { .. }
                    | EventPayload::ConversationRolledBack { .. }
            )
        }) && let Some(notify) = &self.activity_changed
        {
            notify();
        }
    }

    pub(super) fn notify_deleted(&self) {
        if let Some(notify) = &self.activity_changed {
            notify();
        }
        if let Some(notify) = &self.subtasks_changed {
            notify();
        }
    }
}
