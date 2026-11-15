use std::{fs, path::PathBuf};

use super::*;
use crate::ai::provider::ProviderErrorKind;

struct Fixture {
    connection: Connection,
    access: EmbeddingAccess,
    credentials: JsonCredentialStore,
    root: PathBuf,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn fixture() -> Fixture {
    let root = std::env::temp_dir().join(format!("core-embedding-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&root).expect("temporary credential directory");
    let credentials = JsonCredentialStore::from_path(root.join("credentials.json"));
    credentials
        .set("qwen-embedding", "synthetic-embedding-fixture-key")
        .expect("synthetic credential");
    let mut connection = Connection::open_in_memory().expect("database");
    crate::storage::migrate(&mut connection).expect("migrations");
    connection
        .execute(
            "update ai_provider_settings set enabled=1,
             base_url='https://dashscope.aliyuncs.com/compatible-mode/v1',
             model_id='text-embedding-v4', timeout_ms=30000
             where provider_id='qwen-embedding'",
            [],
        )
        .expect("embedding settings");
    Fixture {
        access: EmbeddingAccess::new(credentials.clone()),
        connection,
        credentials,
        root,
    }
}

#[test]
fn prepares_from_the_callers_transaction_and_keeps_its_frozen_configuration() {
    let mut fixture = fixture();
    let transaction = fixture.connection.transaction().expect("transaction");
    transaction
        .execute(
            "update ai_provider_settings set model_id='transaction-model', timeout_ms=7000
             where provider_id='qwen-embedding'",
            [],
        )
        .expect("uncommitted settings");
    let prepared = fixture.access.prepare_in(&transaction).expect("prepare");
    assert_eq!(prepared.provider_id(), "qwen-embedding");
    assert_eq!(prepared.model_id(), "transaction-model");
    assert_eq!(prepared.configuration.timeout_ms, 7000);
    transaction.rollback().expect("rollback settings");
    let next = fixture
        .access
        .prepare_in(&fixture.connection)
        .expect("prepare current settings");
    assert_eq!(next.model_id(), "text-embedding-v4");
    assert_eq!(next.configuration.timeout_ms, 30000);
    assert_eq!(prepared.model_id(), "transaction-model");
    assert_eq!(prepared.configuration.timeout_ms, 7000);
}

#[test]
fn rejects_disabled_settings_and_missing_credentials_without_returning_them() {
    let fixture = fixture();
    fixture
        .connection
        .execute(
            "update ai_provider_settings set enabled=0 where provider_id='qwen-embedding'",
            [],
        )
        .expect("disable");
    assert!(matches!(
        fixture.access.prepare_in(&fixture.connection),
        Err(EmbeddingCallError::Configuration)
    ));
    fixture
        .connection
        .execute(
            "update ai_provider_settings set enabled=1 where provider_id='qwen-embedding'",
            [],
        )
        .expect("enable");
    fixture
        .credentials
        .delete("qwen-embedding")
        .expect("delete synthetic credential");
    assert!(matches!(
        fixture.access.prepare_in(&fixture.connection),
        Err(EmbeddingCallError::Configuration)
    ));
}

#[tokio::test]
async fn cancellation_uses_the_existing_provider_without_sending_a_request() {
    let fixture = fixture();
    let prepared = fixture
        .access
        .prepare_in(&fixture.connection)
        .expect("prepare");
    let cancellation = CancellationSignal::default();
    cancellation.cancel();
    assert!(matches!(
        prepared
            .embed(vec!["synthetic input".to_owned()], 1024, &cancellation)
            .await,
        Err(EmbeddingCallError::Provider(ProviderError {
            kind: ProviderErrorKind::Canceled,
            ..
        }))
    ));
}

#[tokio::test]
async fn retains_existing_input_limits_and_provider_validation_before_cancellation() {
    let fixture = fixture();
    let cancellation = CancellationSignal::default();
    cancellation.cancel();
    // A canceled signal ensures these regression cases cannot send a request
    // even if input validation unexpectedly stops rejecting a malformed input.
    for (inputs, dimension) in [
        (Vec::new(), 1024),
        (vec!["input".to_owned(); 21], 1024),
        (vec![String::new()], 1024),
        (vec!["input".to_owned()], 0),
        (vec!["input".to_owned()], 4097),
    ] {
        let prepared = fixture
            .access
            .prepare_in(&fixture.connection)
            .expect("prepare");
        assert!(matches!(
            prepared.embed(inputs, dimension, &cancellation).await,
            Err(EmbeddingCallError::Provider(ProviderError {
                kind: ProviderErrorKind::InvalidRequest,
                ..
            }))
        ));
    }
}

#[tokio::test]
async fn invalid_timeout_remains_a_configuration_failure_before_provider_execution() {
    let fixture = fixture();
    let mut prepared = fixture
        .access
        .prepare_in(&fixture.connection)
        .expect("prepare");
    // Exercise the existing fallible signed-to-Duration conversion without
    // weakening schema validation or creating a deployed invalid migration.
    prepared.configuration.timeout_ms = -1;
    let cancellation = CancellationSignal::default();
    cancellation.cancel();
    assert!(matches!(
        prepared
            .embed(vec!["input".to_owned()], 1024, &cancellation)
            .await,
        Err(EmbeddingCallError::Configuration)
    ));
}
