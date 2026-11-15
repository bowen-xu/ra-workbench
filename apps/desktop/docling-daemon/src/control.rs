//! In-process status and stop intent shared by the native menu and server.
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use tokio::sync::Notify;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Status {
    pub ready: bool,
    pub stopping: bool,
    pub clients: usize,
    pub requests: usize,
    pub port: Option<u16>,
}

impl Status {
    #[cfg(any(target_os = "macos", test))]
    pub fn label(&self) -> String {
        if self.stopping {
            "状态：正在退出，等待当前处理结束".into()
        } else if !self.ready {
            "状态：正在启动／加载模型".into()
        } else if self.requests > 0 {
            format!("状态：正在解析（{} 个任务）", self.requests)
        } else {
            "状态：运行中，等待解析".into()
        }
    }
}

#[derive(Default)]
struct Inner {
    stopped: AtomicBool,
    changed: Notify,
    status: Mutex<Status>,
    operation: Mutex<Option<String>>,
}

#[derive(Clone, Default)]
pub(crate) struct Control(Arc<Inner>);

impl Control {
    pub fn stopping(&self) -> bool {
        self.0.stopped.load(Ordering::Acquire)
    }

    pub fn request_stop(&self) -> String {
        let mut operation = self.0.operation.lock().unwrap_or_else(|e| e.into_inner());
        let id = operation
            .get_or_insert_with(|| uuid::Uuid::new_v4().to_string())
            .clone();
        self.0.stopped.store(true, Ordering::Release);
        if let Ok(mut status) = self.0.status.lock() {
            status.stopping = true;
        }
        self.0.changed.notify_waiters();
        id
    }

    pub fn operation_id(&self) -> Option<String> {
        self.0.operation.lock().ok().and_then(|id| id.clone())
    }

    pub fn update(&self, mut status: Status) {
        if let Ok(mut current) = self.0.status.lock() {
            status.stopping |= self.stopping();
            *current = status;
        }
    }

    pub fn status(&self) -> Status {
        self.0
            .status
            .lock()
            .map(|status| status.clone())
            .unwrap_or_else(|_| Status {
                stopping: true,
                ..Status::default()
            })
    }

    pub async fn stopped(&self) {
        loop {
            let changed = self.0.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if self.stopping() {
                return;
            }
            changed.await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn stopping_is_idempotent_and_cannot_be_overwritten_by_a_late_status() {
        let control = Control::default();
        assert!(control.status().label().contains("加载模型"));
        control.update(Status {
            ready: true,
            requests: 2,
            ..Status::default()
        });
        assert!(control.status().label().contains("2 个任务"));
        let id = control.request_stop();
        assert_eq!(control.request_stop(), id);
        control.update(Status {
            ready: true,
            ..Status::default()
        });
        assert!(control.status().stopping);
        tokio::time::timeout(std::time::Duration::from_millis(100), control.stopped())
            .await
            .unwrap();
    }
}
