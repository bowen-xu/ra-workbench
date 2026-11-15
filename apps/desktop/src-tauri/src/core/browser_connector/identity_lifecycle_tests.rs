use super::identity::ReceiverIdentity;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::thread;
use std::time::Duration;

#[test]
fn shutdown_drain_waits_for_the_import_lease_after_its_http_waiter_is_gone() {
    let directory = tempfile::tempdir().expect("directory");
    let identity = Arc::new(ReceiverIdentity::open(directory.path()).expect("identity"));
    let completed = Arc::new(AtomicBool::new(false));
    let (entered, admitted) = mpsc::channel();
    let (release, finish_import) = mpsc::channel();
    let importer = {
        let identity = identity.clone();
        let completed = completed.clone();
        thread::spawn(move || {
            identity.with_scope(|_| {
                entered.send(()).expect("admitted");
                finish_import
                    .recv_timeout(Duration::from_secs(5))
                    .expect("release import");
                completed.store(true, Ordering::Release);
            })
        })
    };
    admitted
        .recv_timeout(Duration::from_secs(5))
        .expect("import acquired lease");
    let (draining, started) = mpsc::channel();
    let (finished, drained) = mpsc::channel();
    let shutdown = {
        let identity = identity.clone();
        let completed = completed.clone();
        thread::spawn(move || {
            draining.send(()).expect("starting drain");
            let result = identity.wait_for_imports();
            assert!(
                completed.load(Ordering::Acquire),
                "drain returned before the admitted import completed"
            );
            finished.send(result).expect("drain result");
        })
    };
    started
        .recv_timeout(Duration::from_secs(5))
        .expect("drain scheduled");
    assert!(matches!(drained.try_recv(), Err(mpsc::TryRecvError::Empty)));
    release.send(()).expect("finish import");
    assert_eq!(importer.join().expect("import thread"), Some(()));
    drained
        .recv_timeout(Duration::from_secs(5))
        .expect("drain completed")
        .expect("drained");
    shutdown.join().expect("shutdown thread");
}
