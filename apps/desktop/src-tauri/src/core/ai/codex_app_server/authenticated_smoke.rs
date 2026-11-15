//! Opt-in real transport acceptance. The development application must be closed.
//! Only the app-owned development login is used; no credential files are read here.
use super::*;
use crate::agent::{
    authentication::AgentChatGptAuthentication,
    model::responses_transport::{
        ResponsesReceipt, ResponsesReceiptSink, ResponsesTransport, ResponsesTransportConfig,
    },
};
use research_agent_core::{
    model::{
        ContentPart, FinishReason, ModelCapabilities, ModelError, ModelFuture, ModelMessage,
        ModelRequest, ModelStreamEvent, ModelTransport,
    },
    protocol::ModelIdentity,
};

#[derive(Default)]
struct ReceiptCounter(AtomicU64);
impl ResponsesReceiptSink for ReceiptCounter {
    fn prepare_input(
        &self,
        _: Uuid,
        _: ModelIdentity,
        input: Vec<Value>,
    ) -> ModelFuture<'_, Vec<Value>> {
        Box::pin(async move { Ok(input) })
    }

    fn preserve(&self, _: ResponsesReceipt) -> ModelFuture<'_, ()> {
        Box::pin(async move {
            // One-shot smoke: intentionally retain no provider content or account identity.
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
    }
}

static PROFILE_OWNER: Mutex<()> = Mutex::const_new(());

fn development_bridge() -> (CodexAppServerManager, tempfile::TempDir) {
    assert!(
        std::env::var("RESEARCH_AGENT_CHATGPT_SMOKE").as_deref() == Ok("1"),
        "set RESEARCH_AGENT_CHATGPT_SMOKE=1 to authorize this synthetic model call"
    );
    let stopped = std::process::Command::new("/bin/ps")
        .args(["-axo", "pid=,command="])
        .output()
        .ok()
        .map(|output| {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let matching: Vec<_> = stdout
                .lines()
                .filter(|line| {
                    line.contains("/Research Assistant Dev.app/Contents/MacOS/research-assistant")
                })
                .collect();
            matching.is_empty()
        })
        .unwrap_or(false);
    assert!(
        stopped,
        "close the dev app; process inspection must succeed"
    );
    let home = PathBuf::from(std::env::var_os("HOME").expect("HOME is required"))
        .join("Library/Application Support/com.bowenxu.research-assistant.dev/codex");
    assert!(
        home.is_dir(),
        "log in using the development application first"
    );
    let work = tempfile::tempdir().unwrap();
    let (notifications, _) = broadcast::channel(1024);
    let bridge = CodexAppServerManager {
        codex_path: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".generated/codex/codex"),
        codex_home: home,
        work_root: work.path().join("work"),
        next_id: Arc::new(AtomicU64::new(1)),
        pending: Arc::new(StdMutex::new(HashMap::new())),
        notifications,
        process: Arc::new(Mutex::new(ProcessState::default())),
        start_lock: Arc::new(Mutex::new(())),
        auth_lock: Arc::new(Mutex::new(())),
        auth_epoch: watch::channel(Uuid::new_v4()).0,
    };
    (bridge, work)
}

#[tokio::test]
#[ignore = "real model call; requires explicit opt-in, app-owned login, and closed dev application"]
async fn authenticated_chatgpt_direct_text_smoke() {
    let _owner = PROFILE_OWNER.lock().await;
    let (bridge, _work) = development_bridge();
    let receipts = Arc::new(ReceiptCounter::default());
    // Always stop the bridge before asserting the result, including timeout/failure.
    let outcome = tokio::time::timeout(
        Duration::from_secs(90),
        synthetic_request(
            bridge.clone(),
            receipts.clone(),
            vec![ContentPart::Text(
                "This is a synthetic connectivity test. Reply with only OK.".into(),
            )],
        ),
    )
    .await;
    bridge.stop().await;
    let result = outcome.expect("synthetic transport acceptance timed out");
    // ModelError is a fixed safe enum, never an HTTP body or authentication response.
    result.expect("synthetic direct transport failed");
    assert_eq!(receipts.0.load(Ordering::SeqCst), 1);
    assert_eq!(bridge.next_id.load(Ordering::Acquire), 3);
}

#[tokio::test]
#[ignore = "real vision model call; requires explicit opt-in, app-owned login, and closed dev application"]
async fn authenticated_chatgpt_direct_vision_smoke() {
    let _owner = PROFILE_OWNER.lock().await;
    let (bridge, _work) = development_bridge();
    let receipts = Arc::new(ReceiptCounter::default());
    let outcome = tokio::time::timeout(
        Duration::from_secs(90),
        synthetic_request(bridge.clone(), receipts.clone(), synthetic_vision_content()),
    )
    .await;
    bridge.stop().await;
    let result = outcome.expect("synthetic vision transport acceptance timed out");
    result.expect("synthetic direct vision transport failed");
    assert_eq!(receipts.0.load(Ordering::SeqCst), 1);
    assert_eq!(bridge.next_id.load(Ordering::Acquire), 3);
}

