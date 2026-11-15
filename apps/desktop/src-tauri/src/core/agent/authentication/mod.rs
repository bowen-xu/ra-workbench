//! Native-only, fixed-version ChatGPT authentication bridge.
//! No credential or provider response is exposed through the Agent IPC protocol.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use reqwest::{
    Method, Request,
    header::{AUTHORIZATION, HeaderValue},
};
use serde::Deserialize;
use serde_json::Value;
#[cfg(test)]
use serde_json::json;
use sha2::{Digest, Sha256};
use tokio::{sync::watch, time::Instant};
use uuid::Uuid;

use crate::ai::CodexAppServerManager;

const RESPONSE_URL: &str = "https://chatgpt.com/backend-api/codex/responses";
const COMPACT_URL: &str = "https://chatgpt.com/backend-api/codex/responses/compact";
const MAX_TOKEN_BYTES: usize = 32_768;

pub type CredentialFuture<'a> = std::pin::Pin<
    Box<dyn std::future::Future<Output = Result<ChatGptLease, AuthenticationError>> + Send + 'a>,
>;
/// Native credential capability; no raw-token access and no Agent execution.
pub trait ChatGptCredentialSource: Send + Sync {
    fn acquire(&self) -> CredentialFuture<'_>;
}
impl ChatGptCredentialSource for AgentChatGptAuthentication {
    fn acquire(&self) -> CredentialFuture<'_> {
        Box::pin(AgentChatGptAuthentication::acquire(self))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AuthenticationError {
    #[error("authentication bridge unavailable")]
    Unavailable,
    #[error("managed ChatGPT authentication is required")]
    Unsupported,
    #[error("authentication response is unusable")]
    InvalidCredential,
    #[error("authentication lease expired or was revoked")]
    Revoked,
    #[error("credential destination is not permitted")]
    Destination,
}

/// App-owned service; the existing bridge remains the sole refresh-token writer.
pub struct AgentChatGptAuthentication {
    bridge: CodexAppServerManager,
}

impl AgentChatGptAuthentication {
    pub(crate) fn new(bridge: CodexAppServerManager) -> Self {
        Self { bridge }
    }

    /// Serializes credential acquisition with login/logout. No thread/turn RPCs.
    /// A concurrent account notification rejects this acquisition; callers may
    /// acquire again, but must never replay an already admitted model request.
    pub async fn acquire(&self) -> Result<ChatGptLease, AuthenticationError> {
        let _operation = self.bridge.auth_lock.lock().await;
        let epoch = self.bridge.auth_epoch.subscribe();
        let identity = *epoch.borrow();
        let response = self
            .bridge
            .auth_status()
            .await
            .map_err(|_| AuthenticationError::Unavailable)?;
        let lease = ChatGptLease::from_response(response, epoch, identity, SystemTime::now())?;
        lease.check()?;
        Ok(lease)
    }
}

/// Intentionally neither Debug, Serialize nor Clone. Headers remain native.
/// The HTTP adapter must disable redirects and race the entire request/stream
/// against `revoked`; request admission alone cannot cancel an in-flight stream.
pub struct ChatGptLease {
    authorization: HeaderValue,
    account: HeaderValue,
    epoch: watch::Receiver<Uuid>,
    /// Stable, non-reversible account scope used to bind persisted provider
    /// continuation state across application restarts. This is deliberately
    /// separate from `identity`, which is the process-local revocation epoch.
    account_scope: Uuid,
    identity: Uuid,
    deadline: Instant,
    admission_deadline: Instant,
}

#[derive(Deserialize)]
struct AccessClaims {
    exp: u64,
    #[serde(rename = "https://api.openai.com/auth")]
    account: AccountClaims,
}

#[derive(Deserialize)]
struct AccountClaims {
    chatgpt_account_id: String,
    #[serde(default)]
    chatgpt_account_is_fedramp: bool,
}

impl ChatGptLease {
    /// Stable opaque account scope, not an account identifier/token.
    ///
    /// This value is deterministic for one validated ChatGPT account so a
    /// persisted Responses checkpoint can be recovered after an app restart.
    /// Live leases are still revoked by the process-local authentication epoch
    /// checked in `check`/`revoked` below.
    pub fn scope_id(&self) -> Uuid {
        self.account_scope
    }

