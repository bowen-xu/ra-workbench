use std::{
    collections::HashMap,
    path::PathBuf,
    process::Stdio,
    sync::{
        Arc, Mutex as StdMutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use serde_json::{Value, json};
use tauri::{AppHandle, Manager, path::BaseDirectory};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, Command},
    sync::{Mutex, broadcast, oneshot, watch},
};
use uuid::Uuid;

use super::{
    error::AiError,
    model::{ChatGptAccountStatus, ChatGptLoginStart},
};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
type Pending = HashMap<u64, oneshot::Sender<Result<Value, AiError>>>;

#[path = "codex_app_server/pending_request.rs"]
mod pending_request;
#[path = "codex_app_server/process_reader.rs"]
mod process_reader;

#[derive(Clone)]
pub(crate) struct CodexAppServerManager {
    codex_path: PathBuf,
    codex_home: PathBuf,
    work_root: PathBuf,
    next_id: Arc<AtomicU64>,
    pending: Arc<StdMutex<Pending>>,
    notifications: broadcast::Sender<Value>,
    process: Arc<Mutex<ProcessState>>,
    start_lock: Arc<Mutex<()>>,
    pub(crate) auth_lock: Arc<Mutex<()>>,
    pub(crate) auth_epoch: watch::Sender<Uuid>,
}

#[derive(Default)]
struct ProcessState {
    generation: Uuid,
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    initialized: bool,
}