#[tokio::test]
#[ignore = "real auth refresh; requires explicit opt-in, app-owned login, and closed dev application"]
async fn authenticated_chatgpt_auth_refresh_smoke() {
    let _owner = PROFILE_OWNER.lock().await;
    let (bridge, _work) = development_bridge();
    let auth = AgentChatGptAuthentication::new(bridge.clone());
    let first = tokio::time::timeout(Duration::from_secs(30), auth.acquire())
        .await
        .expect("first auth acquisition timed out")
        .expect("first auth acquisition failed");
    let first_scope = first.scope_id();
    drop(first);
    let second = tokio::time::timeout(Duration::from_secs(30), auth.acquire())
        .await
        .expect("refreshed auth acquisition timed out")
        .expect("refreshed auth acquisition failed");
    assert_eq!(second.scope_id(), first_scope);
    bridge.stop().await;
}

#[path = "authenticated_smoke/agent_execution_smoke.rs"]
mod agent_execution_smoke;
#[path = "authenticated_smoke/compaction.rs"]
mod compaction;
#[path = "authenticated_smoke/tool_smoke.rs"]
mod tool_smoke;

fn chatgpt_transport(
    bridge: CodexAppServerManager,
    receipts: Arc<dyn ResponsesReceiptSink>,
    capabilities: ModelCapabilities,
) -> Result<ResponsesTransport, ModelError> {
    ResponsesTransport::chatgpt(
        ResponsesTransportConfig {
            credential_scope_id: Uuid::new_v4(),
            provider_id: "openai".into(),
            timeout: Duration::from_secs(60),
            service_tier: None,
            capabilities,
        },
        Arc::new(AgentChatGptAuthentication::new(bridge)),
        receipts,
    )
}

async fn synthetic_request(
    bridge: CodexAppServerManager,
    receipts: Arc<ReceiptCounter>,
    content: Vec<ContentPart>,
) -> Result<(), ModelError> {
    let vision = content
        .iter()
        .any(|part| matches!(part, ContentPart::Image { .. }));
    let transport = chatgpt_transport(
        bridge,
        receipts,
        ModelCapabilities {
            vision,
            tools: false,
            parallel_tools: false,
            reasoning_efforts: vec![],
        },
    )?;
    let mut stream = transport
        .start(ModelRequest {
            captured_history: None,
            request_id: Uuid::new_v4(),
            model: ModelIdentity {
                provider_id: "openai".into(),
                model_id: "gpt-5.6-terra".into(),
                revision: None,
            },
            messages: vec![ModelMessage::User {
                content,
                source_item_id: None,
            }],
            tools: vec![],
            max_output_tokens: 32,
            parallel_tool_calls: false,
            reasoning_effort: None,
        })
        .await?;
    let mut started = false;
    let mut text = false;
    let mut completed = false;
    while let Some(event) = stream.next().await? {
        match event {
            ModelStreamEvent::Started { .. } => started = true,
            ModelStreamEvent::TextDelta(value)
            | ModelStreamEvent::AssistantTextDelta { text: value, .. } => {
                text |= !value.trim().is_empty()
            }
            ModelStreamEvent::AssistantMessageStarted { .. }
            | ModelStreamEvent::AssistantMessageCompleted { .. }
            | ModelStreamEvent::OutputOrder(_) => (),
            ModelStreamEvent::Completed {
                reason: FinishReason::Stop,
                ..
            } => completed = true,
            ModelStreamEvent::Usage(_) => (),
            _ => return Err(ModelError::InvalidStream),
        }
    }
    if started && text && completed {
        Ok(())
    } else {
        Err(ModelError::IncompleteStream)
    }
}

fn synthetic_vision_content() -> Vec<ContentPart> {
    // Valid 1x1 PNG migrated from the pinned Codex image transport fixtures.
    // The smoke checks our shared image transport and response lifecycle without
    // reading a user's paper, screenshot or object store.
    const PNG: &[u8] = &[
        137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 6,
        0, 0, 0, 31, 21, 196, 137, 0, 0, 0, 13, 73, 68, 65, 84, 120, 156, 99, 248, 207, 192, 240,
        31, 0, 5, 0, 1, 255, 137, 153, 61, 29, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
    ];
    vec![
        ContentPart::Text(
            "A synthetic PNG is attached for transport validation. Reply with only OK.".into(),
        ),
        ContentPart::Image {
            media_type: "image/png".into(),
            bytes: Arc::from(PNG),
        },
    ]
}
