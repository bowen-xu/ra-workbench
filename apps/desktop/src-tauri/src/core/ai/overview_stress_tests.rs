use rusqlite::{Connection, params};

use crate::storage::migrate;

use super::task_evidence_support::TaskEvidenceFacts;
use research_literature_library_native::analysis_workflow::task_preview::pages;

fn preview_overview_evidence(
    connection: &Connection,
    attachment_id: &str,
) -> Vec<super::model::AiEvidencePreview> {
    let facts = TaskEvidenceFacts::new(connection);
    let attachment = facts.attachment(attachment_id).unwrap();
    pages::validate_attachment(attachment.as_ref()).unwrap();
    let sources = pages::select_pages(
        &attachment.unwrap(),
        attachment_id,
        &facts.page_texts(attachment_id).unwrap(),
    )
    .unwrap();
    pages::preview_sources(&sources)
}

const ATTACHMENT_HASH: &str = "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";

fn long_pdf_database() -> Connection {
    let mut connection = Connection::open_in_memory().expect("open isolated database");
    migrate(&mut connection).expect("apply migrations");
    connection
        .execute_batch(
            "insert into bibliographic_items
             (id, item_type, title, created_at, updated_at)
             values ('paper-long', 'journalArticle', 'Synthetic long paper', 'now', 'now');",
        )
        .expect("insert synthetic paper");
    connection
        .execute(
            "insert into content_objects
             (sha256, relative_path, mime_type, byte_size, created_at)
             values (?1, 'dd/synthetic-long.pdf', 'application/pdf', 10000, 'now')",
            [ATTACHMENT_HASH],
        )
        .expect("insert synthetic content");
    connection
        .execute(
            "insert into attachments
             (id, item_id, content_sha256, display_name, created_at)
             values ('attachment-long', 'paper-long', ?1, 'synthetic-long.pdf', 'now')",
            [ATTACHMENT_HASH],
        )
        .expect("insert synthetic attachment");
    connection
        .execute(
            "insert into full_text_index_jobs
             (attachment_id, state, page_count, updated_at)
             values ('attachment-long', 'completed', 30, 'index-v1')",
            [],
        )
        .expect("mark synthetic attachment indexed");
    for page_index in 0..30 {
        let page_text = format!(
            "Page {page_index}. Methods. Results. Conclusion. {}",
            "bounded synthetic evidence ".repeat(250),
        );
        connection
            .execute(
                "insert into attachment_page_texts
                 (attachment_id, page_index, page_text, indexed_at)
                 values ('attachment-long', ?1, ?2, 'index-v1')",
                params![page_index, page_text],
            )
            .expect("insert synthetic page text");
    }
    connection
}

#[test]
fn long_pdf_preview_is_bounded_and_identity_stable() {
    let connection = long_pdf_database();

    let first = preview_overview_evidence(&connection, "attachment-long");
    let second = preview_overview_evidence(&connection, "attachment-long");

    assert_eq!(first.len(), 10, "a long PDF must freeze at most ten pages");
    assert_eq!(
        first.iter().map(|item| item.page_index).collect::<Vec<_>>(),
        (0..10).collect::<Vec<_>>(),
        "equal-scoring pages must remain deterministic after the first-page preference",
    );
    assert!(first.iter().all(|item| {
        item.quote_exact.chars().count() <= 1_200
            && item.context_text.chars().count() <= 1_520
            && item.content_sha256 == ATTACHMENT_HASH
    }));
    assert_eq!(
        first
            .iter()
            .map(|item| (&item.citation_key, &item.anchor_id))
            .collect::<Vec<_>>(),
        second
            .iter()
            .map(|item| (&item.citation_key, &item.anchor_id))
            .collect::<Vec<_>>(),
        "repeating preview must preserve citation and source-anchor identities",
    );
}
