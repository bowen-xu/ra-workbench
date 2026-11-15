use super::{
    Shared,
    identity::{ReceiverIdentity, new_connection_token},
    protocol::{INSTANCE_HEADER, TOKEN_HEADER},
    server,
};
use crate::storage::{AppStore, migrate};
use reqwest::{Client, RequestBuilder};
use rusqlite::Connection;
use std::{path::Path, sync::Arc};
use tokio::sync::oneshot;

pub(super) struct TestServer {
    pub(super) shared: Arc<Shared>,
    pub(super) stop: oneshot::Sender<()>,
    pub(super) task: tokio::task::JoinHandle<()>,
    pub(super) token: String,
    pub(super) client: Client,
}

impl TestServer {
    pub(super) async fn start_with_binding(
        directory: &Path,
        binding: Option<crate::runtime_plugins::literature_trace::binding::NoteTraceBinding>,
    ) -> Self {
        let content = directory.join("objects");
        std::fs::create_dir_all(&content).expect("objects");
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("loopback");
        listener.set_nonblocking(true).expect("nonblocking");
        let host = listener.local_addr().expect("address").to_string();
        let identity = ReceiverIdentity::open(&directory.join("receiver")).expect("identity");
        let token = new_connection_token();
        let mut store = if binding.is_some() {
            use crate::{
                runtime_events::RuntimeEvents,
                storage::{AppStorePaths, AppStorePorts, AppStoreSources},
            };
            AppStore::open_with_paths(
                AppStorePaths {
                    data_directory: directory.into(),
                    agent_configuration_directory: directory.join("configuration"),
                },
                AppStorePorts::with_events(
                    RuntimeEvents::new(|_, _| Ok(())),
                    AppStoreSources::default(),
                ),
            )
            .expect("Core-only source composition")
        } else {
            let mut connection =
                Connection::open(directory.join("capture.sqlite3")).expect("database");
            migrate(&mut connection).expect("migrations");
            AppStore::for_test(connection, content)
        };
        if let Some(binding) = binding {
            store = store.with_native_note_trace(binding);
        }
        let wait = if store.native_note_trace().is_some() {
            70
        } else {
            10
        };
        let shared = Arc::new(Shared {
            store,
            capture: crate::browser_capture_app::dispatch,
            identity,
            environment: "development",
            instance_id: uuid::Uuid::new_v4().simple().to_string(),
            connection_token: token.clone(),
            active: std::sync::atomic::AtomicBool::new(true),
            host,
            app: None,
        });
        let (stop, stopped) = oneshot::channel();
        let task = tokio::spawn(server::run(listener, shared.clone(), stopped));
        let client = Client::builder()
            .no_proxy()
            .timeout(std::time::Duration::from_secs(wait))
            .build()
            .expect("client");
        Self {
            shared,
            stop,
            task,
            token,
            client,
        }
    }

    pub(super) fn post(&self, route: &str) -> RequestBuilder {
        self.client
            .post(format!("http://{}/connector/{route}", self.shared.host))
            .header(TOKEN_HEADER, &self.token)
            .header(INSTANCE_HEADER, &self.shared.instance_id)
            .header("X-Zotero-Connector-API-Version", "3")
            .header("X-Research-Assistant-Connector-API-Version", "2")
    }

    pub(super) async fn oversized_pdf_status(&self) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut stream = tokio::net::TcpStream::connect(&self.shared.host)
            .await
            .unwrap();
        // Only headers are sent: production rejects this declared size before reading a body.
        // Sending 64 MiB eagerly can reset the client while the server sends its 413 response.
        let headers = format!(
            "POST /connector/saveStandaloneAttachment?sessionID=oversized-pdf HTTP/1.1\r\nHost: {}\r\nContent-Type: application/pdf\r\nX-Metadata: {{\"title\":\"oversized.pdf\",\"contentType\":\"application/pdf\"}}\r\nContent-Length: {}\r\n{}: {}\r\n{}: {}\r\nX-Zotero-Connector-API-Version: 3\r\nX-Research-Assistant-Connector-API-Version: 2\r\nConnection: close\r\n\r\n",
            self.shared.host,
            64 * 1024 * 1024 + 1,
            TOKEN_HEADER,
            self.token,
            INSTANCE_HEADER,
            self.shared.instance_id,
        );
        stream.write_all(headers.as_bytes()).await.unwrap();
        let mut response = Vec::new();
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            stream.read_to_end(&mut response),
        )
        .await
        .unwrap()
        .unwrap();
        String::from_utf8(response)
            .unwrap()
            .lines()
            .next()
            .unwrap()
            .to_owned()
    }

    pub(super) async fn shutdown(self) {
        self.close(false).await;
    }

    pub(super) async fn shutdown_fenced(self) {
        self.close(true).await;
    }

    async fn close(self, fenced: bool) {
        self.stop.send(()).expect("stop");
        self.task.await.expect("server shutdown");
        self.shared.identity.wait_for_imports().expect("drain");
        if let Some(binding) = self.shared.store.native_note_trace() {
            binding.service.shutdown().await.expect("Native shutdown");
            let database = self.shared.store.shared_agent_connection();
            let closed = tokio::task::spawn_blocking(move || database.shutdown())
                .await
                .expect("Core shutdown worker");
            if fenced {
                assert!(
                    closed
                        .unwrap_err()
                        .to_string()
                        .contains("data_recovery_required")
                );
            } else {
                closed.expect("Core database shutdown");
            }
        }
    }
}
