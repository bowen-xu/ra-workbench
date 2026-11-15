//! Controls only cancellation timing after the original process has settled.
//! This is not a reproduction of Darwin's initial signal failure.
use super::{super::NativeHost, WAIT};
use crate::{
    agent::process::{ProcessError, ProcessIoOperation},
    runtime_plugins::native::{
        NativePreviewService,
        package::{stage, tests::fixture},
    },
};
use std::{
    io,
    sync::{Arc, atomic::Ordering},
};
use tokio::{sync::Notify, time::timeout};

#[tokio::test]
async fn cancelling_real_service_shutdown_before_its_return_fences_later_calls() {
    let (_root, catalog, key) = fixture(
        br#"#!/bin/sh
IFS= read -r line
identity=${line%\"body\":*}
printf '%s"body":{"kind":"initialized","capability":"writing.import-preview","version":1}}\n' "$identity"
IFS= read -r ignored
exit 0
"#,
    );
    let staged = stage(&catalog, &key).unwrap();
    let directory = staged.directory.path().to_owned();
    let executable = staged.executable.clone();
    let mut host = NativeHost::spawn(staged, &key).unwrap();
    timeout(WAIT, host.initialize()).await.unwrap().unwrap();
    assert!(host.request_identity().is_some());
    assert!(host.process.exit().is_none());

    let entered = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    host.failed = true;
    host.initial_termination_error = Some(ProcessError::OperationIo {
        operation: ProcessIoOperation::GroupTerminate,
        kind: io::ErrorKind::PermissionDenied,
        raw_os_error: Some(nix::libc::EPERM),
    });
    host.cleanup_pause = Some((Arc::clone(&entered), release));
    let service = NativePreviewService::new(catalog);
    *service.host.lock().await = Some(host);

    let mut shutdown = Box::pin(service.shutdown());
    timeout(WAIT, async {
        tokio::select! {
            _ = entered.notified() => {},
            result = &mut shutdown => panic!("shutdown returned before its cancellation gate: {result:?}"),
        }
    })
    .await
    .unwrap();
    // The hook is after actual drain, input cleanup and exit identity capture.
    // Its awaited return is still unobserved by the real service's guard.
    assert!(!service.cleanup_failed.load(Ordering::Acquire));
    drop(shutdown);

    assert!(service.cleanup_failed.load(Ordering::Acquire));
    assert!(service.host.lock().await.is_none());
    assert!(!directory.exists());
    assert!(!executable.exists());
    let blocked = timeout(
        WAIT,
        service.preview(key.clone(), "/synthetic/not-dispatched".into()),
    )
    .await
    .unwrap()
    .unwrap_err();
    assert_eq!(blocked.code, "plugin_host_cleanup_blocked");
    assert_eq!(blocked.package_key.as_deref(), Some(key.as_str()));
    assert!(blocked.instance_id.is_none() && blocked.request_id.is_none());
    let repeated_shutdown = timeout(WAIT, service.shutdown())
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(repeated_shutdown.code, "plugin_host_cleanup_blocked");
    assert!(repeated_shutdown.cleanup_error.is_some());
    assert!(service.cleanup_failed.load(Ordering::Acquire));
}
