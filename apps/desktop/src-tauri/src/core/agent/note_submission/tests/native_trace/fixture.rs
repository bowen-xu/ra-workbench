use super::super::*;
use crate::runtime_plugins::{
    RuntimePluginCatalog,
    literature_trace::{binding::NoteTraceBinding, service::LiteratureTraceService},
};
use std::{fs, path::Path, sync::Arc, time::Duration};

pub(super) const TRACE: &str = "native-note-trace";
pub(super) const QUERY: &str = "Synthetic native retrieval query";
pub(super) const BODY: &str = "[CN-S1] Unchanged native retrieval evidence\0original bytes";
pub(super) const WAIT: Duration = Duration::from_secs(8);

pub(super) struct HostFixture {
    pub directory: tempfile::TempDir,
    pub service: Arc<LiteratureTraceService>,
    pub key: String,
}

pub(super) fn attach(f: &mut Fixture, executable: &[u8]) -> HostFixture {
    let directory = tempfile::tempdir().unwrap();
    let package = directory.path().join("literature");
    fs::create_dir_all(package.join("data")).unwrap();
    fs::write(package.join("host"), executable).unwrap();
    for name in ["read", "expire", "mark-sent"] {
        let name = format!("trace-{name}.v1.sql");
        fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../../tests/fixtures/native-resources-v1/literature")
                .join(&name),
            package.join("data").join(name),
        )
        .unwrap();
    }
    fs::write(package.join("manifest.json"), serde_json::to_vec(&serde_json::json!({
        "formatVersion":2,"id":"literature_library","version":"0.1.0","apiVersion":1,
        "dependencies":{"required":[],"optional":[]},"consumes":{"required":[],"optional":[]},
        "provides":[{"id":"literature.note-trace-decision","version":1}],
        "native":{"protocolVersion":2,"os":std::env::consts::OS,"arch":std::env::consts::ARCH,"entry":"host"}
    })).unwrap()).unwrap();
    let catalog = Arc::new(RuntimePluginCatalog::new(Some(directory.path().into())));
    let key = catalog.discover().unwrap().packages[0].package_key.clone();
    let service = Arc::new(LiteratureTraceService::new(catalog));
    f.store = f.store.clone().with_native_note_trace(NoteTraceBinding {
        service: Arc::clone(&service),
        package_key: key.clone(),
    });
    HostFixture {
        directory,
        service,
        key,
    }
}

pub(super) fn real_host() -> Vec<u8> {
    let path = std::env::var_os("RESEARCH_ASSISTANT_TEST_LITERATURE_HOST").expect(
        "set RESEARCH_ASSISTANT_TEST_LITERATURE_HOST to the independently built literature host",
    );
    fs::read(path).unwrap()
}

pub(super) fn trace(f: &Fixture) {
    f.store
        .connection()
        .unwrap()
        .execute(
            "insert into concept_retrieval_traces
         (id,paper_id,query_sha256,status,config_json,context_text,context_sha256,
          seed_count,edge_count,source_count,external_paper_count,created_at,expires_at)
         values(?1,null,?2,'previewed','{}',?3,?4,0,0,0,0,'now','2999-01-01T00:00:00.000Z')",
            params![
                TRACE,
                format!("{:x}", Sha256::digest(QUERY.as_bytes())),
                BODY,
                format!("{:x}", Sha256::digest(BODY.as_bytes()))
            ],
        )
        .unwrap();
}

pub(super) fn input(f: &Fixture, id: Uuid) -> PrepareRequest {
    let mut input = request(f, id);
    input.retrieval = Some(crate::agent::retrieval_context::RetrievalSelection {
        trace_id: TRACE.into(),
        query: QUERY.into(),
    });
    input
}

pub(super) fn status(f: &Fixture) -> String {
    f.store
        .connection()
        .unwrap()
        .query_row(
            "select status from concept_retrieval_traces where id=?1",
            [TRACE],
            |row| row.get(0),
        )
        .unwrap()
}

const HANDSHAKE: &str = r#"IFS= read -r line
identity=${line%\"body\":*}
printf '%s"body":{"kind":"initialized","capability":"literature.note-trace-decision","version":1}}\n' "$identity"
IFS= read -r line
identity=${line%\"body\":*}
"#;
const READY: &str = r#"printf '%s"body":{"kind":"noteTraceDecision","decision":{"status":"ready","markSent":{"expectedRows":1,"onMismatch":"already_consumed"}}}}\n' "$identity"
IFS= read -r line
identity=${line%\"body\":*}
printf '%s"body":{"kind":"shutdownComplete"}}\n' "$identity"
"#;

pub(super) fn paused_host(marker: &Path, gate: Option<&Path>) -> Vec<u8> {
    // The marker/gate are synthetic temp paths, never command input from a user.
    let wait = gate.map_or_else(
        || "IFS= read -r cancelled\n".into(),
        |gate| {
            format!(
                "while [ ! -f '{}' ]; do sleep 0.01; done\n{READY}",
                gate.display()
            )
        },
    );
    format!("#!/bin/sh\n{HANDSHAKE}touch '{}'\n{wait}", marker.display()).into_bytes()
}

pub(super) async fn wait_for_marker(marker: &Path) {
    tokio::time::timeout(WAIT, async {
        while !marker.exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}
