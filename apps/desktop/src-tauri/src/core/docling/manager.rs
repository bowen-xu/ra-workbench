#[path = "manager/conversion.rs"]
mod conversion;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use reqwest::Client;
use serde_json::{Value, json};
use tauri::{AppHandle, Manager, path::BaseDirectory};
use tokio::{
    sync::{Mutex as AsyncMutex, watch},
    task::JoinHandle,
};
use uuid::Uuid;

use super::{
    error::DoclingError,
    launcher::{
        STARTUP_TIMEOUT, acquire_start_lock, cleanup_failed_start, create_private_directory,
        process_is_alive, recorded_pid_is_alive, spawn_daemon,
    },
    lease::{open_lease, run_lease_loop},
    model::{DoclingPingResult, DoclingServiceStatus, DoclingSettings},
    protocol::{
        DaemonStatus, LEASE_HEADER, PROTOCOL_VERSION, RegistryRecord, SCHEMA_VERSION, STATUS_PATH,
        read_registry, validate_model_identity, validate_registry, validate_status,
    },
};
#[path = "manager/lifecycle.rs"]
mod lifecycle;
#[path = "manager/storage.rs"]
mod storage;

const REGISTRY_FILE_NAME: &str = "docling-service.json";
const TOKEN_FILE_NAME: &str = "docling-service.token";
const START_LOCK_FILE_NAME: &str = "docling-start.lock";
#[derive(Debug, Default)]
struct RuntimeState {
    service: Option<RunningDoclingService>,
    state: String,
    last_error: Option<String>,
    active_client_count: Option<u32>,
    active_request_count: Option<u32>,
    manually_disconnected: bool,
    generation: u64,
    last_instance_id: Option<String>,
}

struct LeaseSession {
    generation: u64,
    stop: watch::Sender<bool>,
    task: JoinHandle<()>,
}

#[derive(Clone)]
pub(crate) struct DoclingServiceManager {
    inner: Arc<DoclingServiceManagerInner>,
}

struct DoclingServiceManagerInner {
    state: Mutex<RuntimeState>,
    session: Mutex<Option<LeaseSession>>,
    connect_lock: AsyncMutex<()>,
    service_directory: PathBuf,
    registry_path: PathBuf,
    token_path: PathBuf,
    start_lock_path: PathBuf,
    daemon_script_path: PathBuf,
    client_id: String,
    client: Client,
    pub(super) models: super::models::ModelResources,
    selected_models: Mutex<Option<(research_docling_runtime::ModelManifest, PathBuf)>>,
    startup_attempted: AtomicBool,
}

impl DoclingServiceManager {
    pub(crate) fn from_app(app: &AppHandle) -> Result<Self, DoclingError> {
        let service_directory = app
            .path()
            .home_dir()?
            .join(".research-assistant")
            .join("services")
            .join("docling-rs-v2");
        create_private_directory(&service_directory)?;
        let daemon_script_path = app.path().resolve(
            if cfg!(windows) {
                "docling/research-docling-daemon.exe"
            } else {
                "docling/research-docling-daemon"
            },
            BaseDirectory::Resource,
        )?;
        Ok(Self::from_paths(service_directory, daemon_script_path))
    }

    pub(super) fn from_paths(service_directory: PathBuf, daemon_script_path: PathBuf) -> Self {
        Self {
            inner: Arc::new(DoclingServiceManagerInner {
                registry_path: service_directory.join(REGISTRY_FILE_NAME),
                token_path: service_directory.join(TOKEN_FILE_NAME),
                start_lock_path: service_directory.join(START_LOCK_FILE_NAME),
                service_directory: service_directory.clone(),
                daemon_script_path: daemon_script_path.clone(),
                state: Mutex::new(RuntimeState {
                    state: "disconnected".to_owned(),
                    ..RuntimeState::default()
                }),
                session: Mutex::new(None),
                connect_lock: AsyncMutex::new(()),
                client_id: Uuid::new_v4().to_string(),
                client: Client::new(),
                models: super::models::ModelResources::native(
                    service_directory.join("models"),
                    daemon_script_path,
                ),
                selected_models: Mutex::new(None),
                startup_attempted: AtomicBool::new(false),
            }),
        }
    }

