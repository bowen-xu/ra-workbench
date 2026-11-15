//! Cancellable, read-only history wait scoped to the requesting webview.
use super::{
    service::AgentSessionService,
    session_commands::{self, SessionCommandError, WaitRequest, WaitResponse},
};
use std::{
    future::Future,
    sync::{Arc, Mutex, Weak},
};
use tauri::{Manager, Resource, ResourceId, ResourceTable, State, Webview};
use tokio::sync::watch;

// The table owns the sender; dropping it wakes the wait. The window registry
// also signals destruction because Tauri's async IPC resolver holds the view.
struct HistoryWaitResource {
    lifetime: watch::Sender<bool>,
}
impl Resource for HistoryWaitResource {}

#[derive(Default)]
struct WindowWaitState {
    closed: bool,
    resources: Vec<Weak<HistoryWaitResource>>,
}
#[derive(Default)]
struct WindowWaits(Mutex<WindowWaitState>);
impl Resource for WindowWaits {}
impl WindowWaits {
    fn track(&self, resource: Weak<HistoryWaitResource>) {
        let mut state = self.0.lock().unwrap_or_else(|error| error.into_inner());
        if state.closed {
            if let Some(resource) = resource.upgrade() {
                resource.lifetime.send_replace(true);
            }
            return;
        }
        state
            .resources
            .retain(|resource| resource.strong_count() > 0);
        state.resources.push(resource);
    }

    fn mark_closed(&self) {
        let resources = {
            let mut state = self.0.lock().unwrap_or_else(|error| error.into_inner());
            state.closed = true;
            std::mem::take(&mut state.resources)
        };
        for resource in resources
            .into_iter()
            .filter_map(|resource| resource.upgrade())
        {
            resource.lifetime.send_replace(true);
        }
    }
}

fn window_waits(table: &mut ResourceTable) -> Arc<WindowWaits> {
    if let Some(marker) = table
        .names()
        .find_map(|(id, _)| table.get::<WindowWaits>(id).ok())
    {
        return marker;
    }
    let marker = Arc::new(WindowWaits::default());
    table.add_arc(marker.clone());
    marker
}

pub fn close_window(window: &tauri::Window) {
    window_waits(&mut window.resources_table()).mark_closed();
}

fn create(table: &mut ResourceTable) -> ResourceId {
    let (lifetime, _) = watch::channel(false);
    table.add(HistoryWaitResource { lifetime })
}

fn subscribe(
    table: &ResourceTable,
    resource_id: ResourceId,
) -> Option<(watch::Receiver<bool>, Weak<HistoryWaitResource>)> {
    table
        .get::<HistoryWaitResource>(resource_id)
        .ok()
        .map(|resource| (resource.lifetime.subscribe(), Arc::downgrade(&resource)))
}

fn finish(table: &mut ResourceTable, resource_id: ResourceId, owner: &Weak<HistoryWaitResource>) {
    if let Ok(current) = table.get::<HistoryWaitResource>(resource_id)
        && Weak::ptr_eq(owner, &Arc::downgrade(&current))
    {
        drop(current);
        cancel(table, resource_id);
    }
}

fn cancel(table: &mut ResourceTable, resource_id: ResourceId) {
    // Typed take is idempotent and cannot remove an unrelated webview resource.
    let _ = table.take::<HistoryWaitResource>(resource_id);
}

async fn until_cancelled<T>(
    mut lifetime: watch::Receiver<bool>,
    future: impl Future<Output = T>,
) -> Option<T> {
    if *lifetime.borrow() {
        return None;
    }
    tokio::select! {
        biased;
        _ = lifetime.changed() => None,
        result = future => Some(result),
    }
}

#[tauri::command]
pub fn agent_create_history_wait(webview: Webview) -> ResourceId {
    let mut table = webview.resources_table();
    let id = create(&mut table);
    if let Some((_, owner)) = subscribe(&table, id) {
        window_waits(&mut webview.window().resources_table()).track(owner);
    }
    id
}

#[tauri::command]
pub fn agent_cancel_history_wait(webview: Webview, resource_id: ResourceId) {
    cancel(&mut webview.resources_table(), resource_id);
}

#[tauri::command]
pub async fn agent_wait_for_history_cancellable(
    webview: Webview,
    service: State<'_, Arc<AgentSessionService>>,
    request: WaitRequest,
    resource_id: ResourceId,
) -> Result<WaitResponse, SessionCommandError> {
    let (lifetime, owner) =
        subscribe(&webview.resources_table(), resource_id).ok_or(SessionCommandError::Cancelled)?;
    let result = until_cancelled(
        lifetime,
        session_commands::wait_for_history(&service, request),
    )
    .await;
    finish(&mut webview.resources_table(), resource_id, &owner);
    result.unwrap_or(Err(SessionCommandError::Cancelled))
}

#[cfg(test)]
#[path = "history_wait_commands/tests.rs"]
mod tests;
