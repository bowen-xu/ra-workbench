use super::*;

#[tokio::test]
async fn read_receipts_report_lines_for_the_actual_unicode_character_window() {
    let f = fixture(true);
    let path = path(f.page);
    read(&f, &path).await;
    f.tools
        .apply("unicode-lines", &patch(&path, "old", "一🧪\n二\n三"))
        .await
        .unwrap();
    let full = f
        .tools
        .read(Read {
            path: path.clone(),
            start_char: 0,
            max_chars: 8000,
            view: None,
        })
        .await
        .unwrap();
    let text = full["text"].as_str().unwrap();
    assert_eq!(full["startChar"], 0);
    assert!(full["nextChar"].is_null());
    assert_eq!(full["startLine"], 1);
    assert_eq!(full["endLine"], text.lines().count());
    let chars = text.chars().collect::<Vec<_>>();
    let unicode = chars.iter().position(|c| *c == '一').unwrap();
    for (start, length) in [
        (unicode, 3),
        (unicode + 1, 4),
        (unicode + 3, 1),
        (chars.len(), 1),
    ] {
        let result = f
            .tools
            .read(Read {
                path: path.clone(),
                start_char: start,
                max_chars: length,
                view: None,
            })
            .await
            .unwrap();
        let returned = result["text"].as_str().unwrap();
        assert_eq!(
            returned,
            chars.iter().skip(start).take(length).collect::<String>()
        );
        if returned.is_empty() {
            assert!(result.get("startLine").is_none());
            assert!(result.get("endLine").is_none());
        } else {
            let first = chars[..start].iter().filter(|c| **c == '\n').count() + 1;
            assert_eq!(result["startLine"], first);
            assert_eq!(result["endLine"], first + returned.lines().count() - 1);
        }
    }
}

#[tokio::test]
async fn note_read_saved_and_candidate_preserve_unicode_style_and_whitespace() {
    let f = fixture(true);
    let original = {
        let mut db = f.tools.store.connection().unwrap();
        let tx = db.transaction().unwrap();
        let mut document = crate::notes::load_document_value(&tx, &f.page.to_string()).unwrap();
        document.blocks[0].content = json!({"kind":"inline","items":[
            {"type":"text","text":" ","styles":[]},
            {"type":"text","text":"核心问题：","styles":["bold"]},
            {"type":"text","text":"待讨论 ","styles":[]}
        ]});
        let revision = tx
            .query_row(
                "select revision from note_pages where id=?1",
                [f.page.to_string()],
                |r| r.get(0),
            )
            .unwrap();
        crate::notes::save_document_in(
            &tx,
            crate::notes::NoteDocumentSaveRequest {
                document: document.clone(),
                expected_revision: revision,
                save_reason: "manual".into(),
            },
        )
        .unwrap();
        tx.commit().unwrap();
        document
    };
    let p = path(f.page);
    let mut original_line = String::new();
    for view in ["saved", "candidate"] {
        let result = f
            .tools
            .read(Read {
                path: p.clone(),
                view: Some(view.into()),
                start_char: 0,
                max_chars: 8000,
            })
            .await
            .unwrap();
        let text = result["text"].as_str().unwrap();
        assert!(text.contains("<strong>核心问题：</strong>"));
        assert!(text.contains("&#32;"));
        original_line = text
            .lines()
            .find(|line| line.contains("待讨论"))
            .unwrap()
            .to_owned();
        // This codec returns frontmatter title separately; the actual tool
        // applies it through the existing document/scope owner below.
        let (title, document) = crate::notes::text_view::candidate(&original, text).unwrap();
        assert_eq!(Some(title.as_str()), original.title.as_deref());
        assert_eq!(document.page_id, original.page_id);
        assert_eq!(
            serde_json::to_value(document.blocks).unwrap(),
            serde_json::to_value(&original.blocks).unwrap()
        );
    }
    f.tools
        .apply(
            "edit-boundary",
            &patch(
                &p,
                &original_line,
                &original_line.replace("待讨论", "已讨论"),
            ),
        )
        .await
        .unwrap();
    let db = f.tools.store.connection().unwrap();
    let after = crate::notes::load_document_value(&db, &f.page.to_string()).unwrap();
    assert_eq!(after.blocks[0].id, original.blocks[0].id);
    assert_eq!(after.title, original.title);
    assert_eq!(
        after.blocks[0].content["items"][1]["styles"],
        json!(["bold"])
    );
    assert_eq!(after.blocks[0].content["items"][2]["text"], "已讨论 ");
}