    pub(crate) fn status(&self) -> DoclingServiceStatus {
        let Ok(state) = self.inner.state.lock() else {
            return DoclingServiceStatus::unavailable("Docling 服务状态锁已损坏");
        };
        let service = state.service.as_ref();
        DoclingServiceStatus {
            state: state.state.clone(),
            base_url: service.map(|value| value.base_url.clone()),
            port: service.map(|value| value.port),
            pid: service.map(|value| value.pid),
            instance_id: service.map(|value| value.instance_id.clone()),
            last_error: state.last_error.clone(),
            registry_path: Some(
                self.discovery_path(&self.inner.registry_path)
                    .display()
                    .to_string(),
            ),
            active_client_count: state.active_client_count,
            active_request_count: state.active_request_count,
        }
    }

    pub(crate) async fn refreshed_status(&self) -> DoclingServiceStatus {
        if let Some(service) = self.current_connected()
            && let Ok(status) = self
                .fetch_status_at(&service.base_url, &service.token)
                .await
            && status.schema_version == SCHEMA_VERSION
            && status.protocol_version == PROTOCOL_VERSION
            && status.service_instance_id == service.instance_id
            && status.state == "ready"
        {
            self.update_instance_counts(
                &service.instance_id,
                status.active_client_count,
                status.active_request_count,
            );
        }
        self.status()
    }

    pub(crate) async fn ping(&self) -> Result<DoclingPingResult, DoclingError> {
        let service = self.current_connected().ok_or(DoclingError::Disconnected)?;
        let started_at = std::time::Instant::now();
        let status = self
            .fetch_status_at(&service.base_url, &service.token)
            .await?;
        if status.schema_version != SCHEMA_VERSION || status.protocol_version != PROTOCOL_VERSION {
            return Err(DoclingError::Incompatible(
                "Docling daemon Ping 返回了不兼容的协议版本".to_owned(),
            ));
        }
        if status.service_instance_id != service.instance_id || status.state != "ready" {
            return Err(DoclingError::Unresponsive(
                "Docling daemon Ping 返回了不匹配的实例或状态".to_owned(),
            ));
        }
        self.update_instance_counts(
            &service.instance_id,
            status.active_client_count,
            status.active_request_count,
        );
        Ok(DoclingPingResult {
            status: self.status(),
            latency_ms: u64::try_from(started_at.elapsed().as_millis()).unwrap_or(u64::MAX),
        })
    }

    pub(crate) async fn require_connected(
        &self,
        settings: &DoclingSettings,
    ) -> Result<RunningDoclingService, DoclingError> {
        if !settings.enabled {
            return Err(DoclingError::Disabled);
        }
        // Wait for an already-authorized startup/connect, without starting one.
        let _connect = self.inner.connect_lock.lock().await;
        self.current_connected()
            .ok_or(DoclingError::ManualStartRequired)
    }

    pub(crate) async fn disconnect(&self) -> Result<bool, DoclingError> {
        let session = self
            .inner
            .session
            .lock()
            .ok()
            .and_then(|mut value| value.take());
        {
            let mut state = self
                .inner
                .state
                .lock()
                .map_err(|_| DoclingError::Request("Docling 服务状态锁已损坏".to_owned()))?;
            state.manually_disconnected = true;
            state.service = None;
            state.state = "disconnected".to_owned();
            state.last_error = None;
            state.active_client_count = None;
            state.active_request_count = None;
        }
        let Some(session) = session else {
            return Ok(false);
        };
        let _ = session.stop.send(true);
        if tokio::time::timeout(Duration::from_secs(2), session.task)
            .await
            .is_err()
        {
            // Dropping the task result closes the socket without owning the daemon.
        }
        Ok(true)
    }

    async fn connect(
        &self,
        settings: &DoclingSettings,
        explicit: bool,
    ) -> Result<RunningDoclingService, DoclingError> {
        if !settings.enabled {
            return Err(DoclingError::Disabled);
        }
        if explicit && let Ok(mut state) = self.inner.state.lock() {
            state.manually_disconnected = false;
        }
        if !explicit
            && self
                .inner
                .state
                .lock()
                .is_ok_and(|state| state.manually_disconnected)
        {
            return Err(DoclingError::Disconnected);
        }
        if let Some(service) = self.current_connected() {
            return Ok(service);
        }
        let _connect_guard = self.inner.connect_lock.lock().await;
        if let Some(service) = self.current_connected() {
            return Ok(service);
        }
        let selected = self.inner.models.current().inspect_err(|error| {
            self.set_state("models_unavailable", Some(error.to_string()));
        })?;
        *self
            .inner
            .selected_models
            .lock()
            .map_err(|_| DoclingError::Disconnected)? = Some(selected);
        if let Ok(mut session) = self.inner.session.lock()
            && let Some(old) = session.take()
        {
            let _ = old.stop.send(true);
        }
        self.set_state("discovering", None);
        if let Some(service) = self.discover_and_connect(settings, explicit).await? {
            return Ok(service);
        }

        if !explicit {
            self.set_state(
                "disconnected",
                Some(DoclingError::ManualStartRequired.to_string()),
            );
            return Err(DoclingError::ManualStartRequired);
        }
        let lock_path = self.discovery_path(&self.inner.start_lock_path);
        if let Some(parent) = lock_path.parent() {
            create_private_directory(parent)?;
        }
        let start_lock =
            acquire_start_lock(self.discovery_path(&self.inner.start_lock_path)).await?;
        if let Some(service) = self.discover_and_connect(settings, true).await? {
            drop(start_lock);
            return Ok(service);
        }
        self.set_state("starting", None);
        let result = self.start_daemon_and_connect(settings).await;
        drop(start_lock);
        result
    }

