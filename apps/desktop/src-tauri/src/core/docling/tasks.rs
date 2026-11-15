//! Bounded manual task handles. Persistent parse records remain owned by the repository.
use super::{
    error::{DoclingCommandError, DoclingError},
    task_control::{ParseControl, ParseProgress},
};
use crate::storage::AppStore;
use futures_util::FutureExt;
use serde::Serialize;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParseTaskStatus {
    id: String,
    state: String,
    #[serde(flatten)]
    progress: ParseProgress,
    parse_run_id: Option<String>,
    error: Option<DoclingCommandError>,
}
struct Task {
    control: Arc<ParseControl>,
    last_access: Mutex<Instant>,
    result: Mutex<Option<Result<String, DoclingCommandError>>>,
}
impl Task {
    fn status(&self) -> ParseTaskStatus {
        let result = self.result.lock().unwrap();
        let state = match result.as_ref() {
            Some(Ok(_)) => "succeeded",
            Some(Err(error)) if error.is_cancelled() => "cancelled",
            Some(Err(_)) => "failed",
            None if self.control.is_cancelled() => "cancelling",
            None => "running",
        };
        ParseTaskStatus {
            id: self.control.id.clone(),
            state: state.into(),
            progress: self.control.snapshot(),
            parse_run_id: result.as_ref().and_then(|r| r.as_ref().ok()).cloned(),
            error: result.as_ref().and_then(|r| r.as_ref().err()).cloned(),
        }
    }
}
#[derive(Default)]
pub(super) struct ParseTasks(Mutex<HashMap<String, Arc<Task>>>);
impl ParseTasks {
    fn add(&self) -> Result<Arc<Task>, DoclingError> {
        let mut tasks = self.0.lock().map_err(|_| DoclingError::Disconnected)?;
        tasks.retain(|_, task| {
            task.result.lock().unwrap().is_none()
                || task.last_access.lock().unwrap().elapsed() < Duration::from_secs(600)
        });
        if tasks.len() >= 128 {
            let oldest = tasks
                .iter()
                .filter(|(_, task)| task.result.lock().unwrap().is_some())
                .min_by_key(|(_, task)| *task.last_access.lock().unwrap())
                .map(|(id, _)| id.clone());
            if let Some(id) = oldest {
                tasks.remove(&id);
            }
        }
        if tasks.len() >= 128 {
            return Err(DoclingError::Request("解析任务过多，请稍后重试".into()));
        }
        let task = Arc::new(Task {
            control: ParseControl::new(),
            last_access: Mutex::new(Instant::now()),
            result: Mutex::new(None),
        });
        tasks.insert(task.control.id.clone(), task.clone());
        Ok(task)
    }
    fn get(&self, id: &str) -> Result<Arc<Task>, DoclingError> {
        let task = self
            .0
            .lock()
            .map_err(|_| DoclingError::Disconnected)?
            .get(id)
            .cloned()
            .ok_or_else(|| DoclingError::Request("解析任务已过期，请重新解析".into()))?;
        *task.last_access.lock().unwrap() = Instant::now();
        Ok(task)
    }
}
pub(super) fn start(
    store: AppStore,
    item: String,
    attachment: String,
    force: bool,
) -> Result<ParseTaskStatus, DoclingError> {
    let preparation = store
        .docling
        .get()
        .ok_or(DoclingError::Disconnected)?
        .clone();
    let task = preparation.tasks.add()?;
    let status = task.status();
    tauri::async_runtime::spawn(async move {
        let outcome = std::panic::AssertUnwindSafe(preparation.prepare_controlled(
            &store,
            &item,
            &attachment,
            force,
            Some(task.control.clone()),
        ))
        .catch_unwind()
        .await;
        let result = match outcome {
            Ok(result) => result
                .map(|run| run.summary.id)
                .map_err(|error| DoclingCommandError::from(error.as_ref())),
            Err(_) => Err(DoclingCommandError::from(DoclingError::Request(
                "解析任务意外退出，请重试".into(),
            ))),
        };
        *task.result.lock().unwrap() = Some(result);
        *task.last_access.lock().unwrap() = Instant::now();
    });
    Ok(status)
}
pub(super) fn status(
    store: &AppStore,
    id: &str,
    cancel: bool,
) -> Result<ParseTaskStatus, DoclingError> {
    let preparation = store.docling.get().ok_or(DoclingError::Disconnected)?;
    let task = preparation.tasks.get(id)?;
    if cancel && task.result.lock().unwrap().is_none() {
        task.control.cancel();
    }
    Ok(task.status())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn finished_handles_are_evicted_before_blocking_new_tasks() {
        let tasks = ParseTasks::default();
        for _ in 0..128 {
            let task = tasks.add().unwrap();
            *task.result.lock().unwrap() = Some(Ok("run".into()));
        }
        let fresh = tasks.add().unwrap();
        assert!(tasks.get(&fresh.control.id).is_ok());
        assert_eq!(tasks.0.lock().unwrap().len(), 128);
    }
    #[test]
    fn capacity_does_not_evict_an_active_task() {
        let tasks = ParseTasks::default();
        let first = tasks.add().unwrap();
        for _ in 1..128 {
            tasks.add().unwrap();
        }
        assert!(tasks.add().is_err());
        assert!(tasks.get(&first.control.id).is_ok());
    }
}
