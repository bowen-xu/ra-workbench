//! Await the file-owned production coordinator inside the shared tool loop.
//! Only the issued, scope-bound waiter may receive a result. Dropped/expired
//! waiters never imply that a dispatched write failed or may be replayed.
use super::writing_workspace_commands::{Confirmation, Scope};
use crate::storage::AppStore;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::Mutex};
use tauri::State;
use tokio::sync::oneshot;
use uuid::Uuid;

pub(crate) const NAME: &str = "research_deliver_files";
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Request {
    pub request_id: Uuid,
    pub scope: Scope,
    pub turn_id: Uuid,
    pub binding: Confirmation,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ResultInput {
    pub request: Request,
    pub complete: bool,
    pub message: String,
}
struct Pending {
    request: Request,
    sender: oneshot::Sender<ResultInput>,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SuccessorRequest {
    pub request_id: Uuid,
    pub scope: Scope,
    pub turn_id: Uuid,
    pub generation: u32,
    pub predecessor_delivery_id: Uuid,
    pub lease_id: Uuid,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SuccessorResult {
    pub request: SuccessorRequest,
    pub confirmation: Option<Confirmation>,
}
struct PendingSuccessor {
    request: SuccessorRequest,
    sender: Option<oneshot::Sender<SuccessorResult>>,
}
#[derive(Default)]
pub(crate) struct DeliveryTools {
    events: Option<crate::runtime_events::RuntimeEvents>,
    pending: Mutex<HashMap<Uuid, Pending>>,
    successors: Mutex<HashMap<Uuid, PendingSuccessor>>,
}
struct SuccessorGuard<'a> {
    owner: &'a DeliveryTools,
    request: Uuid,
}
impl Drop for SuccessorGuard<'_> {
    fn drop(&mut self) {
        if let Ok(mut pending) = self.owner.successors.lock() {
            pending.remove(&self.request);
        }
    }
}
struct PendingGuard<'a> {
    owner: &'a DeliveryTools,
    request: Uuid,
}
impl Drop for PendingGuard<'_> {
    fn drop(&mut self) {
        if let Ok(mut pending) = self.owner.pending.lock() {
            pending.remove(&self.request);
        }
    }
}
impl DeliveryTools {
    /// Ask the file owner to capture live editor buffers and prepare this exact
    /// successor. A lost waiter never authorizes another preparation attempt.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub(crate) async fn request_successor(
        &self,
        request: SuccessorRequest,
    ) -> Result<Confirmation, String> {
        request
            .scope
            .validate()
            .map_err(|_| "invalid successor scope")?;
        if request.request_id.is_nil()
            || request.turn_id.is_nil()
            || request.lease_id.is_nil()
            || request.predecessor_delivery_id.is_nil()
            || request.generation == 0
            || request.generation > i32::MAX as u32
        {
            return Err("invalid successor identity".into());
        }
        let events = self.events.as_ref().ok_or("file owner unavailable")?;
        let (sender, receiver) = oneshot::channel();
        {
            let mut pending = self
                .successors
                .lock()
                .map_err(|_| "successor registry unavailable")?;
            if pending.len() >= 256 || pending.contains_key(&request.request_id) {
                return Err("successor request already pending".into());
            }
            pending.insert(
                request.request_id,
                PendingSuccessor {
                    request: request.clone(),
                    sender: Some(sender),
                },
            );
        }
        let _guard = SuccessorGuard {
            owner: self,
            request: request.request_id,
        };
        events.emit("research-writing-successor-workspace", &request)?;
        let result = tokio::time::timeout(std::time::Duration::from_secs(30), receiver)
            .await
            .map_err(|_| "successor result unknown; reconcile original preparation")?
            .map_err(|_| "successor result unknown; reconcile original preparation")?;
        result.confirmation.ok_or_else(|| {
            "successor preparation unconfirmed; reconcile original preparation".into()
        })
    }

    pub(crate) fn report_successor(&self, result: SuccessorResult) -> Result<(), String> {
        let mut pending = self
            .successors
            .lock()
            .map_err(|_| "successor registry unavailable")?;
        let entry = pending
            .get_mut(&result.request.request_id)
            .ok_or("successor waiter expired")?;
        if serde_json::to_value(&entry.request).ok() != serde_json::to_value(&result.request).ok() {
            return Err("successor request identity changed".into());
        }
        if let Some(confirmation) = &result.confirmation
            && (confirmation.binding().is_err()
                || confirmation.turn_id != entry.request.turn_id
                || confirmation.lease_id != entry.request.lease_id
                || serde_json::to_value(&confirmation.scope).ok()
                    != serde_json::to_value(&entry.request.scope).ok())
        {
            return Err("successor confirmation identity changed".into());
        }
        entry
            .sender
            .take()
            .ok_or("successor already reported")?
            .send(result)
            .map_err(|_| "successor waiter closed".into())
    }
    pub(crate) fn for_app(app: tauri::AppHandle) -> Self {
        Self::for_events(crate::runtime_events::RuntimeEvents::for_app(app))
    }
    pub(crate) fn for_events(events: crate::runtime_events::RuntimeEvents) -> Self {
        Self {
            events: Some(events),
            ..Self::default()
        }
    }
    pub(crate) async fn deliver(
        &self,
        scope: Scope,
        turn_id: Uuid,
        binding: Confirmation,
    ) -> Result<ResultInput, String> {
        if binding.turn_id != turn_id
            || serde_json::to_value(&binding.scope).ok() != serde_json::to_value(&scope).ok()
            || binding.binding().is_err()
        {
            return Err("file delivery workspace identity changed".into());
        }
        let events = self
            .events
            .as_ref()
            .ok_or("file delivery host unavailable")?;
        let request = Request {
            request_id: Uuid::new_v4(),
            scope,
            turn_id,
            binding,
        };
        let (sender, receiver) = oneshot::channel();
        self.pending
            .lock()
            .map_err(|_| "file delivery registry unavailable")?
            .insert(
                request.request_id,
                Pending {
                    request: request.clone(),
                    sender,
                },
            );
        let _guard = PendingGuard {
            owner: self,
            request: request.request_id,
        };
        let emitted = events.emit("research-writing-file-delivery", &request);
        let result = if emitted.is_ok() {
            match tokio::time::timeout(std::time::Duration::from_secs(30), receiver).await {
                Ok(Ok(result)) => Ok(result),
                _ => Err("File delivery result unknown. Reconcile the original operation; do not replay the patch.".into()),
            }
        } else {
            Err("file delivery host unavailable".into())
        };
        self.pending
            .lock()
            .map_err(|_| "file delivery registry unavailable")?
            .remove(&request.request_id);
        result
    }
    pub(crate) fn report(&self, result: ResultInput) -> Result<(), String> {
        if result.message.len() > 4096 {
            return Err("file delivery result too large".into());
        }
        let mut pending = self
            .pending
            .lock()
            .map_err(|_| "file delivery registry unavailable")?;
        let entry = pending
            .get(&result.request.request_id)
            .ok_or("file delivery waiter expired")?;
        if serde_json::to_value(&entry.request).ok() != serde_json::to_value(&result.request).ok() {
            return Err("file delivery identity changed".into());
        }
        let entry = pending
            .remove(&result.request.request_id)
            .ok_or("file delivery waiter expired")?;
        entry
            .sender
            .send(result)
            .map_err(|_| "file delivery waiter closed".into())
    }
}
#[tauri::command]
pub(crate) fn agent_report_writing_successor_workspace(
    store: State<'_, AppStore>,
    result: SuccessorResult,
) -> Result<(), String> {
    store.writing_delivery_tools().report_successor(result)
}
#[tauri::command]
pub(crate) fn agent_report_writing_file_delivery(
    store: State<'_, AppStore>,
    result: ResultInput,
) -> Result<(), String> {
    store.writing_delivery_tools().report(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request() -> Request {
        let scope = Scope {
            conversation_id: Uuid::new_v4(),
            request_id: Uuid::new_v4(),
            manuscript_id: Uuid::new_v4(),
            context_snapshot_id: Uuid::new_v4(),
        };
        let turn_id = Uuid::new_v4();
        Request {
            request_id: Uuid::new_v4(),
            turn_id,
            scope: scope.clone(),
            binding: Confirmation {
                scope,
                turn_id,
                lease_id: Uuid::new_v4(),
                manifest_hash: "a".repeat(64),
            },
        }
    }
    #[tokio::test]
    async fn scope_bound_receipt_cannot_be_retargeted_or_consumed_twice() {
        let tools = DeliveryTools::default();
        let request = request();
        let (sender, receiver) = oneshot::channel();
        tools.pending.lock().unwrap().insert(
            request.request_id,
            Pending {
                request: request.clone(),
                sender,
            },
        );
        let mut changed = request.clone();
        changed.scope.manuscript_id = Uuid::new_v4();
        assert!(
            tools
                .report(ResultInput {
                    request: changed,
                    complete: true,
                    message: String::new()
                })
                .is_err()
        );
        let mut wrong_lease = request.clone();
        wrong_lease.binding.lease_id = Uuid::new_v4();
        assert!(
            tools
                .report(ResultInput {
                    request: wrong_lease,
                    complete: true,
                    message: String::new()
                })
                .is_err()
        );
        tools
            .report(ResultInput {
                request: request.clone(),
                complete: false,
                message: "save failed".into(),
            })
            .unwrap();
        assert!(!receiver.await.unwrap().complete);
        assert!(
            tools
                .report(ResultInput {
                    request,
                    complete: true,
                    message: String::new()
                })
                .is_err()
        );
    }
    #[test]
    fn cancellation_removes_only_the_waiter() {
        let tools = DeliveryTools::default();
        let request = request();
        let (sender, _receiver) = oneshot::channel();
        tools.pending.lock().unwrap().insert(
            request.request_id,
            Pending {
                request: request.clone(),
                sender,
            },
        );
        drop(PendingGuard {
            owner: &tools,
            request: request.request_id,
        });
        assert!(tools.pending.lock().unwrap().is_empty());
    }
}
