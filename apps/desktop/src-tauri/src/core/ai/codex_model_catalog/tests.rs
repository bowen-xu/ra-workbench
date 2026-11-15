use super::*;
use crate::agent::authentication::CredentialFuture;
use serde_json::json;
use std::{
    collections::VecDeque,
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

fn models(window: u64) -> Vec<CatalogModel> {
    serde_json::from_value(json!([{"slug":"gpt-fixture","display_name":"Fixture","visibility":"list","priority":0,
        "context_window":window,"max_context_window":1050000,"effective_context_window_percent":95,
        "supported_reasoning_levels":[{"effort":"high"},{"effort":"minimal"},{"effort":"ultra"}],"default_reasoning_level":"high",
        "service_tiers":[{"id":"priority"}]}])).unwrap()
}

#[test]
fn truncation_policy_survives_cache_without_rejecting_unknown_catalog_values() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("catalog.json");
    let mut values = serde_json::to_value(models(272000)).unwrap();
    let models = values.as_array_mut().unwrap();
    models[0]["truncation_policy"] = json!({"mode":"tokens","limit":10000,"future":true});
    let parsed: Vec<CatalogModel> = serde_json::from_value(values.clone()).unwrap();
    model::validate(&parsed).unwrap();
    let policy = parsed[0].truncation_policy.unwrap();
    assert_eq!(policy.mode, model::TruncationMode::Tokens);
    assert_eq!(policy.limit, 10000);
    cache::write(
        &path,
        &CatalogCache {
            schema_version: 1,
            client_version: CLIENT_VERSION.into(),
            fetched_at: 1,
            etag: None,
            models: parsed,
        },
    )
    .unwrap();
    let cached = cache::read(&path).unwrap();
    assert_eq!(cached.models[0].truncation_policy, Some(policy));

    for value in [
        json!({"mode":"future","limit":10000}),
        json!(null),
        json!("unknown"),
    ] {
        values[0]["truncation_policy"] = value;
        let parsed: Vec<CatalogModel> = serde_json::from_value(values.clone()).unwrap();
        model::validate(&parsed).unwrap();
        assert_eq!(parsed[0].truncation_policy, None);
    }
    values[0]
        .as_object_mut()
        .unwrap()
        .remove("truncation_policy");
    let old: Vec<CatalogModel> = serde_json::from_value(values).unwrap();
    assert_eq!(old[0].truncation_policy, None);

    let bundled = bundled_snapshot().unwrap();
    let tokens = bundled
        .model("gpt-5.6-sol")
        .unwrap()
        .truncation_policy
        .unwrap();
    assert_eq!(
        (tokens.mode, tokens.limit),
        (model::TruncationMode::Tokens, 10000)
    );
    let bytes = bundled.model("gpt-5.2").unwrap().truncation_policy.unwrap();
    assert_eq!(
        (bytes.mode, bytes.limit),
        (model::TruncationMode::Bytes, 10000)
    );
    use research_agent_core::model::tool_output::ToolOutputPolicy;
    assert_eq!(tokens.shared(), Some(ToolOutputPolicy::Tokens(10000)));
    assert_eq!(bytes.shared(), Some(ToolOutputPolicy::Bytes(10000)));
    let invalid = model::TruncationPolicy {
        mode: model::TruncationMode::Tokens,
        limit: -1,
    };
    assert_eq!(invalid.shared(), None);
}
struct Auth {
    account: Mutex<String>,
    epochs: Mutex<Vec<tokio::sync::watch::Sender<uuid::Uuid>>>,
}
impl ChatGptCredentialSource for Auth {
    fn acquire(&self) -> CredentialFuture<'_> {
        Box::pin(async {
            let (lease, epoch) = crate::agent::authentication::tests::synthetic_lease_for_account(
                &self.account.lock().unwrap(),
            );
            self.epochs.lock().unwrap().push(epoch);
            Ok(lease)
        })
    }
}
struct FakeTransport {
    calls: AtomicUsize,
    replies: Mutex<VecDeque<transport::FetchResult>>,
    etags: Mutex<Vec<Option<String>>>,
    versions: Mutex<Vec<String>>,
}
impl transport::CatalogTransport for FakeTransport {
    fn fetch<'a>(
        &'a self,
        lease: &'a ChatGptLease,
        previous: Option<&'a CatalogCache>,
        version: &'a str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = transport::FetchResult> + Send + 'a>>
    {
        Box::pin(async move {
            lease
                .check()
                .map_err(|_| AiError::Provider("authentication"))?;
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.versions.lock().unwrap().push(version.into());
            self.etags
                .lock()
                .unwrap()
                .push(previous.and_then(|cache| cache.etag.clone()));
            self.replies
                .lock()
                .unwrap()
                .pop_front()
                .expect("unexpected network fetch")
        })
    }
}
fn fixture() -> (
    tempfile::TempDir,
    Arc<Auth>,
    Arc<FakeTransport>,
    CodexModelCatalog,
) {
    let directory = tempfile::tempdir().unwrap();
    let auth = Arc::new(Auth {
        account: Mutex::new("account-a".into()),
        epochs: Mutex::new(vec![]),
    });
    let transport = Arc::new(FakeTransport {
        calls: AtomicUsize::new(0),
        replies: Mutex::new(VecDeque::new()),
        etags: Mutex::new(vec![]),
        versions: Mutex::new(vec![]),
    });
    let catalog = CodexModelCatalog {
        source: auth.clone(),
        transport: transport.clone(),
        directory: directory.path().join("cache"),
        overrides: directory.path().join("overrides.json"),
        version_settings: directory.path().join("versions.json"),
        refresh: tokio::sync::Mutex::new(()),
    };
    (directory, auth, transport, catalog)
}
#[tokio::test]
async fn discovery_cache_force_refresh_and_execution_share_window_metadata() {
    let (_directory, _auth, transport, catalog) = fixture();
    transport.replies.lock().unwrap().extend([
        Ok((models(272000), Some("etag-1".into()))),
        Ok((models(500000), Some("etag-2".into()))),
    ]);
    let first = catalog.load(false).await.unwrap();
    assert_eq!(
        first.provider_models().models[0].context_window_tokens,
        Some(258400)
    );
    assert_eq!(first.model("gpt-fixture").unwrap().window(), Some(272000));
    assert_eq!(catalog.load(false).await.unwrap().status.source, "cache");
    assert_eq!(transport.calls.load(Ordering::SeqCst), 1);
    let refreshed = catalog.load(true).await.unwrap();
    assert_eq!(
        refreshed.provider_models().models[0].context_window_tokens,
        Some(475000)
    );
    assert_eq!(
        first.model("gpt-fixture").unwrap().window(),
        Some(272000),
        "captured snapshots remain unchanged"
    );
    assert_eq!(
        *transport.etags.lock().unwrap(),
        vec![None, Some("etag-1".into())]
    );
    assert_eq!(
        refreshed.provider_models().models[0].supported_reasoning_efforts,
        vec!["high"]
    );
    assert!(refreshed.provider_models().models[0].supports_fast_mode);
}

