//! Original active-compile owner shared by static and runtime writing adapters.
use crate::agent::writing_error::WritingError;
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

#[derive(Clone, Default)]
pub(crate) struct CompileManager {
    active: Arc<Mutex<HashMap<String, ActiveCompile>>>,
}

struct ActiveCompile {
    manuscript_id: String,
    cancellation: Arc<AtomicBool>,
}

impl CompileManager {
    pub(crate) fn with_idle_manuscript<T>(
        &self,
        id: &str,
        action: impl FnOnce() -> Result<T, WritingError>,
    ) -> Result<T, WritingError> {
        let active = self
            .active
            .lock()
            .map_err(|_| WritingError::Process("编译任务管理器不可用".to_owned()))?;
        if active.values().any(|value| value.manuscript_id == id) {
            return Err(WritingError::CompileAlreadyRunning);
        }
        let result = action();
        drop(active);
        result
    }
    pub(crate) fn register(
        &self,
        run_id: &str,
        manuscript_id: &str,
    ) -> Result<Arc<AtomicBool>, WritingError> {
        let mut active = self
            .active
            .lock()
            .map_err(|_| WritingError::Process("编译任务管理器不可用".to_owned()))?;
        if active
            .values()
            .any(|value| value.manuscript_id == manuscript_id)
        {
            return Err(WritingError::CompileAlreadyRunning);
        }
        let cancellation = Arc::new(AtomicBool::new(false));
        active.insert(
            run_id.to_owned(),
            ActiveCompile {
                manuscript_id: manuscript_id.to_owned(),
                cancellation: Arc::clone(&cancellation),
            },
        );
        Ok(cancellation)
    }

    pub(crate) fn cancel(&self, run_id: &str) -> Result<(), WritingError> {
        let active = self
            .active
            .lock()
            .map_err(|_| WritingError::Process("编译任务管理器不可用".to_owned()))?;
        let compile = active.get(run_id).ok_or(WritingError::CompileRunNotFound)?;
        compile.cancellation.store(true, Ordering::Release);
        Ok(())
    }

    pub(crate) fn finish(&self, run_id: &str) {
        if let Ok(mut active) = self.active.lock() {
            active.remove(run_id);
        }
    }
}
