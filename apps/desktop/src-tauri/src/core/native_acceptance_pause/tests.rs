use super::*;
use tokio::{io::AsyncWriteExt, net::UnixStream};

fn private_directory() -> tempfile::TempDir {
    let root = tempfile::tempdir_in("/private/tmp").unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    root
}

#[tokio::test]
async fn response_stays_pending_until_explicit_release_and_removes_endpoint() {
    let root = private_directory();
    let gate = Gate::bind(root.path(), EndpointName::ModelResponse)
        .await
        .unwrap();
    let path = root.path().join(EndpointName::ModelResponse.file_name());
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let mut waiting = Box::pin(gate.wait(Duration::from_secs(2)));
    std::future::poll_fn(|cx| {
        assert!(waiting.as_mut().poll(cx).is_pending());
        std::task::Poll::Ready(())
    })
    .await;
    let mut client = UnixStream::connect(&path).await.unwrap();
    client.write_u8(1).await.unwrap();
    assert_eq!(waiting.await, Ok(()));
    assert!(!path.exists());
}

#[tokio::test]
async fn cancellation_timeout_and_invalid_release_do_not_leave_a_listener() {
    let root = private_directory();
    let path = root.path().join(EndpointName::ModelResponse.file_name());
    let gate = Gate::bind(root.path(), EndpointName::ModelResponse)
        .await
        .unwrap();
    let mut waiting = Box::pin(gate.wait(Duration::from_secs(2)));
    std::future::poll_fn(|cx| {
        assert!(waiting.as_mut().poll(cx).is_pending());
        std::task::Poll::Ready(())
    })
    .await;
    drop(waiting);
    assert!(!path.exists());
    let gate = Gate::bind(root.path(), EndpointName::ModelResponse)
        .await
        .unwrap();
    assert_eq!(
        gate.wait(Duration::ZERO).await,
        Err(ModelError::Unavailable)
    );
    assert!(!path.exists());
    let gate = Gate::bind(root.path(), EndpointName::ModelResponse)
        .await
        .unwrap();
    let mut client = UnixStream::connect(&path).await.unwrap();
    client.write_u8(0).await.unwrap();
    assert_eq!(
        gate.wait(Duration::from_secs(2)).await,
        Err(ModelError::InvalidRequest)
    );
    assert!(!path.exists());
}

#[tokio::test]
async fn public_directories_and_existing_files_or_symlinks_are_rejected() {
    let root = private_directory();
    let path = root.path().join(EndpointName::ModelResponse.file_name());
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(matches!(
        Gate::bind(root.path(), EndpointName::ModelResponse).await,
        Err(ModelError::InvalidRequest)
    ));
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    std::fs::write(&path, b"existing").unwrap();
    assert!(matches!(
        Gate::bind(root.path(), EndpointName::ModelResponse).await,
        Err(ModelError::InvalidRequest)
    ));
    assert_eq!(std::fs::read(&path).unwrap(), b"existing");
    std::fs::remove_file(&path).unwrap();
    let target = root.path().join("target");
    std::fs::write(&target, b"unchanged").unwrap();
    std::os::unix::fs::symlink(&target, &path).unwrap();
    assert!(matches!(
        Gate::bind(root.path(), EndpointName::ModelResponse).await,
        Err(ModelError::InvalidRequest)
    ));
    assert!(path.symlink_metadata().unwrap().file_type().is_symlink());
    assert_eq!(std::fs::read(target).unwrap(), b"unchanged");
}
