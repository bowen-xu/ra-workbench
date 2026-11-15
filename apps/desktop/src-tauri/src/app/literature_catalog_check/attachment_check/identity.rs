use super::scenarios::{diagnostic, require};
use super::*;
use std::time::Duration;

pub(super) async fn run(
    store: &AppStore,
    service: &Arc<LiteratureTraceService>,
    key: &str,
    attachment: &str,
) -> Result<Value, Error> {
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
    use std::sync::{
        Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    };
    let before = readback::snapshot(store)?;
    store.connection()?.execute("update reading_states set zoom='private synthetic attachment invalid zoom' where attachment_id=?1",[attachment])?;
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    struct Release(Arc<(Mutex<bool>, Condvar)>);
    impl Drop for Release {
        fn drop(&mut self) {
            if let Ok(mut value) = self.0.0.lock() {
                *value = true;
                self.0.1.notify_all();
            }
        }
    }
    let release = Release(Arc::clone(&gate));
    let timed_out = Arc::new(AtomicBool::new(false));
    let callback_timeout = Arc::clone(&timed_out);
    let (send, entered) = tokio::sync::oneshot::channel();
    let mut send = Some(send);
    store
        .connection()?
        .authorizer(Some(move |context: AuthContext<'_>| {
            if matches!(
                context.action,
                AuthAction::Read {
                    table_name: "reading_states",
                    ..
                }
            ) && let Some(send) = send.take()
            {
                let _ = send.send(());
                match gate.0.lock() {
                    Ok(guard) => {
                        match gate
                            .1
                            .wait_timeout_while(guard, Duration::from_secs(20), |ready| !*ready)
                        {
                            Ok((_, timeout)) => {
                                if timeout.timed_out() {
                                    callback_timeout.store(true, Ordering::Release);
                                }
                            }
                            Err(_) => callback_timeout.store(true, Ordering::Release),
                        }
                    }
                    Err(_) => callback_timeout.store(true, Ordering::Release),
                }
            }
            Authorization::Allow
        }));
    let owner = Arc::clone(service);
    let selected = key.to_owned();
    let db = store.clone();
    let id = attachment.to_owned();
    let worker = tokio::spawn(async move {
        owner
            .attachments(
                db,
                selected,
                AttachmentRequest::GetState {
                    item_id: fixture::PAPER.into(),
                    attachment_id: id,
                },
            )
            .await
    });
    tokio::time::timeout(Duration::from_secs(10), entered)
        .await
        .map_err(|_| "no-RPC read did not reach actual SQL gate")??;
    let mut shutdown = Box::pin(service.shutdown());
    require(
        matches!(
            std::future::poll_fn(|cx| std::task::Poll::Ready(std::future::Future::poll(
                shutdown.as_mut(),
                cx
            )))
            .await,
            std::task::Poll::Pending
        ),
        "shutdown did not wait for accepted read owner",
    )?;
    drop(release);
    let result = tokio::time::timeout(Duration::from_secs(20), worker)
        .await
        .map_err(|_| "accepted read/retirement did not settle")??;
    let error = result
        .err()
        .ok_or("invalid real reading-state codec unexpectedly succeeded")?;
    tokio::time::timeout(Duration::from_secs(20), shutdown)
        .await
        .map_err(|_| "actual owner shutdown did not settle")??;
    store
        .connection()?
        .authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
    store.connection()?.execute(
        "update reading_states set zoom=3.0 where attachment_id=?1",
        [attachment],
    )?;
    let value = diagnostic(&error, key, "storage_error")?;
    require(
        error.request_id.is_none()
            && error.data_error.as_ref().is_some_and(|e| {
                e.resource_id == "literature.attachments.reading-state-get"
                    && e.safe_cause == "invalid_column_type"
            })
            && !timed_out.load(Ordering::Acquire)
            && readback::snapshot(store)? == before,
        "no-RPC retirement inherited old request or changed fixed SQL prefix",
    )?;
    Ok(
        json!({"failure":value,"noNewCatalogRequest":true,"shutdownWaitedForActualSqlAndRetirement":true,
        "observerOnlyAllowedSql":true,"noArbitrarySleepOrSyntheticCleanup":true}),
    )
}
