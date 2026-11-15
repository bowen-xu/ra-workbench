use super::*;
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
use tokio::{sync::oneshot, time::timeout};

struct OtherResource;
impl Resource for OtherResource {}

struct Dropped(Arc<AtomicBool>);
impl Drop for Dropped {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

#[tokio::test]
async fn cancelling_wait_drops_inflight_read_and_keeps_other_resources() {
    let mut table = ResourceTable::default();
    let other = table.add(OtherResource);
    let id = create(&mut table);
    let (receiver, _) = subscribe(&table, id).unwrap();
    let dropped = Arc::new(AtomicBool::new(false));
    let flag = Dropped(dropped.clone());
    let (started, ready) = oneshot::channel();
    let waiting = tokio::spawn(until_cancelled(receiver, async move {
        let _flag = flag;
        let _ = started.send(());
        std::future::pending::<()>().await
    }));
    ready.await.unwrap();
    cancel(&mut table, id);
    assert!(
        timeout(Duration::from_secs(1), waiting)
            .await
            .unwrap()
            .unwrap()
            .is_none()
    );
    assert!(dropped.load(Ordering::SeqCst));
    cancel(&mut table, id);
    cancel(&mut table, other);
    assert!(table.get::<OtherResource>(other).is_ok());
}

#[tokio::test]
async fn cancellation_before_wait_and_webview_resource_drop_are_observed() {
    let mut table = ResourceTable::default();
    let id = create(&mut table);
    cancel(&mut table, id);
    assert!(subscribe(&table, id).is_none());

    let id = create(&mut table);
    let (receiver, _) = subscribe(&table, id).unwrap();
    // Dropping the table models webview teardown: Tauri clears resources
    // without calling Resource::close, so sender ownership is significant.
    drop(table);
    assert!(
        timeout(
            Duration::from_secs(1),
            until_cancelled(receiver, std::future::pending::<()>())
        )
        .await
        .unwrap()
        .is_none()
    );
}

#[tokio::test]
async fn normal_wait_can_complete_and_then_release_its_resource() {
    let mut table = ResourceTable::default();
    let id = create(&mut table);
    let (receiver, owner) = subscribe(&table, id).unwrap();
    assert_eq!(until_cancelled(receiver, async { 7 }).await, Some(7));
    finish(&mut table, id, &owner);
    assert!(subscribe(&table, id).is_none());
}

#[test]
fn completing_old_wait_does_not_release_a_recreated_views_resource() {
    let mut old = ResourceTable::default();
    let id = create(&mut old);
    let (_, owner) = subscribe(&old, id).unwrap();
    let (lifetime, _) = watch::channel(false);
    old.replace(id, HistoryWaitResource { lifetime });
    finish(&mut old, id, &owner);
    assert!(subscribe(&old, id).is_some());
}

#[tokio::test]
async fn closing_a_window_only_cancels_its_waits_even_before_wait_starts() {
    let mut first_window = ResourceTable::default();
    let mut second_window = ResourceTable::default();
    let mut first = ResourceTable::default();
    let mut second = ResourceTable::default();
    let first_id = create(&mut first);
    let second_id = create(&mut second);
    let (first_receiver, first_owner) = subscribe(&first, first_id).unwrap();
    let (mut second_receiver, second_owner) = subscribe(&second, second_id).unwrap();
    window_waits(&mut first_window).track(first_owner);
    window_waits(&mut second_window).track(second_owner);
    window_waits(&mut first_window).mark_closed();
    assert!(
        timeout(
            Duration::from_secs(1),
            until_cancelled(first_receiver, std::future::pending::<()>())
        )
        .await
        .unwrap()
        .is_none()
    );
    let (late_receiver, _) = subscribe(&first, first_id).unwrap();
    assert!(
        timeout(
            Duration::from_secs(1),
            until_cancelled(late_receiver, std::future::pending::<()>())
        )
        .await
        .unwrap()
        .is_none()
    );
    assert!(
        timeout(Duration::from_millis(20), second_receiver.changed())
            .await
            .is_err()
    );
    cancel(&mut first, first_id);
    cancel(&mut second, second_id);
}

#[tokio::test]
async fn a_wait_created_after_window_destruction_is_cancelled() {
    let mut window = ResourceTable::default();
    window_waits(&mut window).mark_closed();
    let mut view = ResourceTable::default();
    let id = create(&mut view);
    let (receiver, owner) = subscribe(&view, id).unwrap();
    window_waits(&mut window).track(owner);
    assert!(
        timeout(
            Duration::from_secs(1),
            until_cancelled(receiver, std::future::pending::<()>())
        )
        .await
        .unwrap()
        .is_none()
    );
    // A different window, even with the same user-visible label, owns its own table.
    let mut new_window = ResourceTable::default();
    let mut new_view = ResourceTable::default();
    let new_id = create(&mut new_view);
    let (new_receiver, new_owner) = subscribe(&new_view, new_id).unwrap();
    window_waits(&mut new_window).track(new_owner);
    assert_eq!(until_cancelled(new_receiver, async { 7 }).await, Some(7));
}