#[tokio::test]
async fn legacy_cache_refreshes_without_etag_and_survives_offline_failure() {
    let (_directory, auth, transport, catalog) = fixture();
    let lease = auth.acquire().await.unwrap();
    let path = catalog.directory.join(format!("{}.json", lease.scope_id()));
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    cache::write(
        &path,
        &CatalogCache {
            schema_version: 1,
            client_version: CLIENT_VERSION.into(),
            fetched_at: now,
            etag: Some("legacy-etag".into()),
            models: models(272000),
        },
    )
    .unwrap();
    let mut updated = models(500000);
    updated[0].truncation_policy = Some(model::TruncationPolicy {
        mode: model::TruncationMode::Tokens,
        limit: 10000,
    });
    transport.replies.lock().unwrap().extend([
        Err(AiError::TemporaryUnavailable),
        Ok((updated, Some("new-etag".into()))),
    ]);
    let offline = catalog.load(false).await.unwrap();
    assert_eq!(offline.status.source, "cache");
    assert!(offline.status.stale);
    assert_eq!(offline.model("gpt-fixture").unwrap().window(), Some(272000));
    assert_eq!(cache::read(&path).unwrap().schema_version, 1);
    let refreshed = catalog.load(false).await.unwrap();
    assert_eq!(refreshed.status.source, "remote");
    assert!(!refreshed.status.stale);
    assert_eq!(
        refreshed
            .model("gpt-fixture")
            .unwrap()
            .truncation_policy
            .unwrap()
            .limit,
        10000
    );
    let saved = cache::read(&path).unwrap();
    assert_eq!(saved.schema_version, CURRENT_SCHEMA_VERSION);
    assert_eq!(saved.etag.as_deref(), Some("new-etag"));
    assert_eq!(
        saved.models[0].truncation_policy.unwrap().mode,
        model::TruncationMode::Tokens
    );
    assert_eq!(catalog.load(false).await.unwrap().status.source, "cache");
    assert_eq!(*transport.etags.lock().unwrap(), vec![None, None]);
    assert_eq!(transport.calls.load(Ordering::SeqCst), 2);
}
#[tokio::test]
async fn transient_failure_uses_same_account_cache_but_authentication_failure_does_not() {
    let (_directory, auth, transport, catalog) = fixture();
    transport.replies.lock().unwrap().extend([
        Ok((models(500000), None)),
        Err(AiError::Timeout),
        Err(AiError::Provider("authentication")),
        Err(AiError::Timeout),
    ]);
    catalog.load(true).await.unwrap();
    let stale = catalog.load(true).await.unwrap();
    assert!(stale.status.stale);
    assert_eq!(stale.status.source, "cache");
    assert_eq!(stale.model("gpt-fixture").unwrap().window(), Some(500000));
    assert!(catalog.load(true).await.is_err());
    *auth.account.lock().unwrap() = "account-b".into();
    let other = catalog.load(false).await.unwrap();
    assert_eq!(other.status.source, "bundled");
    assert!(other.model("gpt-fixture").is_none());
    assert!(transport.etags.lock().unwrap().last().unwrap().is_none());
    model::validate(&other.models).unwrap();
}
#[tokio::test]
async fn remote_replacement_removes_retired_models_and_hidden_models_stay_out_of_picker() {
    let (_directory, _auth, transport, catalog) = fixture();
    let mut initial = models(272000);
    let mut hidden = initial[0].clone();
    hidden.slug = "hidden".into();
    hidden.visibility = "hide".into();
    initial.push(hidden);
    let mut replacement = models(500000);
    replacement[0].slug = "new-model".into();
    transport
        .replies
        .lock()
        .unwrap()
        .extend([Ok((initial, None)), Ok((replacement, None))]);
    let first = catalog.load(true).await.unwrap();
    assert_eq!(first.provider_models().models.len(), 1);
    let latest = catalog.load(true).await.unwrap();
    assert!(latest.model("gpt-fixture").is_none());
    assert_eq!(latest.provider_models().models[0].id, "new-model");
}
#[tokio::test]
async fn context_override_is_capped_and_does_not_change_persisted_catalog() {
    let (_directory, _auth, transport, catalog) = fixture();
    transport
        .replies
        .lock()
        .unwrap()
        .push_back(Ok((models(272000), None)));
    std::fs::write(&catalog.overrides,json!({"schemaVersion":1,"models":{"gpt-fixture":{"contextWindowTokens":2000000,"autoCompactTokenLimit":800000}}}).to_string()).unwrap();
    let snapshot = catalog.load(false).await.unwrap();
    assert_eq!(
        snapshot.model("gpt-fixture").unwrap().window(),
        Some(1050000)
    );
    assert_eq!(
        snapshot
            .model("gpt-fixture")
            .unwrap()
            .auto_compact_token_limit,
        Some(800000)
    );
    std::fs::remove_file(&catalog.overrides).unwrap();
    assert_eq!(
        catalog
            .load(false)
            .await
            .unwrap()
            .model("gpt-fixture")
            .unwrap()
            .window(),
        Some(272000)
    );
    std::fs::write(&catalog.overrides, "invalid").unwrap();
    assert!(catalog.load(false).await.is_err());
}
#[test]
fn malformed_duplicate_or_unsafe_metadata_is_rejected() {
    assert!(model::validate(&[]).is_ok());
    let values = models(272000);
    assert!(model::validate(&[values[0].clone(), values[0].clone()]).is_err());
    let mut invalid = values[0].clone();
    invalid.effective_context_window_percent = 0;
    assert!(model::validate(&[invalid]).is_err());
    let mut invalid = values[0].clone();
    invalid.max_context_window = Some(1);
    assert!(model::validate(&[invalid]).is_err());
    let mut missing_default = values[0].clone();
    missing_default.context_window = None;
    assert_eq!(
        missing_default.window(),
        None,
        "a maximum is not a default window"
    );
    assert!(
        missing_default
            .descriptor(false)
            .context_window_tokens
            .is_none()
    );
}

