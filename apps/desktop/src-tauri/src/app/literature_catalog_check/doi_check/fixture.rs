use super::*;
use crate::data_interface::SharedDatabase;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::oneshot,
    task::JoinHandle,
};

pub(super) const NOTE: &str = "doi-cli-independent-note";
pub(super) const DOI: &str = "10.1000/doi-fixture";
pub(super) const RAW: &str = "https://doi.org/10.1000/DOI-FIXTURE.";
pub(super) const BODY: &str = "Independent synthetic Core Note before DOI HTTP";
pub(super) const SAVED_BODY: &str = "Independent synthetic Core Note saved while DOI HTTP waits";

pub(super) fn seed(store: &AppStore) -> Result<Value, Error> {
    let mut db = store.connection()?;
    let tx = db.transaction()?;
    crate::notes::create_page_with_paper_in(
        &tx,
        crate::notes::NotePageDraft {
            title: "Synthetic DOI Core Note".into(),
            parent_page_id: None,
            parent_block_position: None,
        },
        None,
        Some(NOTE),
    )?;
    write_note(&tx, BODY)?;
    tx.commit()?;
    drop(db);
    note(store)
}

fn write_note(tx: &rusqlite::Transaction<'_>, body: &str) -> Result<(), Error> {
    let revision: i64 = tx.query_row(
        "select revision from note_pages where id=?1",
        [NOTE],
        |row| row.get(0),
    )?;
    let mut document = crate::notes::load_document_value(tx, NOTE)?;
    document.blocks[0].content =
        json!({"kind":"inline","items":[{"type":"text","text":body,"styles":[]}]});
    crate::notes::save_document_in(
        tx,
        crate::notes::NoteDocumentSaveRequest {
            document,
            expected_revision: revision,
            save_reason: "manual".into(),
        },
    )?;
    Ok(())
}

pub(super) fn save_during_http(database: &SharedDatabase) -> Result<Value, Error> {
    let mut db = database.lock()?;
    let tx = db.transaction()?;
    write_note(&tx, SAVED_BODY)?;
    tx.commit()?;
    Ok(serde_json::to_value(crate::notes::load_document_value(
        &db, NOTE,
    )?)?)
}

pub(super) fn note(store: &AppStore) -> Result<Value, Error> {
    let db = store.connection()?;
    Ok(serde_json::to_value(crate::notes::load_document_value(
        &db, NOTE,
    )?)?)
}

pub(super) fn response(title: &str, volume: &str) -> String {
    json!({"type":"journal-article","title":title,"volume":volume,
        "issued":{"date-parts":[[2026]]},"author":[{"family":"Synthetic","given":"Researcher"}],
        "keyword":"alpha;beta"})
    .to_string()
}

pub(super) async fn preview(
    service: &Arc<LiteratureTraceService>,
    database: SharedDatabase,
    key: String,
    doi: String,
    item_id: Option<String>,
    resolver: reqwest::Url,
) -> crate::runtime_plugins::native::NativeResult<MetadataPreviewV1> {
    #[cfg(debug_assertions)]
    {
        service
            .preview_doi_fixture(database, key, doi, item_id, resolver)
            .await
    }
    #[cfg(not(debug_assertions))]
    {
        let _ = (service, database, key, doi, item_id, resolver);
        Err(
            crate::runtime_plugins::native::NativeHostError::for_profile(
                crate::runtime_plugins::native::NativeProfile::NoteTraceDecision,
                "plugin_host_profile_unsupported",
                "DOI CLI loopback fixture requires a debug build.",
            ),
        )
    }
}

pub(super) struct Http {
    pub(super) url: reqwest::Url,
    pub(super) seen: oneshot::Receiver<()>,
    pub(super) release: oneshot::Sender<()>,
    pub(super) task: JoinHandle<Result<String, String>>,
}

/// One bounded, synthetic response in this Tokio process, never an external server.
pub(super) async fn http(status: &str, body: String) -> Result<Http, Error> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let url = reqwest::Url::parse(&format!("http://{}/doi-fixture", listener.local_addr()?))?;
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let (seen_tx, seen) = oneshot::channel();
    let (release, release_rx) = oneshot::channel();
    let task = tokio::spawn(async move {
        tokio::time::timeout(std::time::Duration::from_secs(60), async move {
            let (mut socket, _) = listener.accept().await.map_err(|e| e.to_string())?;
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                if request.len() >= 8192 {
                    return Err("synthetic HTTP header exceeded bound".into());
                }
                request.push(socket.read_u8().await.map_err(|e| e.to_string())?);
            }
            let _ = seen_tx.send(());
            release_rx
                .await
                .map_err(|_| "synthetic response not released".to_owned())?;
            socket
                .write_all(response.as_bytes())
                .await
                .map_err(|e| e.to_string())?;
            socket.shutdown().await.map_err(|e| e.to_string())?;
            String::from_utf8(request).map_err(|e| e.to_string())
        })
        .await
        .map_err(|_| "synthetic HTTP fixture timed out".to_owned())?
    });
    Ok(Http {
        url,
        seen,
        release,
        task,
    })
}

pub(super) fn check_request(request: &str) -> Result<(), Error> {
    let headers = request.to_ascii_lowercase();
    if !request.starts_with("GET /doi-fixture HTTP/1.1\r\n")
        || !headers.contains("accept: application/vnd.datacite.datacite+json, application/vnd.crossref.unixref+xml, application/vnd.citationstyles.csl+json\r\n")
        || !headers.contains("user-agent: researchassistant/0.1 (local desktop research tool)\r\n")
    {
        return Err("DOI fixture did not receive the original resolver GET/Accept/UA".into());
    }
    Ok(())
}