impl CodexAppServerManager {
    pub(crate) fn from_app(app: &AppHandle) -> Result<Self, AiError> {
        let resource = app
            .path()
            .resolve("codex/codex", BaseDirectory::Resource)
            .map_err(|_| AiError::TemporaryUnavailable)?;
        let development = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".generated/codex/codex");
        let codex_path = if resource.is_file() {
            resource
        } else {
            development
        };
        let data = app
            .path()
            .app_data_dir()
            .map_err(|_| AiError::TemporaryUnavailable)?;
        let (notifications, _) = broadcast::channel(1024);
        Ok(Self {
            codex_path,
            codex_home: data.join("codex"),
            work_root: data.join("cache/codex-research-work"),
            next_id: Arc::new(AtomicU64::new(1)),
            pending: Arc::new(StdMutex::new(HashMap::new())),
            notifications,
            process: Arc::new(Mutex::new(ProcessState::default())),
            start_lock: Arc::new(Mutex::new(())),
            auth_lock: Arc::new(Mutex::new(())),
            auth_epoch: watch::channel(Uuid::new_v4()).0,
        })
    }

    async fn ensure_started(&self) -> Result<(), AiError> {
        let _start = self.start_lock.lock().await;
        if self.process.lock().await.initialized {
            return Ok(());
        }
        if !self.codex_path.is_file() {
            return Err(AiError::InvalidInput(
                "未找到应用内置 Codex 运行时，请重新安装或更新应用".to_owned(),
            ));
        }
        create_private_directory(&self.codex_home)?;
        create_private_directory(&self.work_root)?;
        let mut child = Command::new(&self.codex_path)
            .args([
                "app-server",
                "--stdio",
                "-c",
                "cli_auth_credentials_store=\"file\"",
            ])
            .env("CODEX_HOME", &self.codex_home)
            // The application's managed login must not adopt the launching
            // terminal's Codex/API credentials. Provider keys use separate adapters.
            .env_remove("CODEX_API_KEY")
            .env_remove("OPENAI_API_KEY")
            .env_remove("CODEX_ACCESS_TOKEN")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|_| AiError::TemporaryUnavailable)?;
        let stdin = child.stdin.take().ok_or(AiError::TemporaryUnavailable)?;
        let stdout = child.stdout.take().ok_or(AiError::TemporaryUnavailable)?;
        let stderr = child.stderr.take().ok_or(AiError::TemporaryUnavailable)?;
        let generation = Uuid::new_v4();
        {
            let mut process = self.process.lock().await;
            self.fail_pending();
            process.generation = generation;
            process.child = Some(child);
            process.stdin = Some(stdin);
        }
        self.spawn_reader(stdout, generation);
        tauri::async_runtime::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while matches!(lines.next_line().await, Ok(Some(_))) {}
        });
        let initialized = self
            .request_started(
                "initialize",
                json!({
                    "clientInfo": {
                        "name": "research_assistant",
                        "title": "Research Assistant",
                        "version": env!("CARGO_PKG_VERSION")
                    },
                    "capabilities": { "experimentalApi": true }
                }),
                REQUEST_TIMEOUT,
            )
            .await;
        if initialized.is_err() {
            self.stop().await;
            return initialized.map(|_| ());
        }
        self.notify_started("initialized", json!({})).await?;
        let mut process = self.process.lock().await;
        if process.generation != generation || process.stdin.is_none() {
            return Err(AiError::TemporaryUnavailable);
        }
        process.initialized = true;
        Ok(())
    }

    /// Send an app-server RPC for the authentication bridge only.
    ///
    /// This deliberately remains private to this module: Agent text/tool
    /// generation must use the shared model resolver and transport rather than
    /// accidentally growing a second Codex-owned execution path.
    async fn request(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, AiError> {
        if !auth_rpc_method_allowed(method) {
            return Err(AiError::InvalidInput(
                "Codex app-server 仅允许认证桥接 RPC".to_owned(),
            ));
        }
        self.ensure_started().await?;
        self.request_started(method, params, timeout).await
    }

    /// Return the managed credential payload for the native authentication
    /// adapter. This is the only non-UI caller that may request a token.
    pub(crate) async fn auth_status(&self) -> Result<Value, AiError> {
        self.request(
            "getAuthStatus",
            json!({"includeToken": true, "refreshToken": true}),
            REQUEST_TIMEOUT,
        )
        .await
    }

    async fn request_started(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, AiError> {
        let id = self.next_id.fetch_add(1, Ordering::AcqRel);
        let (sender, receiver) = oneshot::channel();
        let _pending = pending_request::PendingRequest::register(&self.pending, id, sender)?;
        self.write(json!({ "id": id, "method": method, "params": params }))
            .await?;
        match tokio::time::timeout(timeout, receiver).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(AiError::TemporaryUnavailable),
            Err(_) => Err(AiError::Timeout),
        }
    }

    async fn notify_started(&self, method: &str, params: Value) -> Result<(), AiError> {
        self.write(json!({ "method": method, "params": params }))
            .await
    }

    async fn write(&self, message: Value) -> Result<(), AiError> {
        let bytes = serde_json::to_vec(&message).map_err(|_| AiError::TemporaryUnavailable)?;
        let mut process = self.process.lock().await;
        let stdin = process
            .stdin
            .as_mut()
            .ok_or(AiError::TemporaryUnavailable)?;
        stdin
            .write_all(&bytes)
            .await
            .map_err(|_| AiError::TemporaryUnavailable)?;
        stdin
            .write_all(b"\n")
            .await
            .map_err(|_| AiError::TemporaryUnavailable)?;
        stdin
            .flush()
            .await
            .map_err(|_| AiError::TemporaryUnavailable)
    }

    pub(crate) async fn account_status(&self) -> Result<ChatGptAccountStatus, AiError> {
        // An empty managed profile can reject `account/read` before login.
        // That is an account state, not a missing runtime: keep the bridge
        // usable so the UI can offer login. Only fail closed when both the
        // account probe and the auth-status probe cannot reach the process.
        self.ensure_started().await?;
        let account_result = self
            .request_started(
                "account/read",
                json!({ "refreshToken": false }),
                REQUEST_TIMEOUT,
            )
            .await;
        // `account/read` can still report the remembered ChatGPT account when
        // its managed access token has expired or disappeared. Check the same
        // token-bearing auth status used by the Agent lease, but only keep the
        // boolean here; credentials never cross the UI boundary.
        let credential_status = self
            .request_started(
                "getAuthStatus",
                json!({ "includeToken": true, "refreshToken": true }),
                REQUEST_TIMEOUT,
            )
            .await
            .ok();
        account_status_from_probes(account_result, credential_status)
    }

    pub(crate) async fn start_login(&self) -> Result<ChatGptLoginStart, AiError> {
        self.auth_epoch.send_replace(Uuid::new_v4());
        let _auth = self.auth_lock.lock().await;
        self.auth_epoch.send_replace(Uuid::new_v4());
        let result = self
            .request(
                "account/login/start",
                json!({
                    "type": "chatgpt",
                    "useHostedLoginSuccessPage": true,
                    "appBrand": "chatgpt"
                }),
                REQUEST_TIMEOUT,
            )
            .await?;
        let login_id = result
            .get("loginId")
            .and_then(Value::as_str)
            .ok_or(AiError::Provider("invalid_response"))?;
        let auth_url = result
            .get("authUrl")
            .and_then(Value::as_str)
            .filter(|value| auth_url_allowed(value))
            .ok_or(AiError::Provider("invalid_response"))?;
        Ok(ChatGptLoginStart {
            login_id: login_id.to_owned(),
            auth_url: auth_url.to_owned(),
        })
    }

    pub(crate) async fn cancel_login(&self, login_id: &str) -> Result<(), AiError> {
        if login_id.trim().is_empty() {
            return Ok(());
        }
        self.request(
            "account/login/cancel",
            json!({ "loginId": login_id }),
            REQUEST_TIMEOUT,
        )
        .await
        .map(|_| ())
    }

    pub(crate) async fn logout(&self) -> Result<(), AiError> {
        self.auth_epoch.send_replace(Uuid::new_v4());
        let _auth = self.auth_lock.lock().await;
        self.auth_epoch.send_replace(Uuid::new_v4());
        self.request("account/logout", json!({}), REQUEST_TIMEOUT)
            .await
            .map(|_| ())
    }

    pub(crate) async fn stop(&self) {
        let mut state = self.process.lock().await;
        state.generation = Uuid::new_v4();
        self.auth_epoch.send_replace(Uuid::new_v4());
        self.fail_pending();
        state.initialized = false;
        state.stdin = None;
        if let Some(mut child) = state.child.take() {
            let _ = child.kill().await;
        }
    }
}

