use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU8, Ordering},
    },
};
use tokio::sync::Notify;

const RUNNING: u8 = 0;
const CANCELLED: u8 = 1;
const PUBLISHING: u8 = 2;

#[derive(Default)]
pub struct NoteExportJobs(Mutex<HashMap<String, Arc<ExportJob>>>);

pub struct ExportJob {
    phase: AtomicU8,
    claimed: AtomicBool,
    pub cancelled: Notify,
}

impl NoteExportJobs {
    pub fn begin(&self, id: &str) -> Result<(), String> {
        if uuid::Uuid::parse_str(id).is_err() {
            return Err("NOTE_EXPORT_JOB_INVALID：任务 ID 无效。".into());
        }
        let mut jobs = self
            .0
            .lock()
            .map_err(|_| "NOTE_EXPORT_JOB_STATE：任务状态不可用。")?;
        if jobs.contains_key(id) {
            return Err("NOTE_EXPORT_JOB_EXISTS：任务 ID 已使用。".into());
        }
        if jobs.len() >= 2 {
            return Err("NOTE_EXPORT_BUSY：已有两个导出任务，请先完成或停止其中一个。".into());
        }
        jobs.insert(
            id.into(),
            Arc::new(ExportJob {
                phase: AtomicU8::new(RUNNING),
                claimed: AtomicBool::new(false),
                cancelled: Notify::new(),
            }),
        );
        Ok(())
    }
    pub fn get(&self, id: &str) -> Result<Arc<ExportJob>, String> {
        self.0
            .lock()
            .map_err(|_| "NOTE_EXPORT_JOB_STATE：任务状态不可用。")?
            .get(id)
            .cloned()
            .ok_or_else(|| "NOTE_EXPORT_JOB_NOT_FOUND：导出任务不存在。".into())
    }
    pub fn cancel(&self, id: &str) -> Result<bool, String> {
        let jobs = self
            .0
            .lock()
            .map_err(|_| "NOTE_EXPORT_JOB_STATE：任务状态不可用。")?;
        // A completion may win just before a UI stop click reaches native.
        Ok(jobs.get(id).is_some_and(|job| job.cancel()))
    }
    pub fn release(&self, id: &str) -> Result<(), String> {
        if let Some(job) = self
            .0
            .lock()
            .map_err(|_| "NOTE_EXPORT_JOB_STATE：任务状态不可用。")?
            .remove(id)
        {
            job.cancel();
        }
        Ok(())
    }
}

impl ExportJob {
    pub fn claim(&self) -> Result<(), String> {
        self.check()?;
        if self.claimed.swap(true, Ordering::AcqRel) {
            return Err("NOTE_EXPORT_JOB_STARTED：任务已开始。".into());
        }
        Ok(())
    }
    pub fn check(&self) -> Result<(), String> {
        if self.phase.load(Ordering::Acquire) == CANCELLED {
            Err("NOTE_EXPORT_CANCELLED：导出已停止。".into())
        } else {
            Ok(())
        }
    }
    pub fn cancel(&self) -> bool {
        if self
            .phase
            .compare_exchange(RUNNING, CANCELLED, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            self.cancelled.notify_one();
            true
        } else {
            false
        }
    }
    /// This CAS is the publication/cancellation boundary. Cancellation before
    /// it keeps the previous destination; publication afterwards wins the race.
    pub fn begin_publish(&self) -> Result<(), String> {
        self.phase
            .compare_exchange(RUNNING, PUBLISHING, Ordering::AcqRel, Ordering::Acquire)
            .map(|_| ())
            .map_err(|_| "NOTE_EXPORT_CANCELLED：导出已停止。".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancellation_and_publication_have_one_winner_and_jobs_are_independent() {
        let jobs = NoteExportJobs::default();
        let first = uuid::Uuid::new_v4().to_string();
        let second = uuid::Uuid::new_v4().to_string();
        jobs.begin(&first).unwrap();
        jobs.begin(&second).unwrap();
        let a = jobs.get(&first).unwrap();
        let b = jobs.get(&second).unwrap();
        assert!(a.cancel());
        assert!(!a.cancel());
        assert!(a.begin_publish().is_err());
        b.claim().unwrap();
        assert!(b.claim().is_err());
        b.begin_publish().unwrap();
        assert!(!b.cancel());
        jobs.release(&first).unwrap();
        jobs.release(&second).unwrap();
        assert!(jobs.get(&first).is_err());
    }
}
