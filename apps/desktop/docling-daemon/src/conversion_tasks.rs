//! Authenticated per-conversion control; cancellation never shuts down the shared daemon.
use crate::State;
use axum::{
    Json,
    extract::{Path, State as AppState},
    http::{HeaderMap, StatusCode},
};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

pub(crate) struct Task {
    owner: String,
    pub cancel: Arc<AtomicBool>,
    progress: Mutex<(usize, usize, &'static str)>,
    created: Instant,
    wake: tokio::sync::Notify,
}
impl Task {
    fn new(owner: &str) -> Arc<Self> {
        Arc::new(Self {
            owner: owner.into(),
            cancel: Arc::new(AtomicBool::new(false)),
            progress: Mutex::new((0, 0, "queued")),
            created: Instant::now(),
            wake: tokio::sync::Notify::new(),
        })
    }
    fn request_cancel(&self) {
        self.cancel.store(true, Ordering::Release);
        self.wake.notify_waiters();
    }
    pub async fn cancelled(&self) {
        loop {
            let wake = self.wake.notified();
            tokio::pin!(wake);
            wake.as_mut().enable();
            if self.cancel.load(Ordering::Acquire) {
                return;
            }
            wake.await;
        }
    }
    pub fn stage(&self, stage: &'static str) {
        if let Ok(mut progress) = self.progress.lock() {
            progress.2 = stage;
        }
    }
    pub fn pages(&self, done: usize, total: usize) {
        if let Ok(mut progress) = self.progress.lock() {
            progress.0 = progress.0.max(done);
            progress.1 = total;
            progress.2 = "parsing";
        }
    }
    pub fn snapshot(&self) -> Value {
        let (done, total, stage) = *self.progress.lock().unwrap();
        json!({"stage": if self.cancel.load(Ordering::Acquire) { "cancelling" } else { stage },
            "completedPages": done, "totalPages": total})
    }
}
#[derive(Default)]
pub(crate) struct Tasks(Mutex<HashMap<String, Arc<Task>>>);
impl Tasks {
    pub fn cancel_all(&self) -> Result<(), &'static str> {
        let tasks = self.0.lock().map_err(|_| "task registry unavailable")?;
        for task in tasks.values() {
            task.request_cancel();
        }
        Ok(())
    }
    pub fn register(&self, id: &str, owner: &str) -> Result<Arc<Task>, StatusCode> {
        let mut tasks = self.0.lock().map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        tasks.retain(|_, task| {
            task.created.elapsed() < Duration::from_secs(120)
                || !task.cancel.load(Ordering::Acquire)
        });
        if let Some(task) = tasks.get(id) {
            return Err(
                if task.owner == owner && task.cancel.load(Ordering::Acquire) {
                    StatusCode::GONE
                } else {
                    StatusCode::CONFLICT
                },
            );
        }
        if tasks.len() >= 128 {
            return Err(StatusCode::TOO_MANY_REQUESTS);
        }
        let task = Task::new(owner);
        tasks.insert(id.into(), task.clone());
        Ok(task)
    }
    pub fn remove(&self, id: &str) {
        if let Ok(mut tasks) = self.0.lock() {
            tasks.remove(id);
        }
    }
    fn owned(&self, id: &str, owner: &str, cancel: bool) -> Result<Arc<Task>, StatusCode> {
        let mut tasks = self.0.lock().map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        if let Some(task) = tasks.get(id) {
            if task.owner != owner {
                return Err(StatusCode::NOT_FOUND);
            }
            if cancel {
                task.request_cancel();
            }
            return Ok(task.clone());
        }
        if !cancel {
            return Err(StatusCode::NOT_FOUND);
        }
        // Retain an early cancellation until a request still in transit arrives.
        tasks.retain(|_, task| {
            task.created.elapsed() < Duration::from_secs(120)
                || !task.cancel.load(Ordering::Acquire)
        });
        if tasks.len() >= 128 {
            return Err(StatusCode::TOO_MANY_REQUESTS);
        }
        let task = Task::new(owner);
        task.request_cancel();
        tasks.insert(id.into(), task.clone());
        Ok(task)
    }
}
fn owner(state: &State, headers: &HeaderMap) -> Result<String, StatusCode> {
    if !crate::service::authorized(state, headers) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let owner = headers
        .get("X-Docling-Lease-Id")
        .and_then(|s| s.to_str().ok())
        .ok_or(StatusCode::UNAUTHORIZED)?;
    let life = state
        .lifecycle
        .lock()
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    if life
        .leases
        .get(owner)
        .is_none_or(|l| l.seen.elapsed() >= Duration::from_secs(30))
    {
        return Err(StatusCode::UNAUTHORIZED);
    }
    Ok(owner.into())
}
async fn control(
    state: Arc<State>,
    headers: HeaderMap,
    id: String,
    cancel: bool,
) -> Result<Json<Value>, StatusCode> {
    if uuid::Uuid::parse_str(&id).is_err() {
        return Err(StatusCode::BAD_REQUEST);
    }
    let owner = owner(&state, &headers)?;
    Ok(Json(state.tasks.owned(&id, &owner, cancel)?.snapshot()))
}
pub(crate) async fn status(
    AppState(state): AppState<Arc<State>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>, StatusCode> {
    control(state, headers, id, false).await
}
pub(crate) async fn cancel(
    AppState(state): AppState<Arc<State>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>, StatusCode> {
    control(state, headers, id, true).await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancellation_is_owned_and_wins_over_late_admission() {
        let tasks = Tasks::default();
        let id = uuid::Uuid::new_v4().to_string();
        tasks.owned(&id, "lease-a", true).unwrap();
        assert_eq!(tasks.register(&id, "lease-a").err(), Some(StatusCode::GONE));
        assert!(tasks.owned(&id, "lease-b", true).is_err());
        let other = tasks.register("other", "lease-b").unwrap();
        other.pages(2, 10);
        other.pages(1, 10);
        assert_eq!(other.snapshot()["completedPages"], 2);
        assert!(!other.cancel.load(Ordering::Acquire));
    }
}
