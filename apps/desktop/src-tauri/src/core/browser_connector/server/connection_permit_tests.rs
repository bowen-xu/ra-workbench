use std::{sync::mpsc, time::Duration};

use super::*;

#[tokio::test]
async fn cancelled_http_waiter_retains_admission_until_its_blocking_import_finishes() {
    let slots = Arc::new(Semaphore::new(1));
    let slot = Arc::new(slots.clone().try_acquire_owned().expect("admit import"));
    let (entered, admitted) = oneshot::channel();
    let (release, finish_import) = mpsc::channel();
    // This is the same blocking boundary awaited by handle(). Aborting this
    // outer task represents Hyper dropping a timed-out/disconnected request.
    let waiter = tokio::spawn(async move {
        spawn_import(slot, move || {
            entered.send(()).expect("import admitted");
            finish_import
                .recv_timeout(Duration::from_secs(5))
                .expect("release import");
        })
        .await
    });
    timeout(Duration::from_secs(5), admitted)
        .await
        .expect("admission deadline")
        .expect("import started");
    waiter.abort();
    assert!(
        waiter
            .await
            .expect_err("HTTP waiter cancelled")
            .is_cancelled()
    );
    assert!(
        slots.clone().try_acquire_owned().is_err(),
        "cancelled waiter released a running import's slot"
    );
    release.send(()).expect("finish admitted import");
    let returned_slot = timeout(Duration::from_secs(5), slots.acquire())
        .await
        .expect("import releases slot after completion")
        .expect("semaphore open");
    drop(returned_slot);
    assert_eq!(slots.available_permits(), 1);
}

#[tokio::test]
async fn successful_http_waiter_releases_the_admission_slot_after_its_import() {
    let slots = Arc::new(Semaphore::new(1));
    let slot = Arc::new(slots.clone().try_acquire_owned().expect("admit import"));
    let result = spawn_import(slot, || "durable receipt")
        .await
        .expect("import completed");
    assert_eq!(result, "durable receipt");
    assert_eq!(slots.available_permits(), 1);
}
