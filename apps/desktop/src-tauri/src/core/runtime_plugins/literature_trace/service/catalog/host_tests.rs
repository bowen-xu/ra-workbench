#![cfg(unix)]
use super::*;
use crate::{
    data_interface::Database,
    runtime_plugins::native::package::{stage_for, tests::literature_fixture},
};
use rusqlite::Connection;
use std::{fs, path::Path};
use tokio::time::timeout;

const WAIT: Duration = Duration::from_secs(8);
const WRONG_DTO_HOST: &str = r#"#!/bin/sh
IFS= read -r line
identity=${line%\"body\":*}
printf '%s"body":{"kind":"initialized","capability":"literature.note-trace-decision","version":1}}\n' "$identity"
IFS= read -r line
identity=${line%\"body\":*}
package=${line#*\"packageKey\":\"}; package=${package%%\"*}
instance=${line#*\"instanceId\":\"}; instance=${instance%%\"*}
request=${line#*\"requestId\":\"}; request=${request%%\"*}
result=catalog-result-${request}.json
printf '%s' '__OUTPUT__' > "$result"
bytes=$(/usr/bin/wc -c < "$result")
digest=$(/usr/bin/shasum -a 256 "$result"); digest=${digest%% *}
printf '%s"body":{"kind":"catalogResult","payload":{"relativeHandle":"%s","byteSize":%s,"sha256":"%s","packageKey":"%s","instanceId":"%s","requestId":"%s"}}}\n' "$identity" "$result" "$bytes" "$digest" "$package" "$instance" "$request"
IFS= read -r ignored
"#;

#[tokio::test]
async fn wrong_projection_id_in_a_valid_real_reply_retires_and_reaps_the_service_host() {
    let output = serde_json::to_string(&CatalogOutput::Projection {
        outcome: success(&["wrong-result-only-id"]),
    })
    .unwrap();
    assert!(!output.contains('\''), "fixture uses one shell literal");
    let script = WRONG_DTO_HOST.replace("__OUTPUT__", &output);
    let (package_root, catalog, _) = literature_fixture(script.as_bytes());
    let package_directory = package_root.path().join("writing");
    let manifest_path = package_directory.join("manifest.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    manifest["provides"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id":"literature.catalog-read","version":1}));
    fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../plugins/literature_library/native/data");
    for name in [
        "list-active-ids",
        "search-fts-ids",
        "search-year-ids",
        "item-main",
        "item-creators",
        "item-tags",
        "item-attachments",
        "item-project-ids",
        "item-metadata-sources",
    ] {
        let name = format!("catalog-{name}.v1.sql");
        fs::copy(
            source.join(&name),
            package_directory.join("data").join(name),
        )
        .unwrap();
    }
    let key = catalog.discover().unwrap().packages[0].package_key.clone();
    let data_root = tempfile::tempdir().unwrap();
    let mut sql = Connection::open(data_root.path().join("catalog.sqlite3")).unwrap();
    crate::storage::migrate(&mut sql).unwrap();
    let database = Database::open(sql, &data_root.path().join("data-log")).unwrap();
    database
        .lock()
        .unwrap()
        .execute(
            "insert into bibliographic_items(id,item_type,title,created_at,updated_at)
         values('requested-source-only-id','journalArticle','Synthetic source','now','now')",
            [],
        )
        .unwrap();
    let staged = stage_for(&catalog, &key, NativeProfile::NoteTraceDecision).unwrap();
    let directory = staged.input_directory.as_ref().unwrap().path().to_owned();
    let executable = staged.directory.path().to_owned();
    let mut host = NativeHost::spawn_for(staged, &key, NativeProfile::NoteTraceDecision).unwrap();
    timeout(WAIT, host.initialize()).await.unwrap().unwrap();
    let service = LiteratureTraceService::new(Arc::clone(&catalog));
    *service.host.lock().await = Some(host);

    let error = timeout(
        WAIT,
        service.catalog(
            Arc::clone(&database),
            key.clone(),
            CatalogRequest::Get {
                id: "requested-source-only-id".into(),
            },
        ),
    )
    .await
    .unwrap()
    .unwrap_err();
    assert_eq!(error.code, "plugin_host_response_mismatch");
    assert_eq!(error.plugin_id, "literature_library");
    assert_eq!(error.package_key.as_deref(), Some(key.as_str()));
    assert!(error.instance_id.is_some() && error.request_id.is_some());
    assert!(error.exit_code.is_some() || error.signal.is_some());
    assert!(
        error.cleanup_error.is_none(),
        "safe native cleanup diagnostic: {error:?}"
    );
    assert!(service.host.lock().await.is_none());
    assert!(!service.cleanup_failed.load(Ordering::Acquire));
    assert!(!directory.exists() && !executable.exists());
    let diagnostic = serde_json::to_string(&error).unwrap();
    for private in [
        "wrong-result-only-id",
        "requested-source-only-id",
        data_root.path().to_str().unwrap(),
    ] {
        assert!(!diagnostic.contains(private));
    }
    assert_eq!(
        database
            .lock()
            .unwrap()
            .query_row("select count(*) from bibliographic_items", [], |row| {
                row.get::<_, u64>(0)
            })
            .unwrap(),
        1
    );
    timeout(WAIT, service.shutdown()).await.unwrap().unwrap();
}
