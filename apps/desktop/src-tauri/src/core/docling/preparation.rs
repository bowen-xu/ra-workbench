//! App-owned, exact-version preparation shared by reading and conversation tools.
use super::{
    error::DoclingError, manager::DoclingServiceManager, model::DoclingParseRunDetail, repository,
};
use crate::storage::AppStore;
use futures_util::{
    FutureExt,
    future::{BoxFuture, Shared},
};
use rusqlite::{OptionalExtension, params};
use std::{
    collections::HashMap,
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex, Weak},
};

type ParseFuture<'a> =
    Pin<Box<dyn Future<Output = Result<DoclingParseRunDetail, DoclingError>> + Send + 'a>>;
pub(crate) trait AttachmentParser: Send + Sync {
    fn identity(&self) -> Result<Option<String>, DoclingError> {
        Ok(None)
    }
    fn parse_controlled<'a>(
        &'a self,
        store: &'a AppStore,
        item: String,
        attachment: String,
        _control: Arc<super::task_control::ParseControl>,
    ) -> ParseFuture<'a> {
        self.parse(store, item, attachment)
    }
    fn parse<'a>(
        &'a self,
        store: &'a AppStore,
        item: String,
        attachment: String,
    ) -> ParseFuture<'a>;
}
struct NativeParser(DoclingServiceManager);
impl AttachmentParser for NativeParser {
    fn parse_controlled<'a>(
        &'a self,
        store: &'a AppStore,
        item: String,
        attachment: String,
        control: Arc<super::task_control::ParseControl>,
    ) -> ParseFuture<'a> {
        Box::pin(super::parse::convert(
            store, &self.0, item, attachment, control,
        ))
    }
    fn identity(&self) -> Result<Option<String>, DoclingError> {
        self.0.model_identity().map(Some)
    }
    fn parse<'a>(
        &'a self,
        store: &'a AppStore,
        item: String,
        attachment: String,
    ) -> ParseFuture<'a> {
        Box::pin(super::parse::convert(
            store,
            &self.0,
            item,
            attachment,
            super::task_control::ParseControl::new(),
        ))
    }
}
type Outcome = Result<DoclingParseRunDetail, Arc<DoclingError>>;
struct Flight {
    future: Shared<BoxFuture<'static, Outcome>>,
    control: Arc<super::task_control::ParseControl>,
    waiters: std::sync::atomic::AtomicUsize,
}
struct Waiter {
    flight: Arc<Flight>,
    active: bool,
}
impl Waiter {
    fn detach(&mut self) -> bool {
        if !self.active {
            return false;
        }
        self.active = false;
        let last = self
            .flight
            .waiters
            .fetch_sub(1, std::sync::atomic::Ordering::AcqRel)
            == 1;
        if last {
            self.flight.control.cancel();
        }
        last
    }
}
impl Drop for Waiter {
    fn drop(&mut self) {
        self.detach();
    }
}
type FlightKey = (String, String, String, Option<String>);
pub(crate) struct DoclingPreparation {
    parser: Arc<dyn AttachmentParser>,
    pub(super) tasks: super::tasks::ParseTasks,
    flights: Mutex<HashMap<FlightKey, Weak<Flight>>>,
}
impl DoclingPreparation {
    pub(crate) fn new(manager: DoclingServiceManager) -> Self {
        Self::with_parser(Arc::new(NativeParser(manager)))
    }
    pub(crate) fn with_parser(parser: Arc<dyn AttachmentParser>) -> Self {
        Self {
            parser,
            tasks: super::tasks::ParseTasks::default(),
            flights: Mutex::default(),
        }
    }
    pub(crate) async fn ensure(&self, store: &AppStore, item: &str, attachment: &str) -> Outcome {
        self.prepare(store, item, attachment, false).await
    }
    pub(crate) async fn prepare(
        &self,
        store: &AppStore,
        item: &str,
        attachment: &str,
        force: bool,
    ) -> Outcome {
        self.prepare_controlled(store, item, attachment, force, None)
            .await
    }
    pub(super) async fn prepare_controlled(
        &self,
        store: &AppStore,
        item: &str,
        attachment: &str,
        force: bool,
        observer: Option<Arc<super::task_control::ParseControl>>,
    ) -> Outcome {
        if observer.as_ref().is_some_and(|c| c.is_cancelled()) {
            return Err(Arc::new(DoclingError::Cancelled));
        }
        let hash = current_hash(store, item, attachment).map_err(Arc::new)?;
        let identity = self.parser.identity().map_err(Arc::new)?;
        let flight = {
            let mut flights = self
                .flights
                .lock()
                .map_err(|_| Arc::new(DoclingError::Disconnected))?;
            flights.retain(|_, flight| flight.strong_count() > 0);
            let entry = flights
                .entry((
                    item.to_owned(),
                    attachment.to_owned(),
                    hash.clone(),
                    identity.clone(),
                ))
                .or_default();
            let flight = match entry.upgrade() {
                Some(flight) => flight,
                None => {
                    let control = super::task_control::ParseControl::new();
                    let future = self.conversion(
                        store.clone(),
                        (
                            item.to_owned(),
                            attachment.to_owned(),
                            hash.clone(),
                            identity.clone(),
                        ),
                        force,
                        control.clone(),
                    );
                    let flight = Arc::new(Flight {
                        future,
                        control,
                        waiters: std::sync::atomic::AtomicUsize::new(0),
                    });
                    *entry = Arc::downgrade(&flight);
                    flight
                }
            };
            flight
                .waiters
                .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
            flight
        };
        // All concurrent callers share success AND failure. Once callers release the
        // flight, a later attempt can retry; no retained background task or PDF body.
        // A cancelled caller drops only its Shared handle; other waiters continue
        // the same conversion instead of restarting a cancelled initializer.
        let mut waiter = Waiter {
            flight: flight.clone(),
            active: true,
        };
        let outcome = if let Some(observer) = observer {
            observer.follow(&flight.control);
            let shared = flight.future.clone();
            tokio::pin!(shared);
            tokio::select! {
                result = &mut shared => result,
                _ = observer.cancelled() => {
                    if waiter.detach() {
                        let _ = shared.await; // Wait for in-flight page work to stop before confirming cancellation.
                    }
                    Err(Arc::new(DoclingError::Cancelled))
                }
            }
        } else {
            flight.future.clone().await
        };
        // Even waiters must check that the selected attachment is still current.
        if current_hash(store, item, attachment).map_err(Arc::new)? != hash {
            return Err(Arc::new(DoclingError::Attachment(
                "解析期间 PDF 版本已变化，请重试".into(),
            )));
        }
        if self.parser.identity().map_err(Arc::new)? != identity {
            return Err(Arc::new(DoclingError::Models(
                "解析期间模型已更新，请重试".into(),
            )));
        }
        outcome
    }
    fn conversion(
        &self,
        store: AppStore,
        key: FlightKey,
        force: bool,
        control: Arc<super::task_control::ParseControl>,
    ) -> Shared<BoxFuture<'static, Outcome>> {
        let (item, attachment, hash, identity) = key;
        let parser = self.parser.clone();
        async move {
            if !force
                && let Some(run) = cached(&store, &item, &attachment, &hash, identity.as_deref())
                    .map_err(Arc::new)?
            {
                return Ok(run);
            }
            let run = parser
                .parse_controlled(&store, item.clone(), attachment.clone(), control.clone())
                .await
                .map_err(Arc::new)?;
            if identity
                .as_deref()
                .is_some_and(|id| run.raw_response["runtime"]["identity"].as_str() != Some(id))
            {
                return Err(Arc::new(DoclingError::Models(
                    "解析模型版本与请求不一致，请重试".into(),
                )));
            }
            if run.summary.content_sha256 != hash
                || current_hash(&store, &item, &attachment).map_err(Arc::new)? != hash
            {
                return Err(Arc::new(DoclingError::Attachment(
                    "解析期间 PDF 版本已变化，请重试".into(),
                )));
            }
            Ok(run)
        }
        .boxed()
        .shared()
    }
}
fn current_hash(store: &AppStore, item: &str, attachment: &str) -> Result<String, DoclingError> {
    let db = store.connection()?;
    db.query_row("select a.content_sha256 from attachments a join bibliographic_items i on i.id=a.item_id where a.id=?1 and i.id=?2 and i.trashed_at is null and a.attachment_kind='pdf'", params![attachment,item], |row| row.get(0)).optional()?
        .ok_or_else(|| DoclingError::Attachment("PDF 附件已不可用或不属于当前文献".into()))
}
fn cached(
    store: &AppStore,
    item: &str,
    attachment: &str,
    hash: &str,
    identity: Option<&str>,
) -> Result<Option<DoclingParseRunDetail>, DoclingError> {
    let db = store.connection()?;
    let run: Option<String> = db.query_row("select id from docling_parse_runs where item_id=?1 and attachment_id=?2 and content_sha256=?3 and parse_status in ('success','partial_success') and json_content is not null and json_content != 'null' and (?4 is null or json_extract(raw_response, '$.runtime.identity')=?4) order by finished_at desc,id desc limit 1", params![item,attachment,hash,identity], |row| row.get(0)).optional()?;
    run.map(|id| repository::get_parse_run(&db, &id))
        .transpose()
}

#[cfg(test)]
#[path = "preparation/test_support.rs"]
pub(crate) mod test_support;
#[cfg(test)]
#[path = "preparation/tests.rs"]
mod tests;
