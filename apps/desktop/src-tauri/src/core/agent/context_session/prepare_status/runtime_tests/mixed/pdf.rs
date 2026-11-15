//! The existing Docling owner joins the actual independent manuscript reader.
use super::*;
use crate::docling::preparation::test_support::{Parser, attachment};
use std::sync::atomic::Ordering;

#[tokio::test]
#[ignore = "requires actual independent writing package; mixed raw PDF and manuscript reads"]
async fn actual_package_mixed_pdf_reads_reuse_parse_and_recheck_current_attachment() {
    let mut f = writing_test_fixture(false);
    let native = binding();
    let conversation = writing_test_request(&f, Uuid::new_v4()).conversation_id;
    let page = note(&mut f.store.connection().unwrap());
    f.store.connection().unwrap().execute(
        "insert into bibliographic_items(id,item_type,title,created_at,updated_at) values('mixed-paper','document','Mixed PDF source','now','now')", [],
    ).unwrap();
    attachment(&f.store, "mixed-paper", "mixed-pdf", &"a".repeat(64));
    attachment(&f.store, "mixed-paper", "unselected-pdf", &"a".repeat(64));
    let parser = Parser::new();
    parser.install(&f.store);
    f.store = f
        .store
        .without_static_writing_context_sources()
        .with_native_writing_context(native.clone());
    let mut input = request(conversation, f.manuscript);
    input
        .resources
        .insert(0, Resource::Note { resource_id: page });
    input.resources.insert(
        1,
        Resource::Paper {
            resource_id: "mixed-paper".into(),
            attachment_id: Some("mixed-pdf".into()),
            page_index: None,
        },
    );
    prepare(&f.store, &f.models, input.clone(), None)
        .await
        .unwrap();
    let record = crate::agent::context_session::restore(
        &f.store,
        crate::agent::context_session::Receipt {
            conversation_id: conversation,
            request_id: input.request_id,
        },
    )
    .await
    .unwrap();
    let (_, tools) = crate::agent::context_session::resource_tools::registration(
        f.store.clone(),
        record.clone(),
        OwnedToolRegistration::new(vec![], Arc::new(RejectingToolExecutor)),
    )
    .into_parts();
    let proof_count = || -> u64 {
        f.store.connection().unwrap().query_row(
            "select count(*) from agent_text_read_scopes where conversation_id=?1 and scope_request_id=?2",
            rusqlite::params![conversation.to_string(), input.request_id.to_string()], |row| row.get(0),
        ).unwrap()
    };
    let denied = tools
        .authorize_and_execute(call(
            "research_read_source",
            "unselected-pdf",
            json!({"fileId":"unselected-pdf"}),
        ))
        .await
        .unwrap();
    assert!(denied.is_error);
    assert_eq!(parser.calls.load(Ordering::SeqCst), 0);
    let search = call(
        "research_search_sources",
        "mixed-pdf-search",
        json!({"query":"e"}),
    );
    parser.release.add_permits(1);
    let output = tools.authorize_and_execute(search.clone()).await.unwrap();
    assert!(!output.is_error, "{}", output.content);
    let value: Value = serde_json::from_str(&output.content).unwrap();
    let matched = value["matches"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["sourceId"].as_str().unwrap())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(matched.len(), 3, "{}", output.content);
    assert_eq!(parser.calls.load(Ordering::SeqCst), 1);
    assert_eq!(proof_count(), 1);
    for name in ["research_read_source", "research_search_sources"] {
        let arguments = if name == "research_read_source" {
            json!({"fileId":"mixed-pdf"})
        } else {
            json!({"fileId":"mixed-pdf","query":"structured evidence"})
        };
        let output = tools
            .authorize_and_execute(call(name, name, arguments))
            .await
            .unwrap();
        assert!(!output.is_error, "{}", output.content);
        assert!(output.content.contains("structured evidence"));
    }
    let repeated = tools.authorize_and_execute(search.clone()).await.unwrap();
    assert!(!repeated.is_error, "{}", repeated.content);
    assert_eq!(parser.calls.load(Ordering::SeqCst), 1);
    assert_eq!(proof_count(), 1);
    {
        let db = f.store.connection().unwrap();
        let hash = "b".repeat(64);
        db.execute("insert into content_objects(sha256,relative_path,mime_type,byte_size,created_at) values(?1,?1,'application/pdf',1,'now')", [&hash]).unwrap();
        db.execute(
            "update attachments set content_sha256=?1 where id='mixed-pdf'",
            [&hash],
        )
        .unwrap();
    }
    parser.fail.store(true, Ordering::SeqCst);
    parser.release.add_permits(1);
    let failed = tools.authorize_and_execute(search.clone()).await.unwrap();
    assert!(failed.is_error);
    assert!(
        failed
            .content
            .contains("Automatic Docling preparation failed")
    );
    assert!(!failed.content.contains("structured evidence"));
    assert!(
        !failed
            .content
            .contains("synthetic private transport detail")
    );
    assert_eq!(parser.calls.load(Ordering::SeqCst), 2);
    assert_eq!(proof_count(), 1);
    parser.fail.store(false, Ordering::SeqCst);
    parser.release.add_permits(1);
    let recovered = tools.authorize_and_execute(search).await.unwrap();
    assert!(!recovered.is_error, "{}", recovered.content);
    assert_eq!(parser.calls.load(Ordering::SeqCst), 3);
    f.store
        .connection()
        .unwrap()
        .execute(
            "update bibliographic_items set trashed_at='now' where id='mixed-paper'",
            [],
        )
        .unwrap();
    let revoked = tools
        .authorize_and_execute(call(
            "research_read_source",
            "revoked-pdf",
            json!({"fileId":"mixed-pdf"}),
        ))
        .await
        .unwrap();
    assert!(revoked.is_error);
    assert_eq!(parser.calls.load(Ordering::SeqCst), 3);
    assert_eq!(proof_count(), 1);
    assert!(f.store.connection().unwrap().is_autocommit());
    assert_eq!(
        std::fs::read_to_string(
            f.store
                .manuscript_directory()
                .join(f.manuscript.to_string())
                .join("source/main.tex")
        )
        .unwrap(),
        "Frozen manuscript"
    );
    drop(tools);
    native.service.shutdown().await.unwrap();
    f.store.shared_agent_connection().shutdown().unwrap();
}
