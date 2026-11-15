use super::*;
use research_docling_runtime::{ENGINE_COMMIT, ENGINE_VERSION, ModelFile, REQUIRED_FILES};
use sha2::{Digest, Sha256};

#[test]
fn storage_counts_all_model_versions_and_removes_only_when_unused() {
    let root = tempfile::tempdir().unwrap();
    let resources = ModelResources::new(root.path().join("models"));
    assert_eq!(resources.storage_report().unwrap().total_bytes, 0);
    let guard = resources.begin_removal().unwrap();
    assert!(resources.begin_removal().is_err());
    assert!(resources.storage_report().unwrap().downloading);
    fs::create_dir_all(resources.root.join("old")).unwrap();
    fs::create_dir_all(resources.root.join("new")).unwrap();
    fs::write(resources.root.join("old/model.onnx"), b"old").unwrap();
    fs::write(resources.root.join("new/model.onnx"), b"new-model").unwrap();
    assert_eq!(resources.storage_report().unwrap().total_bytes, 12);
    let usage = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(resources.root.join("usage.lock"))
        .unwrap();
    fs2::FileExt::lock_shared(&usage).unwrap();
    assert!(resources.remove_models().is_err());
    assert!(resources.root.join("old/model.onnx").exists());
    drop(usage);
    assert_eq!(resources.remove_models().unwrap().total_bytes, 0);
    drop(guard);
    assert_eq!(resources.storage_report().unwrap().total_bytes, 0);
    assert!(!resources.storage_report().unwrap().downloading);
    assert_eq!(resources.status().state, "not_installed");
}

#[cfg(unix)]
#[test]
fn model_cleanup_rejects_symlinks_before_deleting_files() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let resources = ModelResources::new(root.path().join("models"));
    let _guard = resources.begin_removal().unwrap();
    fs::write(resources.root.join("keep"), b"keep").unwrap();
    fs::write(outside.path().join("private"), b"untouched").unwrap();
    std::os::unix::fs::symlink(outside.path(), resources.root.join("link")).unwrap();
    assert!(resources.remove_models().is_err());
    assert_eq!(fs::read(resources.root.join("keep")).unwrap(), b"keep");
    assert_eq!(
        fs::read(outside.path().join("private")).unwrap(),
        b"untouched"
    );
}

fn manifest(bytes: &[u8], version: &str) -> ModelManifest {
    ModelManifest {
        schema_version: 1,
        engine_version: ENGINE_VERSION.into(),
        engine_commit: ENGINE_COMMIT.into(),
        precision: "fp32".into(),
        version: version.into(),
        files: REQUIRED_FILES
            .iter()
            .copied()
            .chain(["licenses/NOTICE"])
            .map(|path| ModelFile {
                path: path.into(),
                size_bytes: bytes.len() as u64,
                sha256: format!("{:x}", Sha256::digest(bytes)),
            })
            .collect(),
    }
}

#[test]
fn permits_only_explicit_https_or_loopback_model_channels() {
    for url in [
        "file:///etc/latest.json",
        "http://example.com/latest.json",
        "http://localhost/latest.json",
        "https://user:secret@example.com/latest.json",
        "https://example.com/latest.json?key=x",
    ] {
        assert!(validated_url(url).is_err());
    }
    assert!(validated_url(research_docling_runtime::DEFAULT_MANIFEST_URL).is_ok());
    assert!(validated_url("https://example.com/models/latest.json").is_ok());
}

#[tokio::test]
async fn installs_updates_and_preserves_active_models_on_corrupt_download() {
    use http_body_util::Full;
    use hyper::{Response, body::Bytes, service::service_fn};
    use hyper_util::rt::TokioIo;
    let root = tempfile::tempdir().unwrap();
    let resources = ModelResources::new(root.path().to_owned());
    let served = Arc::new(Mutex::new((manifest(b"first", "v1"), b"first".to_vec())));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!(
        "http://{}/models/docling/latest.json",
        listener.local_addr().unwrap()
    );
    let server_state = served.clone();
    let server = tokio::spawn(async move {
        loop {
            let Ok((socket, _)) = listener.accept().await else {
                break;
            };
            let state = server_state.clone();
            tokio::spawn(async move {
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(
                        TokioIo::new(socket),
                        service_fn(move |request: hyper::Request<hyper::body::Incoming>| {
                            let state = state.clone();
                            async move {
                                let state = state.lock().unwrap();
                                let bytes = if request.uri().path().ends_with("latest.json") {
                                    serde_json::to_vec(&state.0).unwrap()
                                } else {
                                    state.1.clone()
                                };
                                Ok::<_, std::convert::Infallible>(Response::new(Full::new(
                                    Bytes::from(bytes),
                                )))
                            }
                        }),
                    )
                    .await;
            });
        }
    });
    let initial = resources.install(&endpoint).await.unwrap();
    assert_eq!(initial.installed_version.as_deref(), Some("v1"));
    resources
        .current()
        .unwrap()
        .0
        .verify(&resources.current().unwrap().1)
        .unwrap();
    *served.lock().unwrap() = (manifest(b"second", "v2"), b"second".to_vec());
    let checked = resources.check(&endpoint).await.unwrap();
    assert_ne!(checked.installed_identity, checked.available_identity);
    let updated = resources.install(&endpoint).await.unwrap();
    assert_eq!(updated.installed_version.as_deref(), Some("v2"));
    *served.lock().unwrap() = (manifest(b"third", "v3"), b"bad!!".to_vec());
    assert!(resources.install(&endpoint).await.is_err());
    let retained = resources.status();
    assert_eq!(retained.state, "failed");
    assert_eq!(retained.installed_identity, updated.installed_identity);
    assert_eq!(retained.installed_version.as_deref(), Some("v2"));
    server.abort();
}
