//! Actual domain hosts share a dynamic manuscript scope and Core evidence storage.
use super::super::super::fixture::call;
use super::*;
use crate::{
    agent::context_session::resource_grants,
    docling::preparation::test_support::attachment,
    runtime_plugins::literature_trace::{
        binding::NoteTraceBinding, service::LiteratureTraceService,
    },
};

fn seed_pdf(fixture: &Fixture) {
    for (file, hash, text) in [
        ("selected", "a".repeat(64), "Authorized PDF evidence Ω"),
        ("unselected", "b".repeat(64), "Unselected attachment text"),
    ] {
        attachment(&fixture.store, "paper", file, &hash);
        let raw = json!({
            "pages":{
                "1":{"page_no":1,"size":{"width":200.0,"height":400.0}},
                "2":{"page_no":2,"size":{"width":200.0,"height":400.0}}
            },
            "texts":[
                {"self_ref":"#/texts/0","text":text,"prov":[{"page_no":1,"bbox":{"l":20.0,"t":40.0,"r":120.0,"b":80.0}}]},
                {"self_ref":"#/texts/1","text":"Unselected page text","prov":[{"page_no":2,"bbox":{"l":20.0,"t":40.0,"r":120.0,"b":80.0}}]}
            ]
        });
        fixture.store.connection().unwrap().execute(
            "insert into docling_parse_runs(id,item_id,attachment_id,content_sha256,service_base_url,docling_status,parse_status,json_content,raw_response,created_at,finished_at) values(?1,'paper',?2,?3,'fixture','success','success',?4,'{}','now','now')",
            rusqlite::params![format!("parse-{file}"), file, hash, raw.to_string()],
        ).unwrap();
    }
}

fn snapshot_and_proof(fixture: &Fixture) -> (String, String, String) {
    assert_eq!(fixture.proofs(), 1);
    let db = fixture.store.connection().unwrap();
    assert_eq!(
        db.query_row("select count(*) from paper_context_snapshots", [], |row| {
            row.get::<_, u64>(0)
        })
        .unwrap(),
        1
    );
    let snapshot = db
        .query_row("select id from paper_context_snapshots", [], |row| {
            row.get(0)
        })
        .unwrap();
    let (version, proof) = db.query_row(
        "select version,snapshot_json from agent_text_read_scopes where conversation_id=?1 and scope_request_id=?2",
        rusqlite::params![fixture.record.conversation_id.to_string(),fixture.record.request_id.to_string()],
        |row| Ok((row.get(0)?,row.get(1)?)),
    ).unwrap();
    (snapshot, version, proof)
}

