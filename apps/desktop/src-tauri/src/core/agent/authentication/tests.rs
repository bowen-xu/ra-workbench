use super::*;

fn response_for(exp: u64, account: &str) -> Value {
    let payload = URL_SAFE_NO_PAD.encode(
        serde_json::to_vec(&json!({
            "exp": exp, "https://api.openai.com/auth": {"chatgpt_account_id": account}
        }))
        .unwrap(),
    );
    json!({"authMethod": "chatgpt", "requiresOpenaiAuth": true,
        "authToken": format!("fixture.{payload}.synthetic-signature")})
}

fn response(exp: u64) -> Value {
    response_for(exp, "fixture-account")
}

fn lease(sender: &watch::Sender<Uuid>) -> ChatGptLease {
    ChatGptLease::from_response(
        response(5000),
        sender.subscribe(),
        *sender.borrow(),
        UNIX_EPOCH + Duration::from_secs(1000),
    )
    .unwrap()
}

pub(crate) fn synthetic_lease() -> (ChatGptLease, watch::Sender<Uuid>) {
    synthetic_lease_for_account("fixture-account")
}

pub(crate) fn synthetic_lease_for_account(account: &str) -> (ChatGptLease, watch::Sender<Uuid>) {
    let (sender, _) = watch::channel(Uuid::new_v4());
    let lease = ChatGptLease::from_response(
        response_for(5000, account),
        sender.subscribe(),
        *sender.borrow(),
        UNIX_EPOCH + Duration::from_secs(1000),
    )
    .unwrap();
    (lease, sender)
}

#[tokio::test]
async fn model_catalog_credentials_are_bound_to_exact_get_and_revocation() {
    let (lease, epoch) = synthetic_lease();
    let url = format!(
        "{}?client_version={}",
        crate::ai::codex_model_catalog::MODELS_URL,
        crate::ai::codex_model_catalog::CLIENT_VERSION
    );
    let client = reqwest::Client::new();
    for (method, destination) in [
        (Method::POST, url.clone()),
        (Method::GET, format!("{url}&extra=1")),
        (Method::GET, "https://example.com/models".into()),
        (Method::GET, format!("{url}#fragment")),
    ] {
        let mut request = client.request(method, destination).build().unwrap();
        assert_eq!(
            lease.authorize_catalog(&mut request, crate::ai::codex_model_catalog::CLIENT_VERSION),
            Err(AuthenticationError::Destination)
        );
        assert!(request.headers().get(AUTHORIZATION).is_none());
    }
    let mut request = client.get(&url).build().unwrap();
    lease
        .authorize_catalog(&mut request, crate::ai::codex_model_catalog::CLIENT_VERSION)
        .unwrap();
    assert!(request.headers().get(AUTHORIZATION).unwrap().is_sensitive());
    epoch.send_replace(Uuid::new_v4());
    let mut request = client.get(url).build().unwrap();
    assert_eq!(
        lease.authorize_catalog(&mut request, crate::ai::codex_model_catalog::CLIENT_VERSION),
        Err(AuthenticationError::Revoked)
    );
}

#[tokio::test]
async fn credentials_are_destination_bound_and_redacted() {
    let (sender, _) = watch::channel(Uuid::new_v4());
    let lease = lease(&sender);
    for url in [
        "https://api.openai.com/v1/responses",
        "http://chatgpt.com/backend-api/codex/responses",
        "https://chatgpt.com/backend-api/codex/responses?other=1",
        "https://user@chatgpt.com/backend-api/codex/responses",
        "https://chatgpt.com:444/backend-api/codex/responses",
        "https://chatgpt.com/backend-api/codex/responses/compact?other=1",
        "http://chatgpt.com/backend-api/codex/responses/compact",
        "https://user@chatgpt.com/backend-api/codex/responses/compact",
        "https://chatgpt.com:444/backend-api/codex/responses/compact",
        "https://chatgpt.com/backend-api/codex/responses/compact/other",
    ] {
        let mut request = Request::new(Method::POST, url.parse().unwrap());
        assert_eq!(
            lease.authorize(&mut request),
            Err(AuthenticationError::Destination)
        );
        assert!(request.headers().is_empty());
    }
    let mut request = Request::new(Method::GET, RESPONSE_URL.parse().unwrap());
    assert_eq!(
        lease.authorize(&mut request),
        Err(AuthenticationError::Destination)
    );
    *request.method_mut() = Method::POST;
    lease.authorize(&mut request).unwrap();
    assert!(request.headers()[AUTHORIZATION].is_sensitive());
    assert!(request.headers()["chatgpt-account-id"].is_sensitive());
    assert!(!format!("{:?}", request.headers()).contains("synthetic-signature"));
    let mut compact = Request::new(Method::GET, COMPACT_URL.parse().unwrap());
    assert_eq!(
        lease.authorize(&mut compact),
        Err(AuthenticationError::Destination)
    );
    assert!(compact.headers().is_empty());
    *compact.method_mut() = Method::POST;
    lease.authorize(&mut compact).unwrap();
    assert!(compact.headers()[AUTHORIZATION].is_sensitive());
    assert!(compact.headers()["chatgpt-account-id"].is_sensitive());
}

#[tokio::test]
async fn account_changes_and_bridge_loss_revoke_existing_leases() {
    let (sender, _) = watch::channel(Uuid::new_v4());
    let old = lease(&sender);
    sender.send_replace(Uuid::new_v4());
    assert_eq!(old.check(), Err(AuthenticationError::Revoked));
    tokio::time::timeout(Duration::from_secs(1), old.revoked())
        .await
        .unwrap();
    let current = lease(&sender);
    assert!(current.check().is_ok());
    drop(sender);
    tokio::time::timeout(Duration::from_secs(1), current.revoked())
        .await
        .unwrap();
    assert_eq!(current.check(), Err(AuthenticationError::Revoked));
}