    pub(super) async fn discover_and_connect(
        &self,
        settings: &DoclingSettings,
        explicit: bool,
    ) -> Result<Option<RunningDoclingService>, DoclingError> {
        let record = match read_registry(&self.discovery_path(&self.inner.registry_path)) {
            Ok(Some(record)) => record,
            Ok(None) => return Ok(None),
            Err(error) => {
                if recorded_pid_is_alive(&self.discovery_path(&self.inner.registry_path)) {
                    self.set_state("incompatible", Some(error.to_string()));
                    return Err(error);
                }
                self.clear_stale_files();
                return Ok(None);
            }
        };
        // Check the very record used below, rather than reading it twice across
        // a possible replacement. The lease handshake also validates this UUID.
        if !explicit && !self.matches_previous_instance(&record.service_instance_id) {
            let error = DoclingError::ManualStartRequired;
            self.set_state("disconnected", Some(error.to_string()));
            return Err(error);
        }
        if let Err(error) = validate_registry(&record) {
            if process_is_alive(record.pid) {
                self.set_state("incompatible", Some(error.to_string()));
                return Err(error);
            }
            self.clear_stale_files();
            return Ok(None);
        }
        let token = match fs::read_to_string(self.discovery_path(&self.inner.token_path)) {
            Ok(token) if !token.trim().is_empty() => token.trim().to_owned(),
            _ if process_is_alive(record.pid) => {
                let error = DoclingError::Unresponsive("Docling daemon token 文件缺失".to_owned());
                self.set_state("unresponsive", Some(error.to_string()));
                return Err(error);
            }
            _ => {
                self.clear_stale_files();
                return Ok(None);
            }
        };
        let daemon_status = match self.fetch_status(&record, &token).await {
            Ok(status) => status,
            Err(error) if process_is_alive(record.pid) => {
                let error = DoclingError::Unresponsive(error.to_string());
                self.set_state("unresponsive", Some(error.to_string()));
                return Err(error);
            }
            Err(_) => {
                self.clear_stale_files();
                return Ok(None);
            }
        };
        if let Err(error) = validate_status(&record, &daemon_status).and_then(|_| {
            if let Some(expected) = self.selected_model_identity() {
                validate_model_identity(&record, &daemon_status, &expected)
            } else {
                Ok(())
            }
        }) {
            let state = if matches!(error, DoclingError::Incompatible(_)) {
                "incompatible"
            } else {
                "unresponsive"
            };
            self.set_state(state, Some(error.to_string()));
            return Err(error);
        }
        let service = match self.open_lease(record, token, settings.clone()).await {
            Ok(service) => service,
            Err(error) => {
                self.set_state("unresponsive", Some(error.to_string()));
                return Err(error);
            }
        };
        Ok(Some(service))
    }

    async fn fetch_status(
        &self,
        record: &RegistryRecord,
        token: &str,
    ) -> Result<DaemonStatus, DoclingError> {
        self.fetch_status_at(&record.http_base_url, token).await
    }

    async fn fetch_status_at(
        &self,
        base_url: &str,
        token: &str,
    ) -> Result<DaemonStatus, DoclingError> {
        let response = self
            .inner
            .client
            .get(format!("{base_url}{STATUS_PATH}"))
            .timeout(Duration::from_secs(2))
            .header("X-Api-Key", token)
            .send()
            .await?;
        if !response.status().is_success() {
            return Err(DoclingError::Request(format!(
                "Docling daemon status HTTP {}",
                response.status().as_u16()
            )));
        }
        response
            .json()
            .await
            .map_err(|error| DoclingError::InvalidResponse(error.to_string()))
    }

