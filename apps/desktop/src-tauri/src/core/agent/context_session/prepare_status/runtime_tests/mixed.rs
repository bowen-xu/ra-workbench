//! Same Core Note writer, read proof journal and original source/file tools.
use super::*;
use crate::agent::composed_tools::RejectingToolExecutor;
use research_agent_core::{model::ToolCall, tool_registry::OwnedToolRegistration};
use rusqlite::Connection;
#[path = "mixed/pdf.rs"]
mod pdf;
#[path = "mixed/structured.rs"]
mod structured;

fn note(db: &mut Connection) -> Uuid {
    let tx = db.transaction().unwrap();
    let page = crate::notes::create_page_with_paper_in(
        &tx,
        crate::notes::NotePageDraft {
            title: "Core source note".into(),
            parent_page_id: None,
            parent_block_position: None,
        },
        None,
        None,
    )
    .unwrap();
    crate::notes::save_document_in(
        &tx,
        crate::notes::NoteDocumentSaveRequest {
            document: crate::notes::NoteDocument {
                schema_version: 1,
                page_id: page.clone(),
                title: None,
                blocks: vec![crate::notes::NoteDocumentBlock {
                    id: Uuid::new_v4().to_string(),
                    block_type: "paragraph".into(),
                    props: json!({}),
                    content: json!({"kind":"inline","items":[{"type":"text","text":"Frozen Core note","styles":[]}]}),
                    children: vec![],
                }],
            },
            expected_revision: 0,
            save_reason: "manual".into(),
        },
    ).unwrap();
    tx.commit().unwrap();
    Uuid::parse_str(&page).unwrap()
}
fn call(name: &str, id: &str, arguments: Value) -> ToolCall {
    ToolCall {
        namespace: None,
        raw_arguments_json: None,
        call_id: id.into(),
        name: name.into(),
        arguments,
    }
}

#[tokio::test]
#[ignore = "requires actual independent writing package; mixed Core Note and manuscript tools"]
async fn actual_package_mixed_reads_keep_core_note_proofs_and_current_authorization() {
    let mut f = writing_test_fixture(false);
    let native = binding();
    let conversation = writing_test_request(&f, Uuid::new_v4()).conversation_id;
    let page = note(&mut f.store.connection().unwrap());
    f.store = f
        .store
        .without_static_writing_context_sources()
        .with_native_writing_context(native.clone());
    let mut input = request(conversation, f.manuscript);
    input
        .resources
        .insert(0, Resource::Note { resource_id: page });
    let prepared = prepare(&f.store, &f.models, input.clone(), None)
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
    assert_eq!(record.turn_id, result(&prepared).turn_id);
    let (_, tools) = crate::agent::context_session::resource_tools::registration(
        f.store.clone(),
        record.clone(),
        OwnedToolRegistration::new(vec![], Arc::new(RejectingToolExecutor)),
    )
    .into_parts();
    let proof_count = || -> u64 {
        f.store.connection().unwrap().query_row(
            "select count(*) from agent_text_read_scopes where conversation_id=?1 and scope_request_id=?2",
            rusqlite::params![conversation.to_string(),input.request_id.to_string()], |row| row.get(0),
        ).unwrap()
    };
    let search = call(
        "research_search_sources",
        "mixed-search",
        json!({"query":"Frozen"}),
    );
    let output = tools.authorize_and_execute(search.clone()).await.unwrap();
    assert!(!output.is_error, "{}", output.content);
    let value: Value = serde_json::from_str(&output.content).unwrap();
    let matched = value["matches"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|item| item["sourceId"].as_str())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(matched.len(), 2, "{}", output.content);
    // A Note read occurred before each Native read-prefix pause. Its original
    // autocommit proof survives, and repeated identical reads do not insert it again.
    assert_eq!(proof_count(), 1);
    let note_source = record
        .input
        .context
        .iter()
        .find(|source| source.resource_id == page.to_string())
        .unwrap();
    let file = {
        let db = f.store.connection().unwrap();
        crate::agent::context_session::resource_grants::grants(
            &db,
            &[Resource::ManuscriptFile {
                resource_id: f.manuscript,
                path: "main.tex".into(),
            }],
            None,
        )
        .unwrap()
        .remove(0)
    };
    for source in [&note_source.source_id, &file.source_id] {
        let read = tools
            .authorize_and_execute(call(
                "research_read_source",
                source,
                json!({"sourceId":source}),
            ))
            .await
            .unwrap();
        assert!(!read.is_error, "{}", read.content);
        assert!(read.content.contains("Frozen"));
    }
    assert_eq!(proof_count(), 1);
    let note_file = tools
        .authorize_and_execute(call(
            "research_read_file",
            "core-note-file",
            json!({"path":format!("source://notes/{page}")}),
        ))
        .await
        .unwrap();
    assert!(!note_file.is_error, "{}", note_file.content);
    assert!(note_file.content.contains("Frozen Core note"));
    // Both tools canonicalize this Note to the same source://notes identity
    // and version, so the original journal reuses the existing read proof.
    assert_eq!(proof_count(), 1);
    let repeated = tools.authorize_and_execute(search.clone()).await.unwrap();
    assert!(!repeated.is_error, "{}", repeated.content);
    assert_eq!(proof_count(), 1);
    f.store
        .connection()
        .unwrap()
        .execute(
            "update note_pages set status='trashed' where id=?1",
            [page.to_string()],
        )
        .unwrap();
    let denied = tools.authorize_and_execute(search).await.unwrap();
    assert!(denied.is_error);
    assert!(!denied.content.contains("Frozen Core note"));
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