#[tokio::test]
async fn admission_expiry_does_not_cancel_an_already_admitted_stream() {
    let (sender, _) = watch::channel(Uuid::new_v4());
    let mut lease = lease(&sender);
    lease.admission_deadline = Instant::now();
    let mut request = Request::new(Method::POST, RESPONSE_URL.parse().unwrap());
    assert_eq!(
        lease.authorize(&mut request),
        Err(AuthenticationError::Revoked)
    );
    assert!(lease.check().is_ok());
    lease.deadline = Instant::now();
    lease.revoked().await;
    assert_eq!(lease.check(), Err(AuthenticationError::Revoked));
}

#[test]
fn rejects_unsupported_or_expired_credentials_without_echoing_them() {
    let (sender, _) = watch::channel(Uuid::new_v4());
    let now = UNIX_EPOCH + Duration::from_secs(1000);
    for method in ["apikey", "chatgptAuthTokens", "headers", "agentIdentity"] {
        let mut value = response(5000);
        value["authMethod"] = json!(method);
        assert!(matches!(
            ChatGptLease::from_response(value, sender.subscribe(), *sender.borrow(), now),
            Err(AuthenticationError::Unsupported)
        ));
    }
    for value in [response(1030), response(1)] {
        assert!(matches!(
            ChatGptLease::from_response(value, sender.subscribe(), *sender.borrow(), now),
            Err(AuthenticationError::Revoked)
        ));
    }
    for token in ["", "private-token", "header.invalid.signature"] {
        let mut value = response(5000);
        value["authToken"] = json!(token);
        assert!(matches!(
            ChatGptLease::from_response(value, sender.subscribe(), *sender.borrow(), now),
            Err(AuthenticationError::InvalidCredential)
        ));
    }
}

#[tokio::test]
async fn revocation_wakes_a_live_waiter() {
    let (sender, _) = watch::channel(Uuid::new_v4());
    let lease = lease(&sender);
    let wait = lease.revoked();
    tokio::pin!(wait);
    assert!(matches!(
        futures_util::poll!(&mut wait),
        std::task::Poll::Pending
    ));
    sender.send_replace(Uuid::new_v4());
    tokio::time::timeout(Duration::from_secs(1), wait)
        .await
        .unwrap();
}

#[test]
fn rejects_missing_account_or_unsupported_routing() {
    let (sender, _) = watch::channel(Uuid::new_v4());
    for account in [
        json!({}),
        json!({"chatgpt_account_id": "bad\r\nheader"}),
        json!({"chatgpt_account_id": "fixture", "chatgpt_account_is_fedramp": true}),
    ] {
        let mut value = response(5000);
        let payload = URL_SAFE_NO_PAD.encode(
            serde_json::to_vec(&json!({
                "exp": 5000, "https://api.openai.com/auth": account
            }))
            .unwrap(),
        );
        value["authToken"] = json!(format!("fixture.{payload}.signature"));
        assert!(
            ChatGptLease::from_response(
                value,
                sender.subscribe(),
                *sender.borrow(),
                UNIX_EPOCH + Duration::from_secs(1000)
            )
            .is_err()
        );
    }
}

#[test]
fn stable_account_scope_survives_epochs_but_isolated_between_accounts() {
    let (first_epoch, _) = watch::channel(Uuid::new_v4());
    let (second_epoch, _) = watch::channel(Uuid::new_v4());
    let first = ChatGptLease::from_response(
        response_for(5000, "same-account"),
        first_epoch.subscribe(),
        *first_epoch.borrow(),
        UNIX_EPOCH + Duration::from_secs(1000),
    )
    .unwrap();
    let second = ChatGptLease::from_response(
        response_for(5000, "same-account"),
        second_epoch.subscribe(),
        *second_epoch.borrow(),
        UNIX_EPOCH + Duration::from_secs(1000),
    )
    .unwrap();
    let other = ChatGptLease::from_response(
        response_for(5000, "other-account"),
        second_epoch.subscribe(),
        *second_epoch.borrow(),
        UNIX_EPOCH + Duration::from_secs(1000),
    )
    .unwrap();

    assert_eq!(first.scope_id(), second.scope_id());
    assert_ne!(first.scope_id(), other.scope_id());
    assert!(!first.scope_id().is_nil());
    assert!(!format!("{:?}", first.scope_id()).contains("same-account"));
}

#[tokio::test]
async fn catalog_version_selection_keeps_exact_destination_and_query_authorization() {
    let (lease, _epoch) = synthetic_lease();
    let client = reqwest::Client::new();
    let url = format!(
        "{}?client_version=0.200.0",
        crate::ai::codex_model_catalog::MODELS_URL
    );
    let mut request = client.get(&url).build().unwrap();
    assert_eq!(
        lease.authorize_catalog(&mut request, crate::ai::codex_model_catalog::CLIENT_VERSION),
        Err(AuthenticationError::Destination)
    );
    assert!(request.headers().get(AUTHORIZATION).is_none());
    lease.authorize_catalog(&mut request, "0.200.0").unwrap();
    let mut invalid = client
        .get(format!(
            "{}?client_version=invalid",
            crate::ai::codex_model_catalog::MODELS_URL
        ))
        .build()
        .unwrap();
    assert_eq!(
        lease.authorize_catalog(&mut invalid, "invalid"),
        Err(AuthenticationError::Destination)
    );
    assert!(invalid.headers().get(AUTHORIZATION).is_none());
}