    async fn open_lease(
        &self,
        record: RegistryRecord,
        token: String,
        settings: DoclingSettings,
    ) -> Result<RunningDoclingService, DoclingError> {
        let (socket, granted) = open_lease(
            &record.websocket_url,
            &token,
            &self.inner.client_id,
            &record.service_instance_id,
        )
        .await?;
        let port = reqwest::Url::parse(&record.http_base_url)
            .ok()
            .and_then(|url| url.port())
            .ok_or_else(|| DoclingError::InvalidRegistry("Docling base URL 缺少端口".to_owned()))?;
        let service = RunningDoclingService {
            base_url: record.http_base_url,
            token,
            port,
            instance_id: record.service_instance_id,
            pid: record.pid,
            lease_id: granted.lease_id,
            model_identity: self.selected_model_identity(),
        };
        let generation = {
            let mut state = self
                .inner
                .state
                .lock()
                .map_err(|_| DoclingError::Request("Docling 服务状态锁已损坏".to_owned()))?;
            state.generation += 1;
            state.state = "connected".to_owned();
            state.last_instance_id = Some(service.instance_id.clone());
            state.service = Some(service.clone());
            state.last_error = None;
            state.active_client_count = Some(granted.active_client_count.max(1));
            state.active_request_count = granted.active_request_count;
            state.generation
        };
        let (stop, stop_rx) = watch::channel(false);
        let manager = self.clone();
        let count_manager = self.clone();
        let task_service = service.clone();
        let task = tokio::spawn(async move {
            let failure = run_lease_loop(
                socket,
                &task_service.lease_id,
                granted.heartbeat_interval_ms,
                granted.lease_timeout_ms,
                stop_rx,
                move |clients, requests| {
                    count_manager.update_counts(generation, clients, requests);
                },
            )
            .await;
            manager.lease_finished(generation, failure, settings);
        });
        if let Ok(mut session) = self.inner.session.lock() {
            *session = Some(LeaseSession {
                generation,
                stop,
                task,
            });
        }
        Ok(service)
    }

    async fn start_daemon_and_connect(
        &self,
        settings: &DoclingSettings,
    ) -> Result<RunningDoclingService, DoclingError> {
        let (_, models) = self
            .inner
            .selected_models
            .lock()
            .map_err(|_| DoclingError::Disconnected)?
            .clone()
            .ok_or(DoclingError::Disconnected)?;
        let registry = self.discovery_path(&self.inner.registry_path);
        let token = self.discovery_path(&self.inner.token_path);
        let directory = registry.parent().ok_or(DoclingError::Disconnected)?;
        create_private_directory(directory)?;
        let mut child = spawn_daemon(
            settings,
            &self.inner.daemon_script_path,
            &registry,
            &token,
            directory,
            &models,
            &self.inner.service_directory,
        )?;
        let started_at = tokio::time::Instant::now();
        loop {
            if let Some(status) = child.try_wait()? {
                let error = DoclingError::Start(format!(
                    "Rust Docling 启动后退出（{status}）；请检查 FP32 模型资源与原生解析程序"
                ));
                self.set_state("disconnected", Some(error.to_string()));
                return Err(error);
            }
            match self.discover_and_connect(settings, true).await {
                Ok(Some(service)) => return Ok(service),
                Ok(None) | Err(DoclingError::Unresponsive(_)) => {}
                Err(error) => {
                    cleanup_failed_start(&mut child).await;
                    return Err(error);
                }
            }
            if started_at.elapsed() >= STARTUP_TIMEOUT {
                cleanup_failed_start(&mut child).await;
                self.set_state(
                    "disconnected",
                    Some(DoclingError::StartupTimeout.to_string()),
                );
                return Err(DoclingError::StartupTimeout);
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }

    pub(crate) fn model_resources(&self) -> &super::models::ModelResources {
        &self.inner.models
    }

    pub(crate) fn model_identity(&self) -> Result<String, DoclingError> {
        self.inner
            .models
            .current()?
            .0
            .execution_identity()
            .map_err(DoclingError::Models)
    }
}

impl Drop for DoclingServiceManagerInner {
    fn drop(&mut self) {
        if let Ok(mut session) = self.session.lock()
            && let Some(session) = session.take()
        {
            let _ = session.stop.send(true);
            session.task.abort();
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct RunningDoclingService {
    pub(crate) base_url: String,
    token: String,
    pub(crate) port: u16,
    pub(crate) instance_id: String,
    pub(crate) pid: u32,
    lease_id: String,
    model_identity: Option<String>,
}

#[cfg(test)]
pub(crate) use conversion::build_convert_pdf_payload;