/// Keep the account-status UI aligned with the credential source used by the
/// Agent. This deliberately inspects only shape/presence, never token bytes.
fn has_managed_credential(value: &Value) -> bool {
    value.get("authMethod").and_then(Value::as_str) == Some("chatgpt")
        && value.get("requiresOpenaiAuth").and_then(Value::as_bool) == Some(true)
        && value
            .get("authToken")
            .and_then(Value::as_str)
            .is_some_and(|token| !token.is_empty())
}

fn auth_url_allowed(value: &str) -> bool {
    reqwest::Url::parse(value).is_ok_and(|url| {
        let host = url.host_str().unwrap_or_default();
        url.scheme() == "https"
            && (host == "openai.com"
                || host.ends_with(".openai.com")
                || host == "chatgpt.com"
                || host.ends_with(".chatgpt.com"))
            && url.username().is_empty()
            && url.password().is_none()
    })
}

fn account_status_from_probes(
    account_result: Result<Value, AiError>,
    credential_status: Option<Value>,
) -> Result<ChatGptAccountStatus, AiError> {
    let result = match (account_result, credential_status.as_ref()) {
        (Ok(result), _) => result,
        (Err(_error), Some(_)) => {
            // The runtime answered the auth probe, so preserve the
            // not-yet-logged-in state instead of reporting a missing
            // bundled executable.
            Value::Null
        }
        (Err(error), None) => return Err(error),
    };
    let account = result.get("account");
    let auth_type = account
        .and_then(|value| value.get("type"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    Ok(ChatGptAccountStatus {
        runtime_available: true,
        configured: auth_type == "chatgpt"
            && credential_status
                .as_ref()
                .is_some_and(has_managed_credential),
        auth_type: auth_type.to_owned(),
        email: account
            .and_then(|value| value.get("email"))
            .and_then(Value::as_str)
            .map(str::to_owned),
        plan_type: account
            .and_then(|value| value.get("planType"))
            .and_then(Value::as_str)
            .map(str::to_owned),
    })
}

/// The bundled app-server is an authentication bridge, not the application's
/// Agent owner. Keep the method boundary explicit so a future caller cannot
/// accidentally route model turns, tool calls, or conversation state through
/// the Codex process.
fn auth_rpc_method_allowed(method: &str) -> bool {
    matches!(
        method,
        "initialize"
            | "account/read"
            | "account/login/start"
            | "account/login/cancel"
            | "account/logout"
            | "getAuthStatus"
    )
}

fn create_private_directory(path: &PathBuf) -> Result<(), AiError> {
    std::fs::create_dir_all(path).map_err(AiError::CredentialIo)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
            .map_err(AiError::CredentialIo)?;
    }
    Ok(())
}

#[cfg(all(test, target_os = "macos"))]
#[path = "codex_app_server/authenticated_smoke.rs"]
mod authenticated_smoke;
#[cfg(test)]
#[path = "codex_app_server/process_smoke.rs"]
mod process_smoke;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_url_requires_an_openai_https_host() {
        assert!(auth_url_allowed(
            "https://auth.openai.com/authorize?opaque=1"
        ));
        assert!(auth_url_allowed("https://chatgpt.com/auth"));
        assert!(!auth_url_allowed("http://openai.com/auth"));
        assert!(!auth_url_allowed("https://openai.com.evil.test/auth"));
        assert!(!auth_url_allowed("https://user:pass@openai.com/auth"));
    }

    #[test]
    fn app_server_rpc_boundary_only_allows_authentication_methods() {
        for method in [
            "initialize",
            "account/read",
            "account/login/start",
            "account/login/cancel",
            "account/logout",
            "getAuthStatus",
        ] {
            assert!(auth_rpc_method_allowed(method), "{method}");
        }
        for method in [
            "turn/start",
            "turn/steer",
            "thread/start",
            "model/list",
            "mcp/call",
        ] {
            assert!(!auth_rpc_method_allowed(method), "{method}");
        }
    }

    #[test]
    fn account_status_requires_a_managed_token_not_only_a_remembered_account() {
        assert!(has_managed_credential(&json!({
            "authMethod": "chatgpt",
            "requiresOpenaiAuth": true,
            "authToken": "opaque-token"
        })));
        assert!(!has_managed_credential(&json!({
            "authMethod": "chatgpt",
            "requiresOpenaiAuth": true,
            "authToken": null
        })));
        assert!(!has_managed_credential(&json!({
            "authMethod": "api-key",
            "requiresOpenaiAuth": true,
            "authToken": "opaque-token"
        })));
    }

    #[test]
    fn empty_account_probe_keeps_runtime_available_when_auth_bridge_responds() {
        let status = account_status_from_probes(
            Err(AiError::Provider("authentication")),
            Some(json!({
                "authMethod": null,
                "authToken": null,
                "requiresOpenaiAuth": true
            })),
        )
        .unwrap();
        assert!(status.runtime_available);
        assert!(!status.configured);
        assert!(status.email.is_none());
    }

    #[test]
    fn account_status_fails_when_both_runtime_probes_are_unavailable() {
        assert!(account_status_from_probes(Err(AiError::TemporaryUnavailable), None,).is_err());
    }
}