#[test]
fn adapter_version_and_sanitized_bundle_remain_tied_to_the_reviewed_sidecar() {
    let package: serde_json::Value =
        serde_json::from_str(include_str!("../../../../../package.json")).unwrap();
    assert_eq!(package["devDependencies"]["@openai/codex"], CLIENT_VERSION);
    let snapshot = bundled_snapshot().unwrap();
    model::validate(&snapshot.models).unwrap();
    let wire = json!({"models":[{"slug":"fixture","display_name":"Fixture","visibility":"list","context_window":272000,
        "base_instructions":"must not be persisted","model_messages":{"secret":"not metadata"},"unknown_field":true}]});
    let parsed = transport::decode_models(&serde_json::to_vec(&wire).unwrap()).unwrap();
    let encoded = serde_json::to_string(&parsed).unwrap();
    assert!(!encoded.contains("base_instructions"));
    assert!(!encoded.contains("model_messages"));
    assert!(!encoded.contains("unknown_field"));
    assert!(transport::decode_models(b"not json").is_err());
    assert!(transport::decode_models(b"{}").is_err());
    assert!(transport::decode_models(&vec![b' '; MAX_BYTES + 1]).is_err());
}

#[tokio::test]
async fn expired_or_incompatible_cache_refreshes_and_logout_invalidates_it() {
    let (_directory, auth, transport, catalog) = fixture();
    transport.replies.lock().unwrap().extend([
        Ok((models(272000), None)),
        Ok((models(500000), None)),
        Ok((models(272000), None)),
    ]);
    catalog.load(false).await.unwrap();
    let lease = auth.acquire().await.unwrap();
    let path = catalog.directory.join(format!("{}.json", lease.scope_id()));
    let mut cached = cache::read(&path).unwrap();
    cached.fetched_at -= TTL + 1;
    cache::write(&path, &cached).unwrap();
    assert_eq!(catalog.load(false).await.unwrap().status.source, "remote");
    cached = cache::read(&path).unwrap();
    cached.client_version = "obsolete".into();
    cache::write(&path, &cached).unwrap();
    assert!(cache::read(&path).is_none());
    catalog.load(false).await.unwrap();
    assert_eq!(transport.calls.load(Ordering::SeqCst), 3);
    assert!(transport.etags.lock().unwrap().last().unwrap().is_none());
    catalog.invalidate().await;
    assert!(!catalog.directory.exists());
}

