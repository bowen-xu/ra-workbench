use super::*;
use fixture::{PDF, text};

fn index(
    f: &Fixture,
    attachment: &str,
    pages: Vec<PageText>,
) -> Result<AttachmentResult, AttachmentFailure> {
    f.run(
        AttachmentRequest::Index {
            item_id: "paper".into(),
            attachment_id: attachment.into(),
            pages,
        },
        None,
    )
}

fn page(page_index: i32, text: &str) -> PageText {
    PageText {
        page_index,
        text: text.into(),
    }
}

#[test]
fn page_index_collision_restores_original_pages_job_and_fts_on_same_database_cold_open() {
    let f = Fixture::new();
    let attachment = f.attach("paper", "source.pdf", PDF);
    index(&f, &attachment.id, vec![page(3, "Original indexed prefix")]).unwrap();
    let prefix = f.prefix();
    let failure = index(
        &f,
        &attachment.id,
        vec![
            page(-1, "First clamped input"),
            page(0, "Conflicting later input"),
        ],
    )
    .unwrap_err();
    assert_eq!(failure.progress.phase, "index-page-insert");
    assert_eq!(failure.progress.sql, AttachmentSql::RolledBack);
    assert_eq!(
        failure.data.as_ref().unwrap().resource_id,
        "literature.attachments.index-page-insert"
    );
    assert_eq!(
        failure.data.as_ref().unwrap().sqlite_code.as_deref(),
        Some("ConstraintViolation")
    );
    assert_eq!(f.prefix(), prefix);
    assert!(f.store.connection().unwrap().is_autocommit());
    assert_eq!(f.reopen().prefix(), prefix);
}

#[test]
fn original_input_order_negative_clamping_empty_completion_and_other_item_fts_are_preserved() {
    let f = Fixture::new();
    let attachment = f.attach("paper", "source.pdf", PDF);
    let AttachmentResult::Indexed(indexed) = f
        .run(
            AttachmentRequest::IsIndexed {
                attachment_id: attachment.id.clone(),
            },
            None,
        )
        .unwrap()
    else {
        panic!("index status");
    };
    assert!(!indexed);
    let original_order = vec![page(9, "First supplied"), page(-4, "Second supplied")];
    index(&f, &attachment.id, original_order).unwrap();
    assert_eq!(
        f.rows("select page_index,page_text from attachment_page_texts order by page_index"),
        vec![
            vec![0_i64.into(), text("Second supplied")],
            vec![9_i64.into(), text("First supplied")]
        ]
    );
    assert_eq!(
        f.rows("select state,page_count,error_message from full_text_index_jobs"),
        vec![vec![
            text("completed"),
            2_i64.into(),
            rusqlite::types::Value::Null
        ]]
    );
    // Compare exactly the original unordered group_concat projection, rather
    // than introducing an output order that the old SQL never promised.
    assert_eq!(f.rows("select full_text from literature_search where item_id='paper'"),
        f.rows("select group_concat(page_text,' ') from attachment_page_texts p join attachments a on a.id=p.attachment_id where a.item_id='paper'"));
    assert_eq!(
        f.rows("select full_text from literature_search where item_id='other'"),
        vec![vec![text("before")]]
    );
    let AttachmentResult::Indexed(indexed) = f
        .run(
            AttachmentRequest::IsIndexed {
                attachment_id: attachment.id.clone(),
            },
            None,
        )
        .unwrap()
    else {
        panic!("index status");
    };
    assert!(indexed);
    index(&f, &attachment.id, vec![]).unwrap();
    assert!(f.rows("select * from attachment_page_texts").is_empty());
    assert_eq!(
        f.rows("select state,page_count from full_text_index_jobs"),
        vec![vec![text("completed"), 0_i64.into()]]
    );
    assert_eq!(
        f.rows("select full_text from literature_search where item_id='paper'"),
        vec![vec![text("")]]
    );
}
