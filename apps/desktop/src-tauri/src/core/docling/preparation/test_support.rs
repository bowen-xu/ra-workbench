//! Deterministic conversion boundary: never starts a daemon or reads a user's PDF.
use super::*;
use serde_json::json;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use tokio::sync::Semaphore;
pub(crate) struct Parser {
    pub calls: AtomicUsize,
    pub started: Semaphore,
    pub release: Semaphore,
    pub fail: AtomicBool,
}
impl Parser {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            calls: AtomicUsize::new(0),
            started: Semaphore::new(0),
            release: Semaphore::new(0),
            fail: AtomicBool::new(false),
        })
    }
    pub fn install(self: &Arc<Self>, store: &AppStore) -> Arc<DoclingPreparation> {
        let preparation = Arc::new(DoclingPreparation::with_parser(self.clone()));
        assert!(store.docling.set(preparation.clone()).is_ok());
        preparation
    }
    pub async fn wait_started(&self) {
        self.started.acquire().await.unwrap().forget();
    }
}
impl AttachmentParser for Parser {
    fn parse<'a>(
        &'a self,
        store: &'a AppStore,
        item: String,
        attachment: String,
    ) -> ParseFuture<'a> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let hash = current_hash(store, &item, &attachment)?;
            self.started.add_permits(1);
            self.release.acquire().await.unwrap().forget();
            if self.fail.load(Ordering::SeqCst) {
                return Err(DoclingError::Request(
                    "synthetic private transport detail".into(),
                ));
            }
            let db = store.connection()?;
            repository::save_parse_response(
                &db,
                repository::ParsedDoclingResponse {
                    item_id: item,
                    attachment_id: attachment,
                    content_sha256: hash,
                    service_base_url: "fixture".into(),
                    docling_status: "success".into(),
                    parse_status: "success".into(),
                    json_content: Some(
                        json!({"texts":[{"self_ref":"#/texts/0","text":"structured evidence","prov":[{"page_no":1}]}]}),
                    ),
                    raw_response: json!({"status":"success"}),
                    error_message: None,
                    processing_time_seconds: None,
                },
            )
        })
    }
}
pub(crate) fn attachment(store: &AppStore, item: &str, id: &str, hash: &str) {
    let db = store.connection().unwrap();
    db.execute("insert or ignore into content_objects(sha256,relative_path,mime_type,byte_size,created_at) values (?1,?1,'application/pdf',1,'now')",[hash]).unwrap();
    db.execute("insert into attachments(id,item_id,content_sha256,display_name,created_at) values (?1,?2,?3,'fixture.pdf','now')",params![id,item,hash]).unwrap();
}