    fn from_response(
        response: Value,
        epoch: watch::Receiver<Uuid>,
        identity: Uuid,
        now: SystemTime,
    ) -> Result<Self, AuthenticationError> {
        if response.get("authMethod").and_then(Value::as_str) != Some("chatgpt")
            || response.get("requiresOpenaiAuth").and_then(Value::as_bool) != Some(true)
        {
            return Err(AuthenticationError::Unsupported);
        }
        let token = response
            .get("authToken")
            .and_then(Value::as_str)
            .filter(|token| !token.is_empty() && token.len() <= MAX_TOKEN_BYTES)
            .ok_or(AuthenticationError::InvalidCredential)?;
        let parts = token.split('.').collect::<Vec<_>>();
        if parts.len() != 3 || parts.iter().any(|part| part.is_empty()) {
            return Err(AuthenticationError::InvalidCredential);
        }
        // Claims are routing/expiry hints from our trusted bridge, NOT signature
        // verification. The remote service authenticates the token. Unsupported
        // token shapes fail closed rather than guessing an account or endpoint.
        let payload = URL_SAFE_NO_PAD
            .decode(parts[1])
            .map_err(|_| AuthenticationError::InvalidCredential)?;
        let claims: AccessClaims =
            serde_json::from_slice(&payload).map_err(|_| AuthenticationError::InvalidCredential)?;
        if claims.account.chatgpt_account_is_fedramp {
            return Err(AuthenticationError::Unsupported);
        }
        let account = claims.account.chatgpt_account_id;
        if account.is_empty()
            || account.len() > 256
            || !account
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"-_".contains(&byte))
        {
            return Err(AuthenticationError::InvalidCredential);
        }
        let now = now
            .duration_since(UNIX_EPOCH)
            .map_err(|_| AuthenticationError::InvalidCredential)?
            .as_secs();
        let remaining = claims
            .exp
            .checked_sub(now.saturating_add(30))
            .filter(|seconds| *seconds > 0)
            .ok_or(AuthenticationError::Revoked)?;
        let mut authorization = HeaderValue::from_str(&format!("Bearer {token}"))
            .map_err(|_| AuthenticationError::InvalidCredential)?;
        authorization.set_sensitive(true);
        let mut account =
            HeaderValue::from_str(&account).map_err(|_| AuthenticationError::InvalidCredential)?;
        account.set_sensitive(true);
        let account_scope = stable_account_scope(
            account
                .to_str()
                .map_err(|_| AuthenticationError::InvalidCredential)?,
        );
        let monotonic_now = Instant::now();
        let deadline = monotonic_now
            .checked_add(Duration::from_secs(remaining))
            .ok_or(AuthenticationError::InvalidCredential)?;
        Ok(Self {
            authorization,
            account,
            epoch,
            account_scope,
            identity,
            deadline,
            admission_deadline: monotonic_now + Duration::from_secs(remaining.min(60)),
        })
    }

    pub(crate) fn check(&self) -> Result<(), AuthenticationError> {
        if self.epoch.has_changed().is_err()
            || *self.epoch.borrow() != self.identity
            || Instant::now() >= self.deadline
        {
            Err(AuthenticationError::Revoked)
        } else {
            Ok(())
        }
    }

    /// Only the exact Responses and compaction POST destinations may receive credentials.
    /// Failure leaves the request untouched. Do not log authenticated requests.
    pub fn authorize(&self, request: &mut Request) -> Result<(), AuthenticationError> {
        self.check()?;
        if Instant::now() >= self.admission_deadline {
            return Err(AuthenticationError::Revoked);
        }
        if request.method() != Method::POST
            || ![RESPONSE_URL, COMPACT_URL].contains(&request.url().as_str())
        {
            return Err(AuthenticationError::Destination);
        }
        request
            .headers_mut()
            .insert(AUTHORIZATION, self.authorization.clone());
        request
            .headers_mut()
            .insert("chatgpt-account-id", self.account.clone());
        Ok(())
    }

    /// Read-only model metadata at one fixed destination. No general GET capability.
    pub(crate) fn authorize_catalog(
        &self,
        request: &mut Request,
        client_version: &str,
    ) -> Result<(), AuthenticationError> {
        self.check()?;
        let mut expected = reqwest::Url::parse(crate::ai::codex_model_catalog::MODELS_URL)
            .map_err(|_| AuthenticationError::Destination)?;
        expected
            .query_pairs_mut()
            .append_pair("client_version", client_version);
        if Instant::now() >= self.admission_deadline {
            return Err(AuthenticationError::Revoked);
        }
        if !crate::ai::codex_model_catalog::valid_catalog_version(client_version)
            || request.method() != Method::GET
            || request.url() != &expected
        {
            return Err(AuthenticationError::Destination);
        }
        request
            .headers_mut()
            .insert(AUTHORIZATION, self.authorization.clone());
        request
            .headers_mut()
            .insert("chatgpt-account-id", self.account.clone());
        Ok(())
    }

    /// Resolves on account change, bridge exit, or lease expiry, including changes
    /// that happened before the caller started waiting. Dropping this wait is safe.
    pub async fn revoked(&self) {
        let mut epoch = self.epoch.clone();
        loop {
            if self.check().is_err() {
                return;
            }
            tokio::select! {
                _ = tokio::time::sleep_until(self.deadline) => return,
                result = epoch.changed() => if result.is_err() { return; },
            }
        }
    }
}

/// Derive a stable opaque scope without persisting or exposing the provider's
/// account identifier. The domain separator prevents this internal identity
/// from being confused with any other digest-derived application identifier.
fn stable_account_scope(account: &str) -> Uuid {
    let mut digest = Sha256::new();
    digest.update(b"research-assistant/chatgpt-account-scope/v1\0");
    digest.update(account.as_bytes());
    let bytes = digest.finalize();
    let mut scope = [0_u8; 16];
    scope.copy_from_slice(&bytes[..16]);
    Uuid::from_bytes(scope)
}

#[cfg(test)]
#[path = "tests.rs"]
pub(crate) mod tests;
