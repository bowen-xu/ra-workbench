use super::*;
use crate::runtime_plugins::native::package::tests::literature_fixture;
use rusqlite::{
    Connection,
    hooks::{AuthAction, AuthContext, Authorization},
};
use serde_json::{Value, json};
use std::{fs, future::Future, path::Path, sync::mpsc, task::Poll};
use tokio::time::timeout;

const WAIT: Duration = Duration::from_secs(8);
const PDF: &[u8] = b"%PDF-1.7\nSynthetic concurrent attachment\n";

struct Fixture {
    _package: tempfile::TempDir,
    _data: tempfile::TempDir,
    service: Arc<LiteratureTraceService>,
    store: AppStore,
    key: String,
    attachment_id: String,
}

impl Fixture {
    fn new() -> Self {
        // The read path must work without executing or initializing this host.
        let (package, catalog, _) = literature_fixture(b"synthetic, never executed");
        let directory = package.path().join("writing");
        let file = directory.join("manifest.json");
        let mut manifest: Value = serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
        manifest["provides"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":"literature.attachments","version":1}));
        fs::write(file, serde_json::to_vec(&manifest).unwrap()).unwrap();
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../tests/fixtures/native-resources-v1/literature");
        for entry in fs::read_dir(source).unwrap() {
            let entry = entry.unwrap();
            let name = entry.file_name();
            let name = name.to_str().unwrap();
            if name.starts_with("attachment-") || name == "catalog-save-duplicate-insert.v1.sql" {
                fs::copy(entry.path(), directory.join("data").join(name)).unwrap();
            }
        }
        let selected = catalog.discover().unwrap().packages.remove(0);
        let resources = AttachmentResources::load(&catalog, &selected).unwrap();
        let data = tempfile::tempdir().unwrap();
        let mut db = Connection::open_in_memory().unwrap();
        crate::storage::migrate(&mut db).unwrap();
        db.execute_batch(
            "insert into bibliographic_items(id,item_type,title,created_at,updated_at)
            values('paper','document','Synthetic paper','before','before');",
        )
        .unwrap();
        let objects = data.path().join("objects");
        fs::create_dir(&objects).unwrap();
        let store = AppStore::for_test(db, objects);
        let path = data.path().join("source.pdf");
        fs::write(&path, PDF).unwrap();
        let result = literature_attachments::execute(
            &store,
            &resources,
            AttachmentRequest::Attach {
                item_id: "paper".into(),
                path: path.to_str().unwrap().into(),
            },
            None,
        )
        .unwrap();
        let AttachmentResult::Attachment(attachment) = result else {
            panic!("attachment")
        };
        Self {
            _package: package,
            _data: data,
            service: Arc::new(LiteratureTraceService::new(catalog)),
            store,
            key: selected.package_key,
            attachment_id: attachment.id,
        }
    }

    fn read(&self) -> AttachmentRequest {
        AttachmentRequest::Read {
            attachment_id: self.attachment_id.clone(),
        }
    }
}

#[tokio::test]
async fn pdf_bytes_position_and_index_reads_complete_while_host_is_busy() {
    let f = Fixture::new();
    let _host = f.service.host.lock().await;
    let reads = async {
        tokio::try_join!(
            f.service
                .attachments(f.store.clone(), f.key.clone(), f.read()),
            f.service.attachments(
                f.store.clone(),
                f.key.clone(),
                AttachmentRequest::GetState {
                    item_id: "paper".into(),
                    attachment_id: f.attachment_id.clone(),
                }
            ),
            f.service.attachments(
                f.store.clone(),
                f.key.clone(),
                AttachmentRequest::IsIndexed {
                    attachment_id: f.attachment_id.clone(),
                }
            ),
        )
    };
    let (bytes, state, indexed) = timeout(WAIT, reads).await.unwrap().unwrap();
    assert!(matches!(bytes, AttachmentResult::Bytes(bytes) if bytes == PDF));
    assert!(matches!(state, AttachmentResult::State(None)));
    assert!(matches!(indexed, AttachmentResult::Indexed(false)));
    assert!(_host.is_none());
}

#[tokio::test]
async fn readonly_attachment_rejects_changed_sql_before_execution() {
    let f = Fixture::new();
    let path = f
        ._package
        .path()
        .join("writing/data/attachment-content-path.v1.sql");
    let mut bytes = fs::read(&path).unwrap();
    bytes.push(b' ');
    fs::write(path, bytes).unwrap();
    let error = f
        .service
        .attachments(f.store.clone(), f.key.clone(), f.read())
        .await
        .unwrap_err();
    assert_eq!(error.code, "runtime_plugin_resource_changed");
    assert_eq!(error.package_key.as_deref(), Some(f.key.as_str()));
    assert!(error.instance_id.is_none() && error.request_id.is_none());
    assert!(f.service.host.lock().await.is_none());
    f.service.shutdown().await.unwrap();
}

#[tokio::test]
async fn cancelled_read_is_drained_before_shutdown_and_new_reads_are_rejected() {
    let f = Fixture::new();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let timed_out = Arc::new(AtomicBool::new(false));
    let barrier_timed_out = Arc::clone(&timed_out);
    let mut paused = false;
    // Stop at the real SQL read, after package acceptance and lifetime admission.
    f.store
        .connection()
        .unwrap()
        .authorizer(Some(move |context: AuthContext<'_>| {
            if !paused
                && matches!(
                    context.action,
                    AuthAction::Read {
                        table_name: "attachments",
                        ..
                    }
                )
            {
                paused = true;
                let _ = entered_tx.send(());
                if release_rx.recv_timeout(WAIT).is_err() {
                    barrier_timed_out.store(true, Ordering::SeqCst);
                }
            }
            Authorization::Allow
        }));
    let service = Arc::clone(&f.service);
    let store = f.store.clone();
    let key = f.key.clone();
    let request = f.read();
    let client = tokio::spawn(async move { service.attachments(store, key, request).await });
    let entered = tokio::task::spawn_blocking(move || entered_rx.recv_timeout(WAIT))
        .await
        .unwrap();
    if entered.is_err() {
        let _ = release_tx.send(());
        panic!("read did not reach SQL barrier");
    }
    client.abort();
    let cancelled = client.await.unwrap_err().is_cancelled();
    let mut shutdown = Box::pin(f.service.shutdown());
    let waiting =
        std::future::poll_fn(|cx| Poll::Ready(shutdown.as_mut().poll(cx).is_pending())).await;
    let rejected = timeout(
        WAIT,
        f.service
            .attachments(f.store.clone(), f.key.clone(), f.read()),
    )
    .await;
    let released = release_tx.send(()).is_ok();
    let result = timeout(WAIT, &mut shutdown).await;
    assert!(cancelled && waiting);
    assert!(released && !timed_out.load(Ordering::SeqCst));
    result.unwrap().unwrap();
    f.store
        .connection()
        .unwrap()
        .authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
    let error = rejected.unwrap().unwrap_err();
    assert_eq!(error.code, "plugin_host_stopping");
    assert_eq!(error.package_key.as_deref(), Some(f.key.as_str()));
}
