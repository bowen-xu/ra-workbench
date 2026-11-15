//! Browser-initiated capture with fixed-address discovery and receiver checks.
#[path = "identity.rs"]
mod identity;
#[cfg(test)]
#[path = "identity_lifecycle_tests.rs"]
mod identity_lifecycle_tests;
#[path = "protocol.rs"]
mod protocol;
#[path = "server.rs"]
mod server;

use std::{
    net::{Ipv4Addr, SocketAddrV4},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use serde::Serialize;
use tauri::{AppHandle, State};
use tokio::sync::oneshot;

use crate::storage::AppStore;
use identity::ReceiverIdentity;
mod request;
use crate::runtime_plugins::literature_browser_capture::CaptureFailure;
pub(crate) use request::CaptureRequest;
pub(crate) type CaptureHandler =
    fn(&AppStore, &str, CaptureRequest) -> Result<serde_json::Value, CaptureFailure>;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConnectorStatus {
    pub(crate) endpoint: String,
    pub(crate) available: bool,
    pub(crate) message: String,
}

pub(crate) struct BrowserConnector {
    endpoint: String,
    message: String,
    shared: Option<Arc<Shared>>,
    stop: Mutex<Option<oneshot::Sender<()>>>,
    task: Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
}

struct Shared {
    store: AppStore,
    capture: CaptureHandler,
    identity: ReceiverIdentity,
    environment: &'static str,
    instance_id: String,
    connection_token: String,
    active: AtomicBool,
    host: String,
    app: Option<AppHandle>,
}

fn bind_receiver(port: u16) -> Result<std::net::TcpListener, String> {
    let listener = std::net::TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, port))
        .map_err(|_| format!("浏览器采集端口 {port} 无法使用，可能被其他软件或另一应用实例占用。请检查占用情况后重启本应用。"))?;
    listener
        .set_nonblocking(true)
        .map_err(|_| "无法启动浏览器采集服务".to_owned())?;
    Ok(listener)
}

impl BrowserConnector {
    pub(crate) fn start(
        app: &AppHandle,
        store: AppStore,
        capture: Result<CaptureHandler, String>,
    ) -> Self {
        let port = if app.config().identifier.ends_with(".dev") {
            23121
        } else {
            23120
        };
        let host = format!("127.0.0.1:{port}");
        let endpoint = format!("http://{host}/");
        let capture = match capture {
            Ok(capture) => capture,
            Err(message) => {
                return Self {
                    endpoint,
                    message,
                    shared: None,
                    stop: Mutex::new(None),
                    task: Mutex::new(None),
                };
            }
        };
        // Receiver identity belongs to the selected data store, including fixtures.
        let directory = store
            .content_directory()
            .parent()
            .map(|path| path.join("browser-connector"));
        let prepared = directory
            .ok_or_else(|| "无法定位浏览器连接配置".to_owned())
            .and_then(|directory| ReceiverIdentity::open(&directory))
            .and_then(|identity| {
                let listener = bind_receiver(port)?;
                Ok((identity, listener))
            });
        match prepared {
            Ok((identity, listener)) => {
                let shared = Arc::new(Shared {
                    store,
                    capture,
                    identity,
                    environment: if port == 23121 {
                        "development"
                    } else {
                        "production"
                    },
                    instance_id: uuid::Uuid::new_v4().simple().to_string(),
                    connection_token: identity::new_connection_token(),
                    active: AtomicBool::new(true),
                    host,
                    app: Some(app.clone()),
                });
                let (stop, stopped) = oneshot::channel();
                let task_shared = shared.clone();
                let task = tauri::async_runtime::spawn(async move {
                    server::run(listener, task_shared, stopped).await;
                });
                Self {
                    endpoint,
                    message: "浏览器采集服务已就绪".to_owned(),
                    shared: Some(shared),
                    stop: Mutex::new(Some(stop)),
                    task: Mutex::new(Some(task)),
                }
            }
            Err(message) => Self {
                endpoint,
                message,
                shared: None,
                stop: Mutex::new(None),
                task: Mutex::new(None),
            },
        }
    }

    pub(crate) fn status(&self) -> ConnectorStatus {
        let available = self
            .shared
            .as_ref()
            .is_some_and(|state| state.active.load(Ordering::Acquire));
        let message = if self.shared.is_some() && !available {
            "浏览器采集服务已停止，请重启应用".to_owned()
        } else {
            self.message.clone()
        };
        ConnectorStatus {
            endpoint: self.endpoint.clone(),
            available,
            message,
        }
    }

    /// Stop admission and wait for in-flight database/file operations before exit.
    pub(crate) async fn shutdown(&self) -> Result<(), String> {
        if let Some(shared) = &self.shared {
            shared.active.store(false, Ordering::Release);
        }
        let stop = self
            .stop
            .lock()
            .map_err(|_| "浏览器采集服务暂不可用")?
            .take();
        if let Some(stop) = stop {
            let _ = stop.send(());
        }
        let task = self
            .task
            .lock()
            .map_err(|_| "浏览器采集服务暂不可用")?
            .take();
        if let Some(task) = task {
            task.await.map_err(|_| "浏览器采集尚未完成，请重试退出")?;
        }
        if let Some(shared) = &self.shared {
            let shared = shared.clone();
            // A cancelled HTTP waiter does not cancel an admitted blocking import.
            // Draining shares the import lease lock and waits for actual writes.
            tauri::async_runtime::spawn_blocking(move || shared.identity.wait_for_imports())
                .await
                .map_err(|_| "浏览器采集尚未完成，请重试退出")??;
        }
        Ok(())
    }
}

impl Drop for BrowserConnector {
    fn drop(&mut self) {
        if let Some(shared) = &self.shared {
            shared.active.store(false, Ordering::Release);
        }
        if let Ok(stop) = self.stop.get_mut()
            && let Some(stop) = stop.take()
        {
            let _ = stop.send(());
        }
    }
}

#[tauri::command]
pub(crate) fn get_browser_connector_status(
    connector: State<'_, BrowserConnector>,
) -> ConnectorStatus {
    connector.status()
}

#[test]
fn occupied_port_reports_any_other_software_without_contacting_or_displacing_it() {
    let other = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("other application");
    let port = other.local_addr().expect("address").port();
    other.set_nonblocking(true).expect("nonblocking");
    let error = bind_receiver(port).expect_err("occupied port");
    assert!(error.contains("其他软件"));
    assert!(error.contains(&port.to_string()));
    assert_eq!(
        other
            .accept()
            .expect_err("no request to the other software")
            .kind(),
        std::io::ErrorKind::WouldBlock
    );
}

#[cfg(all(test, unix))]
mod actual_package_tests;
#[cfg(test)]
mod test_support;
