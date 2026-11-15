//! Explicit native-process acceptance with an empty, disposable auth profile.
//! No user credential paths, login flow, model request, or thread RPC is used.
use super::*;
use crate::agent::authentication::{AgentChatGptAuthentication, AuthenticationError};

#[tokio::test]
#[ignore = "requires the prepared bundled Codex runtime; uses a disposable empty profile"]
async fn bundled_auth_bridge_accepts_protocol_and_requires_its_own_login() {
    let directory = tempfile::tempdir().unwrap();
    let (notifications, _) = broadcast::channel(1024);
    let bridge = CodexAppServerManager {
        codex_path: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".generated/codex/codex"),
        codex_home: directory.path().join("codex"),
        work_root: directory.path().join("work"),
        next_id: Arc::new(AtomicU64::new(1)),
        pending: Arc::new(StdMutex::new(HashMap::new())),
        notifications,
        process: Arc::new(Mutex::new(ProcessState::default())),
        start_lock: Arc::new(Mutex::new(())),
        auth_lock: Arc::new(Mutex::new(())),
        auth_epoch: watch::channel(Uuid::new_v4()).0,
    };
    assert!(
        bridge.codex_path.is_file(),
        "prepare the bundled runtime first"
    );
    let authentication = AgentChatGptAuthentication::new(bridge.clone());
    let result = authentication.acquire().await;
    let initialized = bridge.process.lock().await.initialized;
    let empty_account = if initialized {
        bridge
            .request(
                "getAuthStatus",
                json!({ "includeToken": false, "refreshToken": false }),
                REQUEST_TIMEOUT,
            )
            .await
            .is_ok_and(|status| {
                status.get("authMethod") == Some(&Value::Null)
                    && status.get("authToken") == Some(&Value::Null)
                    && status.get("requiresOpenaiAuth") == Some(&Value::Bool(true))
            })
    } else {
        false
    };
    let requests = bridge.next_id.load(Ordering::Acquire);
    bridge.stop().await;
    // Do not Debug-print a response/lease even when a protocol assertion fails.
    assert!(
        initialized,
        "bundled app-server initialization did not complete"
    );
    assert!(
        matches!(result, Err(AuthenticationError::Unsupported)),
        "empty application profile must require managed login"
    );
    assert_eq!(
        requests, 4,
        "only initialize and two getAuthStatus reads are expected"
    );
    assert!(
        empty_account,
        "ambient credentials must not become the application account"
    );
    {
        let process = bridge.process.lock().await;
        assert!(!process.initialized && process.child.is_none() && process.stdin.is_none());
    }
    // Reuse the manager immediately: delayed old-reader EOF must not stop this process.
    let restarted = authentication.acquire().await;
    let initialized = bridge.process.lock().await.initialized;
    bridge.stop().await;
    assert!(
        initialized,
        "replacement process must finish initialization"
    );
    assert!(matches!(restarted, Err(AuthenticationError::Unsupported)));
}
