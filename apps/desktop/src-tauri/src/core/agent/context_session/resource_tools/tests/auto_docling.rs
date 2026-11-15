use super::*;
use crate::docling::preparation::test_support::{Parser, attachment};
use std::sync::atomic::Ordering;
#[path = "auto_docling/cancellation.rs"]
mod cancellation;

fn pdf_tools() -> (tempfile::TempDir, ResourceTools, Arc<Parser>) {
    let (root, store, id) = fixture();
    attachment(&store, "paper", "file", &"a".repeat(64));
    let parser = Parser::new();
    parser.install(&store);
    let tools = tools(
        &store,
        id,
        vec![Resource::Paper {
            resource_id: "paper".into(),
            attachment_id: Some("file".into()),
            page_index: None,
        }],
    );
    (root, tools, parser)
}

#[tokio::test]
async fn structured_reads_with_a_bound_source_still_prepare_the_document() {
    let (_root, mut tools, parser) = pdf_tools();
    {
        let db = tools.store.connection().unwrap();
        db.execute("insert into paper_context_snapshots(id,paper_id,context_hash,context_schema_version,created_at) values('bound','paper',?1,1,'now')",["b".repeat(64)]).unwrap();
    }
    for source in &mut tools.record.input.context {
        let mut anchor: Value = serde_json::from_str(source.anchor.as_deref().unwrap()).unwrap();
        anchor["sourceSnapshotId"] = json!("bound");
        source.anchor = Some(anchor.to_string());
    }
    parser.release.add_permits(1);
    tools
        .prepare_content(&call(
            "research_read_source",
            json!({"fileId":"file","view":"structured"}),
        ))
        .await
        .unwrap();
    assert_eq!(parser.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn discovery_is_lazy_and_read_and_search_prepare_only_once() {
    let (_root, tools, parser) = pdf_tools();
    for name in ["research_list_sources", "research_find_sources"] {
        let result = tools
            .authorize_and_execute(call(name, json!({"resourceType":"paper"})))
            .await
            .unwrap();
        assert!(!result.is_error);
    }
    assert_eq!(parser.calls.load(Ordering::SeqCst), 0);
    parser.release.add_permits(1);
    let result = tools
        .authorize_and_execute(call("research_read_source", json!({"fileId":"file"})))
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.content);
    assert!(result.content.contains("structured evidence"));
    let result = tools
        .authorize_and_execute(call(
            "research_search_sources",
            json!({"fileId":"file","query":"structured evidence"}),
        ))
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.content);
    assert_eq!(
        serde_json::from_str::<Value>(&result.content).unwrap()["total"],
        1
    );
    assert_eq!(parser.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn content_search_can_prepare_an_unparsed_pdf_without_a_prior_read() {
    let (_root, tools, parser) = pdf_tools();
    parser.release.add_permits(1);
    let result = tools
        .authorize_and_execute(call(
            "research_search_sources",
            json!({"resourceType":"paper","query":"structured evidence"}),
        ))
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.content);
    assert_eq!(
        serde_json::from_str::<Value>(&result.content).unwrap()["total"],
        1
    );
    assert_eq!(parser.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn ungranted_files_are_rejected_before_starting_a_conversion() {
    let (_root, tools, parser) = pdf_tools();
    attachment(&tools.store, "paper", "excluded", &"a".repeat(64));
    let result = tools
        .authorize_and_execute(call("research_read_source", json!({"fileId":"excluded"})))
        .await
        .unwrap();
    assert!(result.is_error);
    assert_eq!(parser.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn parsing_failure_is_actionable_and_does_not_expose_raw_service_details() {
    let (_root, tools, parser) = pdf_tools();
    parser.fail.store(true, Ordering::SeqCst);
    parser.release.add_permits(1);
    let result = tools
        .authorize_and_execute(call("research_read_source", json!({"fileId":"file"})))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(
        result
            .content
            .contains("Automatic Docling preparation failed")
    );
    assert!(
        !result
            .content
            .contains("synthetic private transport detail")
    );
    parser.fail.store(false, Ordering::SeqCst);
    parser.release.add_permits(1);
    let result = tools
        .authorize_and_execute(call("research_read_source", json!({"fileId":"file"})))
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.content);
}

#[tokio::test]
async fn conversation_closed_while_parsing_cannot_read_the_result() {
    let (_root, tools, parser) = pdf_tools();
    let close = async {
        parser.wait_started().await;
        tools
            .store
            .connection()
            .unwrap()
            .execute(
                "update agent_conversations set archived=1 where id=?1",
                [tools.record.conversation_id.to_string()],
            )
            .unwrap();
        parser.release.add_permits(1);
    };
    let (result, ()) = tokio::join!(
        tools.authorize_and_execute(call("research_read_source", json!({"fileId":"file"}))),
        close
    );
    let result = result.unwrap();
    assert!(result.is_error);
    assert!(
        result
            .content
            .contains("independent conversation unavailable")
    );
    assert!(!result.content.contains("structured evidence"));
}