#[tokio::test]
#[ignore = "requires actual independent literature and writing package collection"]
async fn actual_two_packages_freeze_only_authorized_pdf_page_with_dynamic_manuscript_and_reopen() {
    let (mut fixture, mut writing, manuscript) = mixed_fixture(false);
    seed_pdf(&fixture);
    {
        let mut db = fixture.store.connection().unwrap();
        let mut resources: Vec<_> = fixture
            .record
            .input
            .context
            .iter()
            .map(|source| resource_grants::resource(source).unwrap())
            .collect();
        resources.push(Resource::Paper {
            resource_id: "paper".into(),
            attachment_id: Some("selected".into()),
            page_index: Some(0),
        });
        let tx = db.transaction().unwrap();
        let mut input = fixture.record.input.clone();
        input.context = resource_grants::grants(&tx, &resources, None).unwrap();
        fixture.record =
            submissions::insert_in(&tx, fixture.record.conversation_id, Uuid::new_v4(), input)
                .unwrap();
        tx.commit().unwrap();
    }
    prepare_dynamic_scope(&mut fixture, &writing).await;
    assert!(fixture.store.paper_reading_source().is_none());
    assert!(fixture.store.writing_resource_source().is_none());
    let denied = fixture
        .execute(call(
            "research_read_source",
            json!({"fileId":"unselected","view":"structured"}),
        ))
        .await;
    assert!(denied.is_error);
    assert_eq!(fixture.proofs(), 0);
    assert_eq!(
        fixture
            .store
            .connection()
            .unwrap()
            .query_row("select count(*) from paper_context_snapshots", [], |row| {
                row.get::<_, u64>(0)
            })
            .unwrap(),
        0
    );
    let read = call(
        "research_read_source",
        json!({"fileId":"selected","view":"structured","maxChars":8000}),
    );
    let output = fixture.execute(read.clone()).await;
    assert!(!output.is_error, "{}", output.content);
    let value: Value = serde_json::from_str(&output.content).unwrap();
    let body: Value = serde_json::from_str(value["text"].as_str().unwrap()).unwrap();
    assert!(output.content.contains("Authorized PDF evidence Ω"));
    assert!(!output.content.contains("Unselected attachment text"));
    assert!(!output.content.contains("Unselected page text"));
    assert_eq!(body["items"].as_array().unwrap().len(), 1);
    assert_eq!(body["items"][0]["attachmentId"], "selected");
    assert_eq!(body["items"][0]["pageIndex"], 0);
    assert!(body["items"][0]["evidenceItemId"].is_string());
    let proof = snapshot_and_proof(&fixture);
    assert_eq!(body["snapshotId"], proof.0);
    assert_eq!(proof.0, proof.1);
    let repeated = fixture.execute(read.clone()).await;
    assert!(!repeated.is_error, "{}", repeated.content);
    assert_eq!(
        serde_json::from_str::<Value>(&repeated.content).unwrap(),
        value
    );
    assert_eq!(snapshot_and_proof(&fixture), proof);
    let file = resource_grants::grants(
        &fixture.store.connection().unwrap(),
        &[Resource::ManuscriptFile {
            resource_id: manuscript,
            path: "main.tex".into(),
        }],
        None,
    )
    .unwrap()
    .remove(0);
    let manuscript_read = fixture
        .execute(call(
            "research_read_source",
            json!({"sourceId":file.source_id}),
        ))
        .await;
    assert!(!manuscript_read.is_error, "{}", manuscript_read.content);
    assert!(manuscript_read.content.contains("Manuscript Ω"));
    assert_eq!(snapshot_and_proof(&fixture), proof);

    writing.service.shutdown().await.unwrap();
    fixture.service.shutdown().await.unwrap();
    fixture.store.shared_agent_connection().shutdown().unwrap();
    let catalog = writing.catalog.clone();
    let key = fixture.key.clone();
    writing.service = Arc::new(NativePreviewService::new(catalog.clone()));
    // Shutdown settles the shared worker; dropping its last Store also releases
    // the original Raft engine before opening the same persistent directory.
    drop(fixture.store);
    fixture.store = AppStore::open_with_paths(
        crate::storage::AppStorePaths {
            data_directory: fixture._data.path().into(),
            agent_configuration_directory: fixture._data.path().join("configuration"),
        },
        crate::storage::AppStorePorts::with_events(
            crate::runtime_events::RuntimeEvents::new(|_, _| Ok(())),
            crate::storage::AppStoreSources::default(),
        ),
    )
    .unwrap()
    .with_native_writing_context(writing.clone());
    fixture.service = Arc::new(LiteratureTraceService::new(catalog));
    let literature = NoteTraceBinding {
        service: fixture.service.clone(),
        package_key: key,
    };
    fixture.store = fixture
        .store
        .clone()
        .with_native_note_trace(literature.clone());
    let record = submissions::read(
        &fixture.store.connection().unwrap(),
        fixture.record.conversation_id,
        fixture.record.request_id,
    )
    .unwrap()
    .unwrap();
    assert_eq!(record, fixture.record);
    fixture.record = record;
    let reopened = fixture.execute(read.clone()).await;
    assert!(!reopened.is_error, "{}", reopened.content);
    assert_eq!(
        serde_json::from_str::<Value>(&reopened.content).unwrap(),
        value
    );
    assert_eq!(snapshot_and_proof(&fixture), proof);
    let mut missing = literature.clone();
    missing.package_key = "0".repeat(64);
    fixture.store = fixture.store.clone().with_native_note_trace(missing);
    let unavailable = fixture.execute(read.clone()).await;
    assert!(unavailable.is_error);
    let diagnostic: Value = serde_json::from_str(&unavailable.content).unwrap();
    assert_eq!(
        diagnostic["requestId"],
        fixture.record.request_id.to_string()
    );
    assert_eq!(diagnostic["turnId"], fixture.record.turn_id.to_string());
    assert_eq!(diagnostic["callId"], read.call_id);
    assert_eq!(diagnostic["nativeFailure"]["packageKey"], "0".repeat(64));
    assert!(!unavailable.content.contains("Authorized PDF evidence Ω"));
    assert_eq!(snapshot_and_proof(&fixture), proof);
    fixture.store = fixture.store.clone().with_native_note_trace(literature);
    let restored = fixture.execute(read.clone()).await;
    assert!(!restored.is_error, "{}", restored.content);
    assert_eq!(
        serde_json::from_str::<Value>(&restored.content).unwrap(),
        value
    );
    fixture
        .store
        .connection()
        .unwrap()
        .execute(
            "update bibliographic_items set trashed_at='now' where id='paper'",
            [],
        )
        .unwrap();
    let revoked = fixture.execute(read).await;
    assert!(revoked.is_error);
    assert!(!revoked.content.contains("Authorized PDF evidence Ω"));
    assert_eq!(snapshot_and_proof(&fixture), proof);
    assert!(fixture.store.connection().unwrap().is_autocommit());
    assert_eq!(
        fs::read_to_string(
            fixture
                .store
                .manuscript_directory()
                .join(manuscript.to_string())
                .join("source/main.tex")
        )
        .unwrap(),
        "Manuscript Ω"
    );
    writing.service.shutdown().await.unwrap();
    fixture.service.shutdown().await.unwrap();
    fixture.store.shared_agent_connection().shutdown().unwrap();
}
