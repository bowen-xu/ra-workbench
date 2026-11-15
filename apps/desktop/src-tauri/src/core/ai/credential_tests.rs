use super::*;

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        let path =
            std::env::temp_dir().join(format!("research-assistant-credential-{}", Uuid::new_v4()));
        fs::create_dir_all(&path).expect("create isolated credential directory");
        Self(path)
    }

    fn credential_path(&self) -> PathBuf {
        self.0.join("credentials.json")
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn write_private(path: &Path, bytes: &[u8]) {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options.open(path).expect("create private fixture");
    file.write_all(bytes).expect("write fixture");
    file.sync_all().expect("sync fixture");
}

#[test]
fn set_resolve_replace_and_delete_never_return_the_secret_in_summaries() {
    let directory = TestDirectory::new();
    let store = JsonCredentialStore::from_path(directory.credential_path());
    let initial = store.summary("deepseek").expect("missing summary");
    assert!(!initial.configured);

    let first = store
        .set("deepseek", "test-secret-one")
        .expect("set credential");
    assert_eq!(first.masked_value.as_deref(), Some("••••-one"));
    assert!(
        !serde_json::to_string(&first)
            .expect("serialize summary")
            .contains("test-secret-one")
    );
    assert_eq!(
        store.resolve("deepseek").expect("resolve").expose(),
        "test-secret-one"
    );

    store
        .set("deepseek", "test-secret-two")
        .expect("replace credential");
    assert_eq!(
        store
            .resolve("deepseek")
            .expect("resolve replacement")
            .expose(),
        "test-secret-two"
    );
    let deleted = store.delete("deepseek").expect("delete credential");
    assert!(!deleted.configured);
    assert!(matches!(
        store.resolve("deepseek"),
        Err(AiError::CredentialMissing)
    ));
}

#[test]
fn replacement_preserves_unknown_non_secret_extensions() {
    let directory = TestDirectory::new();
    let path = directory.credential_path();
    write_private(
        &path,
        br#"{
          "schemaVersion": 1,
          "providers": {
            "deepseek": {"apiKey": "old-secret", "accountLabel": "lab"}
          },
          "ownerExtension": {"version": 2}
        }"#,
    );
    let store = JsonCredentialStore::from_path(path.clone());
    store
        .set("deepseek", "replacement-secret")
        .expect("replace while preserving extensions");
    let value: Value = serde_json::from_slice(&fs::read(path).expect("read replaced file"))
        .expect("parse replaced file");
    assert_eq!(value["providers"]["deepseek"]["accountLabel"], "lab");
    assert_eq!(value["ownerExtension"]["version"], 2);
}

#[test]
fn explicit_development_store_reads_legacy_provider_sections_without_rewriting() {
    for key_name in ["api-key", "api_key", "apiKey"] {
        let directory = TestDirectory::new();
        let path = directory.credential_path();
        let original = format!(
            r#"{{
              "MinerU": {{"api-key": "unrelated-secret"}},
              "DeepSeek": {{"{key_name}": "deepseek-test-secret"}},
              "QWen": {{"api-key": "qwen-test-secret", "openai-compatible-url": "https://example.test/v1"}}
            }}"#
        );
        write_private(&path, original.as_bytes());
        let store = JsonCredentialStore::from_explicit_development_path(path.clone());

        let deepseek = store.summary("deepseek").expect("legacy DeepSeek summary");
        let qwen = store
            .summary("qwen-embedding")
            .expect("legacy QWen summary");
        assert!(deepseek.configured);
        assert!(qwen.configured);
        assert_eq!(
            store.resolve("deepseek").expect("legacy resolve").expose(),
            "deepseek-test-secret"
        );

        assert!(matches!(
            store.set("deepseek", "replacement-secret"),
            Err(AiError::CredentialInvalid)
        ));
        assert_eq!(
            fs::read(path).expect("legacy file preserved"),
            original.as_bytes()
        );
    }
}

#[test]
fn invalid_document_is_rejected_without_rewriting_original_bytes() {
    let directory = TestDirectory::new();
    let path = directory.credential_path();
    let original = br#"{"schemaVersion":2,"providers":{}}"#;
    write_private(&path, original);
    let store = JsonCredentialStore::from_path(path.clone());
    assert!(matches!(
        store.set("deepseek", "replacement-secret"),
        Err(AiError::CredentialInvalid)
    ));
    assert_eq!(fs::read(path).expect("read preserved file"), original);
}

#[cfg(unix)]
#[test]
fn unsafe_permissions_and_symlinks_are_rejected() {
    let directory = TestDirectory::new();
    let path = directory.credential_path();
    write_private(
        &path,
        br#"{"schemaVersion":1,"providers":{"deepseek":{"apiKey":"test-secret"}}}"#,
    );
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).expect("make permissions unsafe");
    let store = JsonCredentialStore::from_path(path.clone());
    assert!(matches!(
        store.summary("deepseek"),
        Err(AiError::CredentialPermissions)
    ));

    fs::remove_file(&path).expect("remove unsafe fixture");
    let target = directory.0.join("target.json");
    write_private(
        &target,
        br#"{"schemaVersion":1,"providers":{"deepseek":{"apiKey":"test-secret"}}}"#,
    );
    std::os::unix::fs::symlink(&target, &path).expect("create credential symlink");
    assert!(matches!(
        store.summary("deepseek"),
        Err(AiError::CredentialFileType)
    ));
}

#[test]
fn writes_leave_no_temporary_credential_files() {
    let directory = TestDirectory::new();
    let store = JsonCredentialStore::from_path(directory.credential_path());
    store
        .set("qwen-embedding", "embedding-test-secret")
        .expect("set embedding credential");
    let names = fs::read_dir(&directory.0)
        .expect("read isolated directory")
        .map(|entry| {
            entry
                .expect("directory entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect::<Vec<_>>();
    assert_eq!(names, vec!["credentials.json"]);
}