struct RevokingTransport(Arc<Auth>);
impl transport::CatalogTransport for RevokingTransport {
    fn fetch<'a>(
        &'a self,
        _: &'a ChatGptLease,
        _: Option<&'a CatalogCache>,
        _: &'a str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = transport::FetchResult> + Send + 'a>>
    {
        Box::pin(async move {
            for epoch in self.0.epochs.lock().unwrap().iter() {
                let _ = epoch.send(uuid::Uuid::new_v4());
            }
            Ok((models(500000), None))
        })
    }
}
#[tokio::test]
async fn account_revocation_during_fetch_neither_publishes_nor_persists_metadata() {
    let (_directory, auth, _transport, mut catalog) = fixture();
    catalog.transport = Arc::new(RevokingTransport(auth));
    assert!(matches!(
        catalog.load(true).await,
        Err(AiError::Provider("authentication"))
    ));
    assert!(!catalog.directory.exists());
}

#[test]
fn corrupt_and_oversized_cache_cannot_be_used_as_model_authority() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("catalog.json");
    std::fs::write(&path, b"invalid").unwrap();
    assert!(cache::read(&path).is_none());
    std::fs::write(&path, vec![b' '; MAX_BYTES + 1]).unwrap();
    assert!(cache::read_bounded(&path).is_err());
}

#[tokio::test]
async fn selected_catalog_version_persists_and_never_reuses_other_version_etags() {
    let (_directory, _auth, transport, catalog) = fixture();
    transport.replies.lock().unwrap().extend([
        Ok((models(272000), Some("old-etag".into()))),
        Ok((models(500000), Some("new-etag".into()))),
    ]);
    catalog.load(false).await.unwrap();
    let mut versions = catalog.versions(false).await.unwrap();
    versions.available_versions.push("0.200.0".into());
    cache::write_json(&catalog.version_settings, &versions).unwrap();
    let selected = catalog.select_version("0.200.0".into()).await.unwrap();
    assert_eq!(selected.runtime_version, CLIENT_VERSION);
    assert_eq!(
        catalog.versions(false).await.unwrap().selected_version,
        "0.200.0"
    );
    assert_eq!(
        catalog
            .load(false)
            .await
            .unwrap()
            .model("gpt-fixture")
            .unwrap()
            .window(),
        Some(500000)
    );
    assert_eq!(
        *transport.versions.lock().unwrap(),
        vec![CLIENT_VERSION, "0.200.0"]
    );
    assert_eq!(*transport.etags.lock().unwrap(), vec![None, None]);
    assert!(catalog.select_version("9.9.9".into()).await.is_err());
    assert_eq!(
        catalog.versions(false).await.unwrap().selected_version,
        "0.200.0"
    );
}
